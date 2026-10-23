use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use extrawork::domain::{self, Decision, Doc, Kind};
use extrawork::extract::parse_upload;
use extrawork::zetaris::{self, ZetarisConfig};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::{limit::RequestBodyLimitLayer, services::ServeDir, trace::TraceLayer};
use uuid::Uuid;

#[derive(Clone)]
struct App {
    db: PgPool,
    http: reqwest::Client,
    root: PathBuf,
    zetaris: Option<ZetarisConfig>,
    meterless_url: String,
    nvidia_key: Option<String>,
    nemotron_model: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let root = PathBuf::from(std::env::var("EXTRAWORK_ROOT").unwrap_or_else(|_| ".".into()));
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://extrawork:extrawork@127.0.0.1:54329/extrawork".into());
    let db = PgPoolOptions::new().max_connections(5).connect(&database_url).await?;
    migrate(&db).await?;
    let app_state = Arc::new(App {
        db,
        http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?,
        root: root.clone(),
        zetaris: ZetarisConfig::from_env(),
        meterless_url: std::env::var("METERLESS_URL").unwrap_or_else(|_| "http://127.0.0.1:8091".into()),
        nvidia_key: std::env::var("NVIDIA_API_KEY").ok().filter(|s| !s.is_empty()),
        nemotron_model: std::env::var("NEMOTRON_MODEL").unwrap_or_else(|_| "nvidia/nemotron-3-nano-30b-a3b".into()),
    });
    seed_if_empty(&app_state).await?;
    let dist = root.join("web/dist");
    let api = Router::new()
        .route("/health", get(health))
        .route("/cases/{id}", get(get_case))
        .route("/cases/{id}/reset", post(reset_case))
        .route("/cases/{id}/decisions", post(decide))
        .route("/cases/{id}/sources", post(upload))
        .route("/cases/{id}/documents/{doc_id}", get(document))
        .route("/cases/{id}/export", get(export_html))
        .route("/cases/{id}/export.zip", get(export_zip));
    let mut router = Router::new().nest("/api", api).layer(RequestBodyLimitLayer::new(6_000_000)).layer(TraceLayer::new_for_http()).with_state(app_state);
    if dist.join("index.html").exists() {
        router = router.fallback_service(ServeDir::new(dist));
    }
    let bind: SocketAddr = std::env::var("EXTRAWORK_BIND").unwrap_or_else(|_| "127.0.0.1:8081".into()).parse()?;
    tracing::info!("ExtraWork listening on http://{bind}");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

async fn migrate(db: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS cases (
            id TEXT PRIMARY KEY,
            revision INT NOT NULL,
            synthetic BOOLEAN NOT NULL,
            meta JSONB NOT NULL,
            proof JSONB NOT NULL DEFAULT '{}'::jsonb
        )",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS documents (
            id TEXT PRIMARY KEY,
            case_id TEXT NOT NULL,
            filename TEXT NOT NULL,
            kind TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            body TEXT NOT NULL
        )",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS decisions (
            case_id TEXT NOT NULL,
            finding_key TEXT NOT NULL,
            decision TEXT NOT NULL,
            PRIMARY KEY (case_id, finding_key)
        )",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS project_lines (
            case_id TEXT NOT NULL,
            line_id TEXT NOT NULL,
            amount_cents BIGINT NOT NULL,
            PRIMARY KEY (case_id, line_id)
        )",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS case_extracts (
            case_id TEXT NOT NULL,
            line_id TEXT NOT NULL,
            document TEXT NOT NULL,
            excerpt TEXT NOT NULL
        )",
    )
    .execute(db)
    .await?;
    Ok(())
}

const HARBOR_FILES: &[&str] = &[
    "return-note.eml",
    "cost-export.csv",
    "invoice-902.txt",
    "invoice-441.txt",
    "field-ticket.txt",
    "base-scope.txt",
    "directive.eml",
    "rate-sheet.csv",
    "prior-billing.csv",
];

const NORTHGATE_FILES: &[&str] = &[
    "return-note.eml",
    "cost-export.csv",
    "delivery-receipt.csv",
    "base-scope.txt",
    "directive.eml",
    "rate-sheet.csv",
];

fn case_spec(id: &str) -> Option<(&'static str, &'static [&'static str], Value)> {
    match id {
        "harbor-annex" => Some((
            "harbor-annex",
            HARBOR_FILES,
            json!({
                "project": "Harbor Annex",
                "requestNumber": "COR-184",
                "contractor": "Northline Electrical",
                "counterparty": "Apex Build",
                "returnedOn": "2026-10-18",
                "summary": "Temporary power relocation before the slab pour"
            }),
        )),
        "northgate-mechanical" => Some((
            "northgate-mechanical",
            NORTHGATE_FILES,
            json!({
                "project": "Northgate Plant",
                "requestNumber": "CHG-207",
                "contractor": "Meridian Mechanical",
                "counterparty": "Calder Industrial",
                "returnedOn": "2026-10-21",
                "summary": "Valve package and crane for the Northgate shutdown"
            }),
        )),
        _ => None,
    }
}

async fn seed_if_empty(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    for id in ["harbor-annex", "northgate-mechanical"] {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cases WHERE id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await?;
        if count == 0 {
            reset_case_seed(app, id).await?;
        }
    }
    Ok(())
}

async fn reset_case_seed(app: &App, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let Some((dir_name, files, meta)) = case_spec(id) else {
        return Err(format!("unknown case {id}").into());
    };
    let dir = app.root.join("fixtures").join(dir_name);
    for table in ["decisions", "documents", "project_lines", "case_extracts"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE case_id = $1"))
            .bind(id)
            .execute(&app.db)
            .await?;
    }
    sqlx::query("INSERT INTO cases (id, revision, synthetic, meta, proof) VALUES ($1, 1, true, $2, '{}') ON CONFLICT (id) DO UPDATE SET revision = 1, synthetic = true, meta = $2, proof = '{}'")
        .bind(id)
        .bind(meta)
        .execute(&app.db)
        .await?;
    for name in files {
        let bytes = std::fs::read(dir.join(name))?;
        let doc = parse_upload(name, &bytes).map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        insert_doc(app, id, &doc).await?;
    }
    seed_evidence(app, id, dir_name)?;
    sync_project_lines(app, id).await?;
    let _ = project_world(app, id).await;
    Ok(())
}

async fn insert_doc(app: &App, case_id: &str, doc: &Doc) -> Result<(), sqlx::Error> {
    let kind = serde_json::to_value(doc.kind).unwrap().as_str().unwrap().to_string();
    // Content hashes are global; the stored id must be case-scoped so two cases can
    // legitimately contain the same file without one silently disappearing.
    let scoped_id = format!("{case_id}-{}", doc.id);
    sqlx::query("INSERT INTO documents (id, case_id, filename, kind, sha256, body) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING")
        .bind(&scoped_id)
        .bind(case_id)
        .bind(&doc.filename)
        .bind(kind)
        .bind(&doc.sha256)
        .bind(&doc.text)
        .execute(&app.db)
        .await?;
    Ok(())
}

async fn load_docs(app: &App, case_id: &str) -> Result<Vec<Doc>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, filename, kind, sha256, body FROM documents WHERE case_id = $1 ORDER BY filename")
        .bind(case_id)
        .fetch_all(&app.db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| Doc {
            id: row.get("id"),
            filename: row.get("filename"),
            kind: kind_from(row.get("kind")),
            sha256: row.get("sha256"),
            text: row.get("body"),
        })
        .collect())
}

fn kind_from(value: String) -> Kind {
    serde_json::from_value(Value::String(value)).unwrap_or(Kind::Other)
}

async fn load_decisions(app: &App, case_id: &str) -> Result<Vec<Decision>, sqlx::Error> {
    let rows = sqlx::query("SELECT finding_key, decision FROM decisions WHERE case_id = $1").bind(case_id).fetch_all(&app.db).await?;
    Ok(rows
        .into_iter()
        .map(|row| Decision { finding_key: row.get("finding_key"), decision: row.get("decision") })
        .collect())
}

async fn load_case_extracts(app: &App, case_id: &str) -> Result<Vec<domain::ExtractRow>, sqlx::Error> {
    let rows = sqlx::query("SELECT line_id, document, excerpt FROM case_extracts WHERE case_id = $1 ORDER BY line_id, document")
        .bind(case_id)
        .fetch_all(&app.db)
        .await?;
    if rows.is_empty() {
        return Ok(domain::parse_extracts(&evidence_text(app, case_id)).unwrap_or_default());
    }
    Ok(rows
        .into_iter()
        .map(|row| domain::ExtractRow {
            line_id: row.get("line_id"),
            document: row.get("document"),
            excerpt: row.get("excerpt"),
        })
        .collect())
}

async fn replace_case_extracts(app: &App, case_id: &str, rows: &[domain::ExtractRow]) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM case_extracts WHERE case_id = $1").bind(case_id).execute(&app.db).await?;
    for row in rows {
        sqlx::query("INSERT INTO case_extracts (case_id, line_id, document, excerpt) VALUES ($1, $2, $3, $4)")
            .bind(case_id)
            .bind(&row.line_id)
            .bind(&row.document)
            .bind(&row.excerpt)
            .execute(&app.db)
            .await?;
    }
    Ok(())
}

fn extracts_from_query(query: &zetaris::ZetarisQuery) -> Vec<domain::ExtractRow> {
    let index = |name: &str| query.headers.iter().position(|h| h == name);
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    if let (Some(line_idx), Some(doc_idx), Some(excerpt_idx)) =
        (index("line_id"), index("document"), index("excerpt"))
    {
        for row in &query.record_sample {
            let Some(values) = row.as_array() else { continue };
            let get = |idx: usize| values.get(idx).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let (line_id, document, excerpt) = (get(line_idx), get(doc_idx), get(excerpt_idx));
            if line_id.is_empty() || document.is_empty() || excerpt.is_empty() {
                continue;
            }
            if seen.insert(format!("{line_id}|{document}|{excerpt}")) {
                out.push(domain::ExtractRow { line_id, document, excerpt });
            }
        }
    } else {
        for row in &query.record_sample {
            let Some(object) = row.as_object() else { continue };
            let get = |key: &str| object.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let (line_id, document, excerpt) = (get("line_id"), get("document"), get("excerpt"));
            if line_id.is_empty() || document.is_empty() || excerpt.is_empty() {
                continue;
            }
            if seen.insert(format!("{line_id}|{document}|{excerpt}")) {
                out.push(domain::ExtractRow { line_id, document, excerpt });
            }
        }
    }
    out
}

async fn sync_project_lines(app: &App, case_id: &str) -> Result<(), sqlx::Error> {
    let docs = load_docs(app, case_id).await?;
    if let Some(csv) = docs.iter().find(|d| d.kind == Kind::CostExport) {
        if let Ok(lines) = domain::parse_cost_csv(&csv.text) {
            sqlx::query("DELETE FROM project_lines WHERE case_id = $1").bind(case_id).execute(&app.db).await?;
            for line in lines {
                sqlx::query("INSERT INTO project_lines (case_id, line_id, amount_cents) VALUES ($1, $2, $3)")
                    .bind(case_id)
                    .bind(line.id)
                    .bind(line.amount_cents)
                    .execute(&app.db)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn db_amounts(app: &App, case_id: &str) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows = sqlx::query("SELECT line_id, amount_cents FROM project_lines WHERE case_id = $1").bind(case_id).fetch_all(&app.db).await?;
    Ok(rows.into_iter().map(|row| (row.get("line_id"), row.get("amount_cents"))).collect())
}

fn evidence_text(app: &App, case_id: &str) -> String {
    let case_path = app.root.join("data/evidence").join(format!("{case_id}.csv"));
    if case_path.exists() {
        return std::fs::read_to_string(case_path).unwrap_or_default();
    }
    let dir = case_spec(case_id).map(|(dir, _, _)| dir).unwrap_or("harbor-annex");
    std::fs::read_to_string(app.root.join("fixtures").join(dir).join("evidence-extracts.csv")).unwrap_or_default()
}

fn seed_evidence(app: &App, case_id: &str, dir_name: &str) -> std::io::Result<()> {
    let dir = app.root.join("data/evidence");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{case_id}.csv"));
    let source = app.root.join("fixtures").join(dir_name).join("evidence-extracts.csv");
    std::fs::copy(source, target)?;
    Ok(())
}

fn append_evidence(app: &App, case_id: &str, doc: &Doc, lines: &[domain::CostLine]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = app.root.join("data/evidence");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{case_id}.csv"));
    let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| "line_id,document,excerpt\n".into());
    let mut rows = Vec::new();
    for line in lines {
        let whole = line.amount_cents / 100;
        let needles = [
            format!("{whole}.00"),
            format!("${whole}.00"),
            format!("{},", group_dollars(whole)),
            format!("${},", group_dollars(whole)),
            format!("{whole}"),
        ];
        let Some(sentence) = doc.text.split(['.', '\n']).find(|s| needles.iter().any(|n| s.contains(n))) else {
            continue;
        };
        let excerpt = sentence.trim();
        if excerpt.is_empty() || excerpt.len() > 400 {
            continue;
        }
        let row = format!("{},{},{}", line.id, doc.filename, csv_escape(excerpt));
        if !existing.contains(&row) {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    for row in rows {
        writeln!(file, "{row}")?;
    }
    Ok(())
}

fn group_dollars(dollars: i64) -> String {
    let text = dollars.to_string();
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i > 0 && (text.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

async fn compose(app: &App, case_id: &str) -> Result<Value, sqlx::Error> {
    let case_row = sqlx::query("SELECT revision, synthetic, meta, proof FROM cases WHERE id = $1").bind(case_id).fetch_optional(&app.db).await?;
    let Some(row) = case_row else {
        return Ok(Value::Null);
    };
    let docs = load_docs(app, case_id).await?;
    let decisions = load_decisions(app, case_id).await?;
    let amounts = db_amounts(app, case_id).await?;
    let extracts = load_case_extracts(app, case_id).await.unwrap_or_else(|_| {
        domain::parse_extracts(&evidence_text(app, case_id)).unwrap_or_default()
    });
    let analysis = domain::analyze(&docs, &extracts, &amounts, &decisions);
    let documents: Vec<Value> = docs
        .iter()
        .map(|d| json!({"id": d.id, "filename": d.filename, "kind": d.kind, "sha256": d.sha256}))
        .collect();
    let mut view = serde_json::to_value(&analysis).unwrap_or(json!({}));
    let meta: Value = row.get("meta");
    view["id"] = json!(case_id);
    view["synthetic"] = json!(row.get::<bool, _>("synthetic"));
    view["revision"] = json!(row.get::<i32, _>("revision"));
    view["project"] = meta.get("project").cloned().unwrap_or(json!(""));
    view["requestNumber"] = meta.get("requestNumber").cloned().unwrap_or(json!(""));
    view["contractor"] = meta.get("contractor").cloned().unwrap_or(json!(""));
    view["counterparty"] = meta.get("counterparty").cloned().unwrap_or(json!(""));
    view["returnedOn"] = meta.get("returnedOn").cloned().unwrap_or(json!(""));
    view["summary"] = meta.get("summary").cloned().unwrap_or(json!(""));
    view["documents"] = json!(documents);
    view["proof"] = row.get("proof");
    Ok(view)
}

async fn health(State(app): State<Arc<App>>) -> Json<Value> {
    let zetaris = match &app.zetaris {
        Some(config) => serde_json::to_value(zetaris::check(config, &app.http).await).unwrap_or(json!({})),
        None => serde_json::to_value(zetaris::not_configured()).unwrap_or(json!({})),
    };
    let meterless = app.http.get(format!("{}/health", app.meterless_url)).send().await.ok();
    let meterless_ok = meterless.as_ref().map(|r| r.status().is_success()).unwrap_or(false);
    Json(json!({
        "ok": true,
        "zetaris": zetaris,
        "meterless": {"state": if meterless_ok {"connected"} else {"unavailable"}, "url": app.meterless_url, "engine": "world-model"},
        "nemotron": {"state": if app.nvidia_key.is_some() {"configured"} else {"not_configured"}, "model": app.nemotron_model},
        "onboarding": {"callback": "CREATE_COMPLETE", "region": "ap-southeast-1", "stack": "zetaris-main-9acdd8a3", "query": "credentials not in this process"}
    }))
}

async fn get_case(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if !safe_id(&id) {
        return (StatusCode::BAD_REQUEST, "bad id").into_response();
    }
    match compose(&app, &id).await {
        Ok(Value::Null) => StatusCode::NOT_FOUND.into_response(),
        Ok(view) => Json(view).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn reset_case(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    if case_spec(&id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(err) = reset_case_seed(&app, &id).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    get_case(State(app), Path(id)).await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DecisionBody {
    finding_key: String,
    decision: String,
    expected_revision: i32,
}

async fn decide(State(app): State<Arc<App>>, Path(id): Path<String>, Json(body): Json<DecisionBody>) -> Response {
    if !safe_id(&id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let revision: i32 = match sqlx::query_scalar("SELECT revision FROM cases WHERE id = $1").bind(&id).fetch_optional(&app.db).await {
        Ok(Some(v)) => v,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };
    if revision != body.expected_revision {
        return (StatusCode::CONFLICT, "This file changed. Reload it before deciding.").into_response();
    }
    if sqlx::query("INSERT INTO decisions (case_id, finding_key, decision) VALUES ($1, $2, $3) ON CONFLICT (case_id, finding_key) DO UPDATE SET decision = $3")
        .bind(&id)
        .bind(&body.finding_key)
        .bind(&body.decision)
        .execute(&app.db)
        .await
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let _ = sqlx::query("UPDATE cases SET revision = revision + 1 WHERE id = $1").bind(&id).execute(&app.db).await;
    let _ = project_world(&app, &id).await;
    get_case(State(app), Path(id)).await
}

async fn upload(State(app): State<Arc<App>>, Path(id): Path<String>, mut multipart: Multipart) -> Response {
    if !safe_id(&id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut saved = false;
    let mut uploaded: Option<Doc> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("upload.txt").to_string();
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(_) => return (StatusCode::BAD_REQUEST, "Could not read the upload.").into_response(),
        };
        match parse_upload(&filename, &bytes) {
            Ok(doc) => {
                if insert_doc(&app, &id, &doc).await.is_err() {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                saved = true;
                uploaded = Some(doc);
            }
            Err(err) => return (StatusCode::BAD_REQUEST, err).into_response(),
        }
    }
    if !saved {
        return (StatusCode::BAD_REQUEST, "Choose a file first.").into_response();
    }
    if let (Some(doc), Ok(docs)) = (uploaded.as_ref(), load_docs(&app, &id).await) {
        if let Some(csv) = docs.iter().find(|d| d.kind == Kind::CostExport) {
            if let Ok(lines) = domain::parse_cost_csv(&csv.text) {
                let _ = append_evidence(&app, &id, doc, &lines);
            }
        }
    }
    let _ = sync_project_lines(&app, &id).await;
    let _ = sqlx::query("UPDATE cases SET revision = revision + 1 WHERE id = $1").bind(&id).execute(&app.db).await;
    let _ = project_world(&app, &id).await;
    get_case(State(app), Path(id)).await
}

async fn document(State(app): State<Arc<App>>, Path((id, doc_id)): Path<(String, String)>) -> Response {
    if !safe_id(&id) || !safe_id(&doc_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match sqlx::query("SELECT filename, kind, sha256, body FROM documents WHERE case_id = $1 AND id = $2")
        .bind(&id)
        .bind(&doc_id)
        .fetch_optional(&app.db)
        .await
    {
        Ok(Some(row)) => Json(json!({
            "id": doc_id,
            "filename": row.get::<String, _>("filename"),
            "kind": row.get::<String, _>("kind"),
            "sha256": row.get::<String, _>("sha256"),
            "text": row.get::<String, _>("body")
        }))
        .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn export_html(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    match compose(&app, &id).await {
        Ok(Value::Null) => StatusCode::NOT_FOUND.into_response(),
        Ok(view) => attachment("text/html; charset=utf-8", "COR-184-repair.html", render_packet(&view).into_bytes()),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn export_zip(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let view = match compose(&app, &id).await {
        Ok(Value::Null) => return StatusCode::NOT_FOUND.into_response(),
        Ok(view) => view,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };
    let docs = match load_docs(&app, &id).await {
        Ok(d) => d,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default();
        let _ = zip.start_file("packet.html", opts);
        let _ = std::io::Write::write_all(&mut zip, render_packet(&view).as_bytes());
        let _ = zip.start_file("manifest.json", opts);
        let _ = std::io::Write::write_all(&mut zip, serde_json::to_string_pretty(&view).unwrap_or_default().as_bytes());
        for doc in docs {
            let _ = zip.start_file(format!("sources/{}", doc.filename), opts);
            let _ = std::io::Write::write_all(&mut zip, doc.text.as_bytes());
        }
        let _ = zip.finish();
    }
    attachment("application/zip", "COR-184-repair.zip", cursor.into_inner())
}

fn attachment(content_type: &str, filename: &str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\""))
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn project_world(app: &App, case_id: &str) -> Result<(), String> {
    let mut join_mode = "local_two_source";
    let mut zetaris_proof = if app.zetaris.is_some() {
        json!({"state": "pending"})
    } else {
        serde_json::to_value(zetaris::not_configured()).unwrap_or(json!({}))
    };
    if let Some(config) = &app.zetaris {
        match zetaris::query_join(config, &app.http, case_id).await {
            Ok(query) => {
                let rows = extracts_from_query(&query);
                if !rows.is_empty() {
                    replace_case_extracts(app, case_id, &rows).await.map_err(|e| e.to_string())?;
                    join_mode = "zetaris_live";
                }
                zetaris_proof = json!({
                    "state": "connected",
                    "live": true,
                    "style": query.style,
                    "detail": format!("Gateway join returned {} extracts from the registered tables.", rows.len()),
                    "requestId": query.request_id,
                    "totalCount": query.total_count,
                    "extractsUsed": rows.len(),
                    "queryClosed": query.query_closed,
                    "sql": query.sql
                });
            }
            Err(err) => {
                zetaris_proof = json!({"state": "error", "live": false, "detail": err});
            }
        }
    }
    let view = compose(app, case_id).await.map_err(|e| e.to_string())?;
    let revision = view.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
    let projection = json!({
        "caseId": case_id,
        "observedAt": "2026-10-18T15:04:00.000Z",
        "revision": revision,
        "entities": projection_entities(&view),
        "links": projection_links(&view),
        "facts": projection_facts(&view)
    });
    let response = app.http.post(format!("{}/project", app.meterless_url)).json(&projection).send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let mut body: Value = response.json().await.unwrap_or(json!({"error": "unreadable world model response"}));
    if let Some(object) = body.as_object_mut() {
        object.remove("shape");
    }
    let nemotron = nemotron_status(app, &view).await;
    let proof = json!({
        "joinMode": join_mode,
        "joinDetail": view.get("sourceAgreement").cloned().unwrap_or(json!("")),
        "zetaris": zetaris_proof,
        "meterless": {
            "state": if status.is_success() { "recorded" } else { "error" },
            "engine": "world-model",
            "commit": "1742dec56a66e58eef9dda2d2b865823758ca8cd",
            "httpStatus": status.as_u16(),
            "result": body
        },
        "nemotron": nemotron,
        "rulesEngine": true,
        "recordedAt": Utc::now().to_rfc3339()
    });
    sqlx::query("UPDATE cases SET proof = $1 WHERE id = $2").bind(proof).bind(case_id).execute(&app.db).await.map_err(|e| e.to_string())?;
    Ok(())
}

fn projection_entities(view: &Value) -> Vec<Value> {
    let mut entities = vec![json!({"type": "case", "id": view["id"], "name": view["requestNumber"], "attrs": {"workflow": view["workflow"]}})];
    if let Some(findings) = view.get("findings").and_then(|v| v.as_array()) {
        for finding in findings {
            entities.push(json!({"type": "finding", "id": finding["key"], "name": finding["title"], "attrs": {"state": finding["state"]}}));
        }
    }
    if let Some(docs) = view.get("documents").and_then(|v| v.as_array()) {
        for doc in docs {
            entities.push(json!({"type": "document", "id": doc["id"], "name": doc["filename"], "attrs": {"kind": doc["kind"]}}));
        }
    }
    entities
}

fn projection_links(view: &Value) -> Vec<Value> {
    let mut links = Vec::new();
    let case_id = view["id"].as_str().unwrap_or("case");
    if let Some(findings) = view.get("findings").and_then(|v| v.as_array()) {
        for finding in findings {
            let key = finding["key"].as_str().unwrap_or("");
            links.push(json!({"from": {"type": "case", "id": case_id}, "to": {"type": "finding", "id": key}, "type": "has"}));
            if let Some(spans) = finding.get("spans").and_then(|v| v.as_array()) {
                for span in spans {
                    links.push(json!({"from": {"type": "finding", "id": key}, "to": {"type": "document", "id": span["documentId"]}, "type": "cites"}));
                }
            }
        }
    }
    links
}

fn projection_facts(view: &Value) -> Vec<Value> {
    let mut facts = vec![json!({"about": {"type": "case", "id": view["id"]}, "predicate": "billingPosture", "value": view["billingPosture"], "confidence": 1})];
    if let Some(findings) = view.get("findings").and_then(|v| v.as_array()) {
        for finding in findings {
            facts.push(json!({"about": {"type": "finding", "id": finding["key"]}, "predicate": "evidenceState", "value": finding["state"], "confidence": 1}));
        }
    }
    facts
}

async fn nemotron_status(app: &App, view: &Value) -> Value {
    let Some(key) = &app.nvidia_key else {
        return json!({"state": "not_configured", "model": app.nemotron_model, "detail": "No NVIDIA_API_KEY. Objections were read by the rules engine."});
    };
    let quotes: Vec<String> = view.get("objections").and_then(|v| v.as_array()).map(|items| items.iter().filter_map(|o| o.get("quote").and_then(|q| q.as_str()).map(|s| s.to_string())).collect()).unwrap_or_default();
    let prompt = format!("Return JSON {{\"quotes\": string[]}} containing only exact objection sentences from this note. Do not add sentences.\n\n{}", quotes.join("\n"));
    let body = json!({"model": app.nemotron_model, "temperature": 0, "max_tokens": 400, "messages": [{"role": "user", "content": prompt}]});
    match app.http.post("https://integrate.api.nvidia.com/v1/chat/completions").bearer_auth(key).json(&body).send().await {
        Ok(res) if res.status().is_success() => json!({"state": "called", "model": app.nemotron_model, "live": true, "detail": "Hosted Nemotron was called. Deterministic checks still decide amounts."}),
        Ok(res) => json!({"state": "error", "model": app.nemotron_model, "live": false, "detail": format!("NVIDIA returned HTTP {}.", res.status())}),
        Err(err) => json!({"state": "error", "model": app.nemotron_model, "live": false, "detail": err.to_string()}),
    }
}

fn packet_money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}${}.{:02}", group_dollars(cents.abs() / 100), cents.abs() % 100)
}

fn packet_citations(item: &Value) -> String {
    let Some(spans) = item.get("spans").and_then(|v| v.as_array()) else {
        return String::new();
    };
    if spans.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ul class=\"cites\">");
    for span in spans {
        out.push_str(&format!(
            "<li><b>{}</b><q>{}</q></li>",
            esc(span["filename"].as_str().unwrap_or("")),
            esc(span["quote"].as_str().unwrap_or(""))
        ));
    }
    out.push_str("</ul>");
    out
}

fn packet_finding(item: &Value) -> String {
    let reservation = item["reservation"].as_str().unwrap_or("");
    format!(
        "<div class=\"finding\"><p class=\"state {state}\">{state}</p><h3>{title}</h3><p>{explanation}</p>{reserve}{cites}</div>",
        state = esc(item["state"].as_str().unwrap_or("")),
        title = esc(item["title"].as_str().unwrap_or("")),
        explanation = esc(item["explanation"].as_str().unwrap_or("")),
        reserve = if reservation.is_empty() {
            String::new()
        } else {
            format!("<p class=\"reserve\">{}</p>", esc(reservation))
        },
        cites = packet_citations(item)
    )
}

fn render_packet(view: &Value) -> String {
    let submitted = view["submittedCents"].as_i64().unwrap_or(0);
    let supported = view["supportedCents"].as_i64().unwrap_or(0);
    let proposal = view["proposalCents"].as_i64().unwrap_or(0);
    let request = view["requestNumber"].as_str().unwrap_or("Request");
    let project = view["project"].as_str().unwrap_or("");
    let contractor = view["contractor"].as_str().unwrap_or("");
    let counterparty = view["counterparty"].as_str().unwrap_or("");
    let returned = view["returnedOn"].as_str().unwrap_or("");
    let summary = view["summary"].as_str().unwrap_or("");
    let workflow = view["workflow"].as_str().unwrap_or("").replace('_', " ");

    let findings = view.get("findings").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let objections = view.get("objections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut sections = String::new();
    for objection in &objections {
        let id = objection["id"].as_str().unwrap_or("");
        let related: Vec<&Value> = findings
            .iter()
            .filter(|item| item["objectionId"].as_str() == Some(id))
            .collect();
        if related.is_empty() {
            continue;
        }
        sections.push_str(&format!(
            "<section><p class=\"num\">0{}</p><blockquote>{}</blockquote>{}</section>",
            objection["index"].as_i64().unwrap_or(0),
            esc(objection["quote"].as_str().unwrap_or("")),
            related.iter().map(|item| packet_finding(item)).collect::<String>()
        ));
    }
    let unlinked: Vec<&Value> = findings
        .iter()
        .filter(|item| item["objectionId"].is_null())
        .collect();
    if !unlinked.is_empty() {
        sections.push_str("<section><p class=\"num\">Also checked</p>");
        sections.push_str(&unlinked.iter().map(|item| packet_finding(item)).collect::<String>());
        sections.push_str("</section>");
    }

    let mut lines = String::new();
    if let Some(items) = view.get("lines").and_then(|v| v.as_array()) {
        for line in items {
            let adjustment = line["adjustmentCents"].as_i64().unwrap_or(0);
            lines.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{} {}</td><td class=\"right\">{}{}</td></tr>",
                esc(line["id"].as_str().unwrap_or("")),
                esc(line["description"].as_str().unwrap_or("")),
                esc(line["quantity"].as_str().unwrap_or("")),
                esc(line["unit"].as_str().unwrap_or("")),
                packet_money(line["amountCents"].as_i64().unwrap_or(0)),
                if adjustment == 0 {
                    String::new()
                } else {
                    format!("<span class=\"adjust\">{} unsupported</span>", packet_money(-adjustment))
                }
            ));
        }
    }

    let mut sources = String::new();
    if let Some(items) = view.get("documents").and_then(|v| v.as_array()) {
        for doc in items {
            let hash = doc["sha256"].as_str().unwrap_or("");
            sources.push_str(&format!(
                "<li><b>{}</b><span>{}</span></li>",
                esc(doc["filename"].as_str().unwrap_or("")),
                esc(hash.get(..12).unwrap_or(hash))
            ));
        }
    }

    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{request} repair packet</title>
<style>
:root{{--paper:#f3eee4;--sheet:#fffdf8;--ink:#1c211e;--muted:#3e4741;--line:#d8cfc0;--accent:#9d3b28;--ok:#1e5c44}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--paper);color:var(--ink);font:16px/1.5 "Source Sans 3","Segoe UI",sans-serif}}
main{{max-width:820px;margin:0 auto;padding:56px 40px 72px}}
.eyebrow{{font-size:12px;letter-spacing:.14em;text-transform:uppercase;color:var(--muted);margin:0 0 10px}}
h1{{font-family:Georgia,serif;font-weight:500;font-size:46px;line-height:1;letter-spacing:-.02em;margin:6px 0 12px}}
h2{{font-size:12px;letter-spacing:.12em;text-transform:uppercase;margin:34px 0 10px}}
h3{{font-family:Georgia,serif;font-weight:500;font-size:22px;line-height:1.15;margin:6px 0}}
.banner{{border-top:1px solid var(--accent);border-bottom:1px solid var(--accent);padding:8px 0;color:var(--accent);letter-spacing:.08em;text-transform:uppercase;font-size:11px}}
.totals{{width:100%;border-collapse:collapse;margin:22px 0 6px}}
.totals td{{padding:10px 8px;border-top:1px solid var(--line);vertical-align:top}}
.totals .label{{font-size:11px;letter-spacing:.1em;text-transform:uppercase;color:var(--muted)}}
.totals .value{{font-size:26px;font-variant-numeric:tabular-nums}}
section{{border-top:1px solid var(--line);padding:18px 0 4px;margin-top:22px}}
.num{{font-size:11px;letter-spacing:.12em;text-transform:uppercase;color:var(--muted);margin:0 0 4px}}
blockquote{{font-family:Georgia,serif;font-size:21px;line-height:1.3;margin:0 0 12px;padding-left:12px;border-left:2px solid var(--ink)}}
.finding{{padding:12px 0;border-top:1px dotted var(--line)}}
.state{{font-size:11px;letter-spacing:.1em;text-transform:uppercase;margin:0;color:var(--muted)}}
.state.supported{{color:var(--ok)}}.state.missing,.state.contradicted{{color:var(--accent)}}.state.ambiguous{{color:#8a5a12}}
.reserve{{color:var(--accent);font-size:14px}}
.cites{{list-style:none;padding:0;margin:8px 0 0;font-size:13px;color:var(--muted)}}
.cites li{{padding:4px 0}}.cites b{{color:var(--ink);font-weight:600;margin-right:6px}}.cites q{{quotes:"“" "”"}}
table.costs{{width:100%;border-collapse:collapse;font-size:14px;font-variant-numeric:tabular-nums}}
table.costs th,table.costs td{{text-align:left;padding:8px 6px;border-top:1px solid var(--line);vertical-align:top}}
table.costs th{{font-size:11px;letter-spacing:.1em;text-transform:uppercase;color:var(--muted)}}
.right{{text-align:right}}.adjust{{display:block;font-size:11px;color:var(--accent)}}
.sources{{list-style:none;padding:0;font-size:13px;color:var(--muted)}}.sources li{{display:flex;justify-content:space-between;gap:16px;padding:4px 0;border-top:1px dotted var(--line)}}.sources b{{color:var(--ink)}}
footer{{margin-top:36px;border-top:1px solid var(--ink);padding-top:14px;font-size:13px;color:var(--muted)}}
@media print{{body{{background:#fff}}main{{padding:24px}}}}
</style></head><body><main>
<p class="banner">Synthetic records · prepared for contractor review · not a billing approval · no money recovered</p>
<p class="eyebrow">{contractor} → {counterparty} · returned {returned} · {workflow}</p>
<h1>{request}: {project}</h1>
<p>{summary}</p>
<table class="totals"><tr>
<td><div class="label">Submitted</div><div class="value">{submitted}</div></td>
<td><div class="label">Supported after review</div><div class="value">{supported}</div></td>
<td><div class="label">If open corrections are accepted</div><div class="value">{proposal}</div></td>
<td><div class="label">Billing posture</div><div class="value">Not authorized</div></td>
</tr></table>
<h2>Objections and findings</h2>
{sections}
<h2>Cost lines</h2>
<table class="costs"><thead><tr><th>Line</th><th>Description</th><th>Quantity</th><th class="right">Amount</th></tr></thead><tbody>{lines}</tbody></table>
<h2>Sources</h2>
<ul class="sources">{sources}</ul>
<footer>A direction to perform work is not an approval of price. A signed field ticket is not a rate agreement. This packet prepares a resubmission; it does not authorize billing.</footer>
</main></body></html>"#,
        request = esc(request),
        project = esc(project),
        contractor = esc(contractor),
        counterparty = esc(counterparty),
        returned = esc(returned),
        workflow = esc(&workflow),
        summary = esc(summary),
        submitted = packet_money(submitted),
        supported = packet_money(supported),
        proposal = packet_money(proposal),
        sections = sections,
        lines = lines,
        sources = sources
    )
}

fn esc(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn safe_id(value: &str) -> bool {
    !value.is_empty() && value.len() < 80 && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[allow(dead_code)]
fn _request_id() -> String {
    Uuid::new_v4().to_string()
}
