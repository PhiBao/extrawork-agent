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
    Ok(())
}

async fn seed_if_empty(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cases WHERE id = 'harbor-annex'").fetch_one(&app.db).await?;
    if count == 0 {
        reset_harbor(app).await?;
    }
    Ok(())
}

async fn reset_harbor(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let dir = app.root.join("fixtures/harbor-annex");
    let names = ["return-note.eml", "cost-export.csv", "invoice-902.txt", "invoice-441.txt", "field-ticket.txt", "base-scope.txt", "directive.eml", "rate-sheet.csv", "prior-billing.csv"];
    sqlx::query("DELETE FROM decisions WHERE case_id = 'harbor-annex'").execute(&app.db).await?;
    sqlx::query("DELETE FROM documents WHERE case_id = 'harbor-annex'").execute(&app.db).await?;
    sqlx::query("DELETE FROM project_lines WHERE case_id = 'harbor-annex'").execute(&app.db).await?;
    let meta = json!({
        "project": "Harbor Annex",
        "requestNumber": "COR-184",
        "contractor": "Northline Electrical",
        "counterparty": "Apex Build",
        "returnedOn": "2026-10-18",
        "summary": "Temporary power relocation before the slab pour"
    });
    sqlx::query("INSERT INTO cases (id, revision, synthetic, meta, proof) VALUES ('harbor-annex', 1, true, $1, '{}') ON CONFLICT (id) DO UPDATE SET revision = 1, synthetic = true, meta = $1, proof = '{}'")
        .bind(meta)
        .execute(&app.db)
        .await?;
    for name in names {
        let bytes = std::fs::read(dir.join(name))?;
        let doc = parse_upload(name, &bytes).map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        insert_doc(app, "harbor-annex", &doc).await?;
    }
    seed_evidence(app, "harbor-annex")?;
    sync_project_lines(app, "harbor-annex").await?;
    let _ = project_world(app, "harbor-annex").await;
    Ok(())
}

async fn insert_doc(app: &App, case_id: &str, doc: &Doc) -> Result<(), sqlx::Error> {
    let kind = serde_json::to_value(doc.kind).unwrap().as_str().unwrap().to_string();
    sqlx::query("INSERT INTO documents (id, case_id, filename, kind, sha256, body) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING")
        .bind(&doc.id)
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
    std::fs::read_to_string(app.root.join("fixtures/harbor-annex/evidence-extracts.csv")).unwrap_or_default()
}

fn seed_evidence(app: &App, case_id: &str) -> std::io::Result<()> {
    let dir = app.root.join("data/evidence");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(format!("{case_id}.csv"));
    let source = app.root.join("fixtures/harbor-annex/evidence-extracts.csv");
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
    let extracts = domain::parse_extracts(&evidence_text(app, case_id)).unwrap_or_default();
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
    if id != "harbor-annex" {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(err) = reset_harbor(&app).await {
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
    let zetaris = if let Some(config) = &app.zetaris {
        match zetaris::query_join(config, &app.http, case_id).await {
            Ok(query) => json!({"state": "connected", "live": true, "requestId": query.request_id, "sql": query.sql}),
            Err(err) => json!({"state": "error", "live": false, "detail": err}),
        }
    } else {
        serde_json::to_value(zetaris::not_configured()).unwrap_or(json!({}))
    };
    let proof = json!({
        "joinMode": "local_two_source",
        "joinDetail": view.get("sourceAgreement").cloned().unwrap_or(json!("")),
        "zetaris": zetaris,
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

fn render_packet(view: &Value) -> String {
    let money = |cents: i64| {
        let sign = if cents < 0 { "-" } else { "" };
        format!("{sign}${}.{:02}", group_dollars(cents.abs() / 100), cents.abs() % 100)
    };
    let submitted = view["submittedCents"].as_i64().unwrap_or(0);
    let supported = view["supportedCents"].as_i64().unwrap_or(0);
    let proposal = view["proposalCents"].as_i64().unwrap_or(0);
    let mut findings = String::new();
    if let Some(items) = view.get("findings").and_then(|v| v.as_array()) {
        for item in items {
            findings.push_str(&format!(
                "<section><h2>{}</h2><p class=\"state\">{}</p><p>{}</p><p class=\"reserve\">{}</p></section>",
                esc(item["title"].as_str().unwrap_or("")),
                esc(item["state"].as_str().unwrap_or("")),
                esc(item["explanation"].as_str().unwrap_or("")),
                esc(item["reservation"].as_str().unwrap_or(""))
            ));
        }
    }
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>COR-184 repair packet</title>
        <style>body{{margin:0;background:#f3eee4;color:#1c211e;font:16px/1.5 "Source Sans 3", "Segoe UI", sans-serif}}main{{max-width:760px;margin:0 auto;padding:48px 24px}}h1{{font-family:Georgia, serif;font-weight:500;font-size:42px;line-height:1.05;margin:8px 0 12px}}.banner{{border-top:1px solid #9d3b28;border-bottom:1px solid #9d3b28;padding:8px 0;color:#9d3b28;letter-spacing:.08em;text-transform:uppercase;font-size:12px}}dl{{display:grid;grid-template-columns:180px 1fr;gap:6px 16px}}.state{{text-transform:uppercase;letter-spacing:.08em;font-size:12px;color:#3e4741}}section{{border-top:1px solid #d8cfc0;padding:16px 0}}.reserve{{color:#9d3b28}}</style></head>
        <body><main><p class="banner">Synthetic records · not a billing approval · not money recovered</p>
        <p>Northline Electrical to Apex Build</p><h1>COR-184 repair</h1>
        <p>{summary}</p>
        <dl><dt>Submitted</dt><dd>{submitted}</dd><dt>Supported after review</dt><dd>{supported}</dd><dt>If the open correction is accepted</dt><dd>{proposal}</dd><dt>Billing posture</dt><dd>Not authorized</dd></dl>
        {findings}
        <p>A direction to perform work is not an approval of price. A signed field ticket is not a rate agreement. This packet is for contractor review.</p>
        </main></body></html>"#,
        summary = esc(view["summary"].as_str().unwrap_or("")),
        submitted = money(submitted),
        supported = money(supported),
        proposal = money(proposal),
        findings = findings
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
