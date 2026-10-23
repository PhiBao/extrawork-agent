use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

const SAAS_DEFAULT_BASE: &str = "https://api.saas.enterprise.zetaris.com";

/// Which Zetaris surface the configured URL points at.
///
/// - `Gateway`: the managed-cluster UI proxy on the Gateway origin
///   (`/api/proxy/...`), used by the getting-started coding clients.
/// - `Rest`: the documented REST API (`/api/v1.0/sql-editor/sqls/run`).
/// - `Saas`: the Cloud SaaS API (`/api/v1.0/query/sql/start`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStyle {
    Gateway,
    Rest,
    Saas,
}

impl ApiStyle {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("rest") | Some("self_managed") | Some("self-managed") => Self::Rest,
            Some("saas") => Self::Saas,
            _ => Self::Gateway,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Rest => "rest",
            Self::Saas => "saas",
        }
    }

    /// Best-effort style inference from a pasted workspace URL.
    pub fn detect(url: &str) -> Self {
        let value = url.trim_end_matches('/');
        if value.contains("api.saas.enterprise.zetaris.com") {
            Self::Saas
        } else if value.ends_with("/api/v1.0") || value.contains(":8888") {
            Self::Rest
        } else {
            Self::Gateway
        }
    }
}

#[derive(Debug, Clone)]
enum Auth {
    Key(String),
    User { username: String, password: String },
}

#[derive(Debug, Clone)]
pub struct ZetarisConfig {
    pub base_url: String,
    pub org_id: Option<String>,
    pub style: ApiStyle,
    pub project_table: String,
    pub evidence_table: String,
    auth: Auth,
}

impl ZetarisConfig {
    pub fn from_env() -> Option<Self> {
        let explicit_style = std::env::var("ZETARIS_API_STYLE").ok().filter(|s| !s.is_empty());
        let configured_base = std::env::var("ZETARIS_BASE_URL").ok().filter(|s| !s.is_empty());
        let style = match explicit_style.as_deref() {
            Some(value) => ApiStyle::parse(Some(value)),
            None => match configured_base.as_deref() {
                Some(value) => ApiStyle::detect(value),
                None => ApiStyle::Gateway,
            },
        };
        let org_id = std::env::var("ZETARIS_ORG_ID").ok().filter(|s| !s.is_empty());
        let auth = match std::env::var("ZETARIS_API_KEY").ok().filter(|s| !s.is_empty()) {
            Some(key) => Auth::Key(key),
            None => {
                let username = std::env::var("ZETARIS_USERNAME").ok().filter(|s| !s.is_empty())?;
                let password = std::env::var("ZETARIS_PASSWORD").ok().filter(|s| !s.is_empty())?;
                Auth::User { username, password }
            }
        };
        let base_url = configured_base.unwrap_or_else(|| match style {
            ApiStyle::Saas => SAAS_DEFAULT_BASE.to_string(),
            _ => String::new(),
        });
        if base_url.is_empty() {
            return None;
        }
        Some(Self {
            base_url: normalize_base(&base_url, style),
            org_id,
            style,
            project_table: std::env::var("ZETARIS_PROJECT_TABLE").unwrap_or_else(|_| "project_lines".into()),
            evidence_table: std::env::var("ZETARIS_EVIDENCE_TABLE").unwrap_or_else(|_| "evidence_extracts".into()),
            auth,
        })
    }

    fn org_header(&self) -> Result<&str, String> {
        self.org_id
            .as_deref()
            .ok_or_else(|| "ZETARIS_ORG_ID is required for this API style.".to_string())
    }
}

fn normalize_base(value: &str, style: ApiStyle) -> String {
    let mut trimmed = value.trim().trim_end_matches('/').to_string();
    match style {
        ApiStyle::Rest => {
            if !trimmed.ends_with("/api/v1.0") {
                trimmed.push_str("/api/v1.0");
            }
        }
        _ => {
            if let Some(stripped) = trimmed.strip_suffix("/api/v1.0") {
                trimmed = stripped.to_string();
            }
        }
    }
    trimmed
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZetarisStatus {
    pub state: String,
    pub detail: String,
    pub request_id: Option<String>,
    pub live: bool,
    pub style: String,
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

/// SaaS OpenSqlRequest body, per the published OpenAPI document.
pub fn saas_start_body(sql: &str, page_limit: u32) -> Value {
    json!({ "select": sql, "pageLimit": page_limit.clamp(1, 100) })
}

/// Documented REST API body (`POST /api/v1.0/sql-editor/sqls/run`).
pub fn rest_body(sql: &str, limit: u32) -> Value {
    json!({
        "queryId": Uuid::new_v4().to_string(),
        "sql": sql,
        "source": "SqlEditor",
        "limit": limit
    })
}

/// Gateway proxy body (`POST /api/proxy/sql-editor/sqls/run-query`).
pub fn gateway_body(sql: &str, limit: u32) -> Value {
    json!({ "sql": sql, "limit": limit })
}

fn ident(value: &str) -> Result<String, String> {
    // One optional namespace separator: `database.table`. Each part must be a
    // simple identifier so no caller-controlled SQL can enter the template.
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() > 2 {
        return Err("Zetaris table names allow at most one namespace separator.".into());
    }
    for part in &parts {
        if part.is_empty()
            || part.len() > 64
            || !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || part.starts_with(|c: char| c.is_ascii_digit())
        {
            return Err("Zetaris table names must be simple identifiers.".into());
        }
    }
    Ok(value.to_string())
}

fn case_id_literal(value: &str) -> Result<String, String> {
    if value.len() > 64 || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Case id is not safe to inline.".into());
    }
    Ok(value.to_string())
}

fn safe_query_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 200
        && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn safe_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 4096
        && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~')
}

impl ZetarisConfig {
    async fn token(&self, http: &reqwest::Client) -> Result<String, String> {
        match &self.auth {
            Auth::Key(key) => Ok(key.clone()),
            Auth::User { username, password } => {
                let response = http
                    .post(format!("{}/api/auth/login", self.base_url))
                    .header("X-Request-ID", Uuid::new_v4().to_string())
                    .json(&json!({ "username": username, "password": password }))
                    .send()
                    .await
                    .map_err(|err| format!("Zetaris login failed: {err}"))?;
                let status = response.status();
                if !status.is_success() {
                    return Err(format!("Zetaris login returned HTTP {status}. Check ZETARIS_USERNAME and ZETARIS_PASSWORD."));
                }
                let body: Value = response.json().await.map_err(|err| format!("Zetaris login response was unreadable: {err}"))?;
                let token = body
                    .get("idToken")
                    .or_else(|| body.get("token"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !safe_token(token) {
                    return Err("Zetaris login did not return an idToken.".into());
                }
                Ok(token.to_string())
            }
        }
    }

    fn auth_headers(
        &self,
        request: reqwest::RequestBuilder,
        token: &str,
        request_id: &str,
    ) -> Result<reqwest::RequestBuilder, String> {
        let mut request = request
            .header("Authorization", format!("Bearer {token}"))
            .header("X-Request-ID", request_id);
        if let Some(org) = &self.org_id {
            request = request.header("X-Org-ID", org);
        }
        Ok(request)
    }
}

pub async fn check(config: &ZetarisConfig, http: &reqwest::Client) -> ZetarisStatus {
    let request_id = Uuid::new_v4().to_string();
    let result = async {
        let token = config.token(http).await?;
        let path = match config.style {
            ApiStyle::Gateway => "/api/proxy/lightning-database/databases",
            ApiStyle::Rest => "/datasource/datasources",
            ApiStyle::Saas => "/api/v1.0/datamart",
        };
        if config.style == ApiStyle::Rest {
            let _ = config.org_header()?;
        }
        let request = http.get(format!("{}{path}", config.base_url));
        let request = config.auth_headers(request, &token, &request_id)?;
        let response = request.send().await.map_err(|err| format!("Zetaris could not be reached: {err}"))?;
        let status = response.status();
        if !status.is_success() {
            let hint = match status.as_u16() {
                401 | 403 => "The API key or login token was rejected.",
                400 => "The organisation id or request shape was rejected.",
                404 => "The API path was not found at this base URL.",
                _ => "Check the workspace URL and credentials.",
            };
            return Err(format!("Zetaris {} API returned HTTP {status}. {hint}", config.style.name()));
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let count = body
            .as_array()
            .map(|a| a.len())
            .or_else(|| body.get("data").and_then(|v| v.as_array()).map(|a| a.len()))
            .or_else(|| body.get("datasources").and_then(|v| v.as_array()).map(|a| a.len()))
            .unwrap_or(0);
        Ok::<usize, String>(count)
    }
    .await;
    match result {
        Ok(count) => ZetarisStatus {
            state: "connected".into(),
            detail: format!(
                "Live Zetaris {} API answered; {count} entries returned.",
                config.style.name()
            ),
            request_id: Some(request_id),
            live: true,
            style: config.style.name().into(),
        },
        Err(detail) => ZetarisStatus {
            state: "error".into(),
            detail,
            request_id: Some(request_id),
            live: false,
            style: config.style.name().into(),
        },
    }
}

pub async fn query_join(config: &ZetarisConfig, http: &reqwest::Client, case_id: &str) -> Result<ZetarisQuery, String> {
    let sql = join_sql(&config.project_table, &config.evidence_table, case_id)?;
    match config.style {
        ApiStyle::Gateway => query_gateway(config, http, sql).await,
        ApiStyle::Rest => query_rest(config, http, sql).await,
        ApiStyle::Saas => query_saas(config, http, sql).await,
    }
}

async fn query_gateway(config: &ZetarisConfig, http: &reqwest::Client, sql: String) -> Result<ZetarisQuery, String> {
    let token = config.token(http).await?;
    let request_id = Uuid::new_v4().to_string();
    let request = http.post(format!("{}/api/proxy/sql-editor/sqls/run-query", config.base_url));
    let request = config.auth_headers(request, &token, &request_id)?;
    let response = request
        .json(&gateway_body(&sql, 100))
        .send()
        .await
        .map_err(|err| format!("Zetaris query failed: {err}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|err| format!("Zetaris returned an unreadable response: {err}"))?;
    if !status.is_success() {
        return Err(format!("Zetaris gateway query returned HTTP {status}."));
    }
    let rows = body
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| body.get("rows").and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default();
    Ok(ZetarisQuery {
        request_id,
        sql,
        total_count: rows.len() as i64,
        record_sample: rows.into_iter().take(5).collect(),
        live: true,
        style: "gateway".into(),
        query_closed: true,
    })
}

async fn query_rest(config: &ZetarisConfig, http: &reqwest::Client, sql: String) -> Result<ZetarisQuery, String> {
    let token = config.token(http).await?;
    let _ = config.org_header()?;
    let request_id = Uuid::new_v4().to_string();
    let request = http.post(format!("{}/sql-editor/sqls/run", config.base_url));
    let request = config.auth_headers(request, &token, &request_id)?;
    let response = request
        .json(&rest_body(&sql, 100))
        .send()
        .await
        .map_err(|err| format!("Zetaris query failed: {err}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|err| format!("Zetaris returned an unreadable response: {err}"))?;
    if !status.is_success() {
        return Err(format!("Zetaris REST query returned HTTP {status}."));
    }
    let rows = body.get("data").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(ZetarisQuery {
        request_id,
        sql,
        total_count: rows.len() as i64,
        record_sample: rows.into_iter().take(5).collect(),
        live: true,
        style: "rest".into(),
        query_closed: true,
    })
}

async fn query_saas(config: &ZetarisConfig, http: &reqwest::Client, sql: String) -> Result<ZetarisQuery, String> {
    let token = config.token(http).await?;
    let _ = config.org_header()?;
    let request_id = Uuid::new_v4().to_string();
    let request = http.post(format!("{}/api/v1.0/query/sql/start", config.base_url));
    let request = config.auth_headers(request, &token, &request_id)?;
    let response = request
        .json(&saas_start_body(&sql, 100))
        .send()
        .await
        .map_err(|err| format!("Zetaris query failed: {err}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|err| format!("Zetaris returned an unreadable response: {err}"))?;
    if !status.is_success() {
        return Err(format!("Zetaris SaaS query returned HTTP {status}."));
    }
    let token = body.get("queryToken").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let total = body.get("totalCount").and_then(|v| v.as_i64()).unwrap_or(0);
    let records = body.get("records").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let closed = if safe_query_token(&token) {
        let close = http.delete(format!("{}/api/v1.0/query/sql/close/{token}", config.base_url));
        let close = config.auth_headers(close, &token, &Uuid::new_v4().to_string()).ok();
        match close {
            Some(request) => request.send().await.map(|r| r.status().is_success()).unwrap_or(false),
            None => false,
        }
    } else {
        false
    };
    Ok(ZetarisQuery {
        request_id,
        sql,
        total_count: total,
        record_sample: records.into_iter().take(5).collect(),
        live: true,
        style: "saas".into(),
        query_closed: closed,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZetarisQuery {
    pub request_id: String,
    pub sql: String,
    pub total_count: i64,
    pub record_sample: Vec<Value>,
    pub live: bool,
    pub style: String,
    pub query_closed: bool,
}

pub fn not_configured() -> ZetarisStatus {
    ZetarisStatus {
        state: "not_configured".into(),
        detail: "AWS quickstart callback succeeded, but no Zetaris Gateway URL plus API key or login is configured. No live query was sent.".into(),
        request_id: None,
        live: false,
        style: "gateway".into(),
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
        assert!(!sql.contains("${"));
    }

    #[test]
    fn rejects_injected_table_names() {
        assert!(join_sql("project_lines; drop", "evidence_extracts", "harbor-annex").is_err());
        assert!(join_sql("project_lines", "evidence_extracts", "harbor' OR 1=1").is_err());
        assert!(join_sql("1project", "evidence", "x").is_err());
        assert!(join_sql("a.b.c", "evidence", "x").is_err());
        assert!(join_sql("harbor.", "evidence", "x").is_err());
    }

    #[test]
    fn accepts_a_single_namespace() {
        let sql = join_sql("harbor.project_lines", "harbor.evidence_extracts", "x").unwrap();
        assert!(sql.contains("FROM harbor.project_lines p"));
        assert!(sql.contains("JOIN harbor.evidence_extracts e"));
    }

    #[test]
    fn saas_body_matches_the_published_schema() {
        let body = saas_start_body("select 1", 500);
        assert_eq!(body["select"], "select 1");
        assert_eq!(body["pageLimit"], 100);
        assert!(body.get("sql").is_none());
    }

    #[test]
    fn gateway_body_matches_the_proxy_contract() {
        let body = gateway_body("select 1", 100);
        assert_eq!(body["sql"], "select 1");
        assert_eq!(body["limit"], 100);
        assert!(body.get("queryId").is_none());
    }

    #[test]
    fn rest_body_matches_the_documented_contract() {
        let body = rest_body("select 1", 25);
        assert_eq!(body["source"], "SqlEditor");
        assert_eq!(body["limit"], 25);
    }

    #[test]
    fn style_parsing_defaults_to_gateway() {
        assert_eq!(ApiStyle::parse(None), ApiStyle::Gateway);
        assert_eq!(ApiStyle::parse(Some("rest")), ApiStyle::Rest);
        assert_eq!(ApiStyle::parse(Some("saas")), ApiStyle::Saas);
    }

    #[test]
    fn style_is_detected_from_a_pasted_url() {
        assert_eq!(ApiStyle::detect("https://api.saas.enterprise.zetaris.com"), ApiStyle::Saas);
        assert_eq!(ApiStyle::detect("http://localhost:8888/api/v1.0"), ApiStyle::Rest);
        assert_eq!(ApiStyle::detect("https://cluster.example.com"), ApiStyle::Gateway);
    }

    #[test]
    fn base_url_is_normalized_per_style() {
        assert_eq!(normalize_base("https://cluster.example/", ApiStyle::Gateway), "https://cluster.example");
        assert_eq!(normalize_base("https://cluster.example/api/v1.0", ApiStyle::Gateway), "https://cluster.example");
        assert_eq!(normalize_base("http://localhost:8888/api/v1.0/", ApiStyle::Rest), "http://localhost:8888/api/v1.0");
        assert_eq!(normalize_base("http://localhost:8888", ApiStyle::Rest), "http://localhost:8888/api/v1.0");
    }

    #[test]
    fn tokens_are_bounded() {
        assert!(safe_token("abc-123_XYZ.jwt"));
        assert!(!safe_token("../close"));
        assert!(!safe_token(""));
    }
}
