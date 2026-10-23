use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ZetarisConfig {
    pub base_url: String,
    pub api_key: String,
    pub org_id: String,
    pub project_table: String,
    pub evidence_table: String,
}

impl ZetarisConfig {
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("ZETARIS_BASE_URL").ok().filter(|s| !s.is_empty())?;
        let api_key = std::env::var("ZETARIS_API_KEY").ok().filter(|s| !s.is_empty())?;
        let org_id = std::env::var("ZETARIS_ORG_ID").ok().filter(|s| !s.is_empty())?;
        let project_table = std::env::var("ZETARIS_PROJECT_TABLE").unwrap_or_else(|_| "project_lines".into());
        let evidence_table = std::env::var("ZETARIS_EVIDENCE_TABLE").unwrap_or_else(|_| "evidence_extracts".into());
        Some(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            org_id,
            project_table,
            evidence_table,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZetarisStatus {
    pub state: String,
    pub detail: String,
    pub request_id: Option<String>,
    pub live: bool,
}

pub fn join_sql(project_table: &str, evidence_table: &str, case_id: &str) -> Result<String, String> {
    let project = ident(project_table)?;
    let evidence = ident(evidence_table)?;
    let case_id = case_id_literal(case_id)?;
    Ok(format!(
        "SELECT p.line_id, p.amount_cents, e.document, e.excerpt \
         FROM {project} p \
         LEFT JOIN {evidence} e ON p.line_id = e.line_id \
         WHERE p.case_id = '{case_id}' \
         LIMIT 100"
    ))
}

pub fn run_body(sql: &str, limit: u32) -> Value {
    json!({
        "queryId": Uuid::new_v4().to_string(),
        "sql": sql,
        "source": "SqlEditor",
        "limit": limit
    })
}

fn ident(value: &str) -> Result<String, String> {
    if value.len() > 64 || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || value.starts_with(|c: char| c.is_ascii_digit()) {
        return Err("Zetaris table names must be simple identifiers.".into());
    }
    Ok(value.to_string())
}

fn case_id_literal(value: &str) -> Result<String, String> {
    if value.len() > 64 || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Case id is not safe to inline.".into());
    }
    Ok(value.to_string())
}

#[derive(Debug, Deserialize)]
struct DatasourceResponse {
    #[serde(default)]
    datasources: Vec<Value>,
}

pub async fn check(config: &ZetarisConfig, http: &reqwest::Client) -> ZetarisStatus {
    let request_id = Uuid::new_v4().to_string();
    let url = format!("{}/api/v1.0/datasource/datasources", config.base_url);
    let response = http
        .get(url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("X-Org-ID", &config.org_id)
        .header("X-Request-ID", &request_id)
        .send()
        .await;
    match response {
        Ok(res) if res.status().is_success() => {
            let count = res
                .json::<DatasourceResponse>()
                .await
                .map(|body| body.datasources.len())
                .unwrap_or(0);
            ZetarisStatus {
                state: "connected".into(),
                detail: format!("Live datasource list returned {count} entries."),
                request_id: Some(request_id),
                live: true,
            }
        }
        Ok(res) => ZetarisStatus {
            state: "error".into(),
            detail: format!("Zetaris returned HTTP {}.", res.status()),
            request_id: Some(request_id),
            live: false,
        },
        Err(err) => ZetarisStatus {
            state: "error".into(),
            detail: format!("Zetaris could not be reached: {err}"),
            request_id: Some(request_id),
            live: false,
        },
    }
}

pub async fn query_join(config: &ZetarisConfig, http: &reqwest::Client, case_id: &str) -> Result<ZetarisQuery, String> {
    let sql = join_sql(&config.project_table, &config.evidence_table, case_id)?;
    let request_id = Uuid::new_v4().to_string();
    let body = run_body(&sql, 100);
    let url = format!("{}/api/v1.0/sql-editor/sqls/run", config.base_url);
    let response = http
        .post(url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("X-Org-ID", &config.org_id)
        .header("X-Request-ID", &request_id)
        .json(&body)
        .send()
        .await
        .map_err(|err| format!("Zetaris query failed: {err}"))?;
    let status = response.status();
    let text = response.text().await.map_err(|err| err.to_string())?;
    if !status.is_success() {
        return Err(format!("Zetaris query returned HTTP {status}."));
    }
    Ok(ZetarisQuery {
        request_id,
        sql,
        body: text.chars().take(4000).collect(),
        live: true,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZetarisQuery {
    pub request_id: String,
    pub sql: String,
    pub body: String,
    pub live: bool,
}

pub fn not_configured() -> ZetarisStatus {
    ZetarisStatus {
        state: "not_configured".into(),
        detail: "AWS quickstart callback succeeded, but ZETARIS_BASE_URL, ZETARIS_API_KEY, and ZETARIS_ORG_ID are not set. No live query was sent.".into(),
        request_id: None,
        live: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_sql_is_a_fixed_template() {
        let sql = join_sql("project_lines", "evidence_extracts", "harbor-annex").unwrap();
        assert!(sql.contains("LEFT JOIN evidence_extracts"));
        assert!(sql.contains("harbor-annex"));
        assert!(sql.contains("\"source\":") == false);
    }

    #[test]
    fn rejects_injected_table_names() {
        assert!(join_sql("project_lines; drop", "evidence_extracts", "harbor-annex").is_err());
        assert!(join_sql("project_lines", "evidence_extracts", "harbor' OR 1=1").is_err());
    }

    #[test]
    fn run_body_matches_the_documented_rest_shape() {
        let body = run_body("select 1", 25);
        assert_eq!(body["source"], "SqlEditor");
        assert_eq!(body["limit"], 25);
        assert!(body["queryId"].as_str().unwrap().len() > 10);
    }
}
