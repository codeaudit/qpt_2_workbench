//! Port of the routing/static section of server.js — every /api route,
//! CORS, NDJSON streaming for the agent bridge, and static file serving
//! from public/ (the Node version served its whole app dir; only the
//! frontend files are exposed here — the API surface is unchanged).

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};
use tokio_stream::wrappers::ReceiverStream;

use crate::agent::{self, AppState};
use crate::jobs::{JobEvent, Status as JobStatus};
use crate::{cli_exec, core, functions, skills, store::StoreData};

const WAIT_JOB_TIMEOUT_MS: u64 = agent::TURN_TIMEOUT_MS + 30_000;
const AGENT_ERROR_HINT: &str =
    "Check that the kimi CLI is installed and logged in (kimi --version), then retry.";

pub fn router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/api/health", axum::routing::any(health))
        .route("/api/models", axum::routing::get(models))
        .route("/api/model", axum::routing::post(set_model))
        .route("/api/state", axum::routing::get(get_state).post(post_state))
        .route("/api/cli", axum::routing::post(cli))
        .route("/api/skills", axum::routing::get(list_skills).post(post_skill).delete(delete_skill))
        .route("/api/skills/all", axum::routing::get(all_skills))
        .route("/api/skills/import", axum::routing::post(import_skill))
        .route("/api/skills/generate", axum::routing::post(generate_skill))
        .route("/api/settings", axum::routing::get(get_settings).put(put_settings))
        .route("/api/keys", axum::routing::get(list_keys).post(post_key).delete(delete_key))
        .route("/api/mcp", axum::routing::get(list_mcp).post(post_mcp).delete(delete_mcp))
        .route("/api/functions", axum::routing::get(list_fns).post(post_fn).delete(delete_fn))
        .route("/api/functions/{name}/run", axum::routing::post(run_fn))
        .route("/api/jobs", axum::routing::get(list_jobs).post(post_job))
        .route("/api/jobs/{id}", axum::routing::get(get_job))
        .route("/api/jobs/{id}/cancel", axum::routing::post(cancel_job))
        .route("/api/agent", axum::routing::post(handle_agent))
        .fallback(static_or_404)
        .layer(axum::middleware::from_fn(cors_and_log))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/* --------------------------------------------------------- middleware */

async fn cors_and_log(req: Request, next: Next) -> Response {
    let t0 = std::time::Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    if method == axum::http::Method::OPTIONS {
        let mut res = Response::new(Body::empty());
        *res.status_mut() = StatusCode::NO_CONTENT;
        add_cors(res.headers_mut());
        return res;
    }

    let mut res = next.run(req).await;
    crate::log::debug(
        "http",
        &[
            ("m", json!(method.as_str())),
            ("p", json!(path)),
            ("s", json!(res.status().as_u16())),
            ("ms", json!(t0.elapsed().as_millis() as u64)),
        ],
    );
    add_cors(res.headers_mut());
    res
}

fn add_cors(headers: &mut HeaderMap) {
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert("access-control-allow-methods", HeaderValue::from_static("GET,POST,OPTIONS"));
    headers.insert("access-control-allow-headers", HeaderValue::from_static("content-type"));
}

/// sendJson(): JSON body + content-type (CORS comes from the middleware).
fn send_json(code: u16, obj: Value) -> Response {
    send_json_raw(code, serde_json::to_string(&obj).unwrap_or_default())
}

fn send_json_raw(code: u16, body: String) -> Response {
    (
        StatusCode::from_u16(code).unwrap_or(StatusCode::OK),
        [("content-type", "application/json; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Reject oversized payloads (parse_body's cap and the extractor layer).
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

async fn parse_body(req: Request) -> Result<Value, Response> {
    let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY_BYTES)
        .await
        .map_err(|_| send_json(400, json!({ "error": "invalid JSON body" })))?;
    serde_json::from_slice(&bytes).map_err(|_| send_json(400, json!({ "error": "invalid JSON body" })))
}

/// JS template-literal coercion for oddball values ("undefined" can't occur
/// here, but null/bool/number can).
fn js_coerce(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => core::js_num_str(n.as_f64().unwrap_or(0.0)),
        other => other.to_string(),
    }
}

/* ------------------------------------------------------------- routes */

async fn health(State(state): State<AppState>) -> Response {
    let store = state.store.lock().await;
    send_json(
        200,
        json!({
            "ok": true,
            "mock": state.mock,
            "backend": if state.mock { "mock" } else { "compat" },
            "executable": state.kimi_executable,
            "model": state.model(),
            "storeVersion": store.data.version,
        }),
    )
}

async fn models(State(state): State<AppState>) -> Response {
    let cfg = agent::list_models(&state).await;
    send_json(200, cfg)
}

async fn set_model(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let wanted = parsed
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // match the JS error shape: 400 {error, models:[ids]}
    let cfg = agent::list_models(&state).await;
    let list = cfg.get("models").and_then(Value::as_array).cloned().unwrap_or_default();
    if let Some(w) = &wanted {
        if !list.is_empty() && !list.iter().any(|m| m.get("id").and_then(Value::as_str) == Some(w)) {
            let ids: Vec<Value> = list.iter().filter_map(|m| m.get("id").cloned()).collect();
            return send_json(400, json!({ "error": format!("unknown model {}", w), "models": ids }));
        }
    }
    if let Ok(mut cur) = state.current_model.write() {
        *cur = wanted;
    }
    let current = state
        .model()
        .or_else(|| cfg.get("defaultModel").and_then(Value::as_str).map(str::to_string));
    crate::log::info("model.set", &[("model", json!(current.clone().unwrap_or_else(|| "default".into())))]);
    send_json(200, json!({ "current": current }))
}

fn state_snapshot(data: &StoreData) -> Value {
    json!({
        "version": data.version,
        "boardId": data.board_id,
        "customSeq": data.custom_seq,
        "cards": data.cards,
    })
}

async fn get_state(State(state): State<AppState>) -> Response {
    let store = state.store.lock().await;
    // StoreData's serde shape IS the snapshot shape — one serialization pass,
    // no intermediate Value tree.
    send_json_raw(200, serde_json::to_string(&store.data).unwrap_or_default())
}

async fn post_state(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let mut store = state.store.lock().await;
    if let Some(cv) = parsed.get("clientVersion").and_then(Value::as_f64) {
        if cv != store.data.version as f64 {
            let mut out = Map::new();
            out.insert("error".into(), json!("version conflict"));
            if let Value::Object(snap) = state_snapshot(&store.data) {
                out.extend(snap);
            }
            return send_json(409, Value::Object(out));
        }
    }
    if let Some(b) = parsed.get("boardId").and_then(Value::as_str) {
        store.data.board_id = b.to_string();
    }
    if let Some(cards) = parsed.get("cards").and_then(Value::as_object) {
        store.data.cards = cards.clone();
    }
    if let Some(cs) = parsed.get("customSeq") {
        if let Some(n) = cs.as_i64().or_else(|| cs.as_f64().map(|f| f as i64)) {
            store.data.custom_seq = n;
        }
    }
    store.save();
    send_json(200, json!({ "version": store.data.version }))
}

async fn cli(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let command = parsed.get("command").and_then(Value::as_str).unwrap_or("").to_string();
    let mut store = state.store.lock().await;
    let r = cli_exec::exec_command(&command, &mut store, &state.paths);
    crate::log::info(
        "cli",
        &[
            ("cmd", json!(command.chars().take(120).collect::<String>())),
            ("ok", json!(r.ok)),
            ("changed", json!(if r.changed { Some(true) } else { None })),
        ],
    );
    send_json(
        if r.ok { 200 } else { 400 },
        json!({ "ok": r.ok, "output": r.output, "changed": r.changed, "version": store.data.version }),
    )
}

async fn list_skills(State(state): State<AppState>) -> Response {
    let (skills, warnings) = skills::list_skills(&state.paths);
    send_json(200, json!({ "skills": skills, "warnings": warnings }))
}

async fn all_skills(State(state): State<AppState>) -> Response {
    let (scopes, builtin) = skills::list_all_skills(&state.paths);
    send_json(200, json!({ "scopes": scopes, "builtin": builtin }))
}

async fn import_skill(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    let result = match id.as_str() {
        Some(s) => skills::import_skill(&state.paths, s),
        None => Err(format!("no external skill {}", js_coerce(&id))),
    };
    match result {
        Ok(skill) => send_json(200, json!({ "skill": skill })),
        Err(msg) => send_json(if msg.contains("already exists") { 409 } else { 404 }, json!({ "error": msg })),
    }
}

/// POST /api/skills — create, or update when the body carries an `id`.
async fn post_skill(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let result = if let Some(id) = parsed.get("id").and_then(Value::as_str) {
        let r = skills::update_skill(&state.paths, id, &parsed);
        if r.is_ok() {
            crate::log::info("skill.update", &[("id", json!(id))]);
        }
        r
    } else {
        let r = skills::create_skill(&state.paths, &parsed);
        if let Ok(s) = &r {
            crate::log::info("skill.create", &[("id", json!(s.id))]);
        }
        r
    };
    match result {
        Ok(skill) => send_json(200, json!({ "skill": skill })),
        Err(msg) => send_json(
            if msg.contains("already exists") {
                409
            } else if msg.contains("no valid") || msg.contains("ENOENT") {
                404
            } else {
                400
            },
            json!({ "error": msg }),
        ),
    }
}

async fn delete_skill(State(state): State<AppState>, Query(q): Query<Map<String, Value>>) -> Response {
    let id = q.get("id").and_then(Value::as_str).map(str::to_string);
    match &id {
        Some(id) if skills::delete_skill(&state.paths, id).is_ok() => {
            crate::log::info("skill.delete", &[("id", json!(id))]);
            send_json(200, json!({ "deleted": id }))
        }
        _ => send_json(
            404,
            json!({ "error": format!("no skill {}", id.as_deref().unwrap_or("null")) }),
        ),
    }
}

async fn generate_skill(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let hint = parsed.get("hint").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if hint.is_empty() {
        return send_json(400, json!({ "error": "hint required" }));
    }
    let job_id = match state.jobs.submit("generate", json!({ "hint": hint })) {
        Ok(id) => id,
        Err(e) => return send_json(400, json!({ "error": e })),
    };
    match state.jobs.wait_job(&job_id, std::time::Duration::from_millis(WAIT_JOB_TIMEOUT_MS)).await {
        Ok(outcome) if outcome.status == JobStatus::Done => {
            let mut out = Map::new();
            out.insert("jobId".into(), json!(job_id));
            if let Some(Value::Object(result)) = outcome.result {
                out.extend(result);
            }
            send_json(200, Value::Object(out))
        }
        Ok(outcome) => {
            let code = outcome.error_status.unwrap_or(if outcome.status == JobStatus::Cancelled { 499 } else { 503 });
            let mut out = Map::new();
            out.insert("jobId".into(), json!(job_id));
            out.insert("error".into(), json!(outcome.error.unwrap_or_else(|| "job failed".into())));
            if let Some(d) = outcome.error_draft {
                out.insert("draft".into(), d);
            }
            if let Some(r) = outcome.error_raw {
                out.insert("raw".into(), json!(r));
            }
            send_json(code, Value::Object(out))
        }
        Err(e) => send_json(503, json!({ "jobId": job_id, "error": e })),
    }
}

/* ------------------------------------------------------------- settings */

async fn get_settings(State(state): State<AppState>) -> Response {
    let settings = state.settings.lock().await;
    send_json(
        200,
        json!({
            "systemPrompt": settings.cfg.system_prompt,
            "defaultSystemPrompt": agent::INSTRUCTIONS,
            "keys": settings.masked_keys(),
            "mcpServers": settings.cfg.mcp_servers,
            "projectMcpPath": settings.project_mcp_path().to_string_lossy(),
        }),
    )
}

async fn put_settings(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let mut settings = state.settings.lock().await;
    if parsed.get("systemPrompt").is_some() {
        let text = parsed.get("systemPrompt").and_then(Value::as_str);
        settings.set_system_prompt(text);
    }
    send_json(200, json!({ "systemPrompt": settings.cfg.system_prompt }))
}

async fn list_keys(State(state): State<AppState>) -> Response {
    let settings = state.settings.lock().await;
    send_json(200, json!({ "keys": settings.masked_keys() }))
}

async fn post_key(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let name = parsed.get("name").and_then(Value::as_str).unwrap_or("");
    let value = match parsed.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    };
    let mut settings = state.settings.lock().await;
    match settings.set_key(name, &value) {
        Ok(name) => {
            crate::log::info("keys.set", &[("name", json!(name))]);
            send_json(200, json!({ "name": name, "keys": settings.masked_keys() }))
        }
        Err(e) => send_json(400, json!({ "error": e })),
    }
}

async fn delete_key(State(state): State<AppState>, Query(q): Query<Map<String, Value>>) -> Response {
    let name = q.get("name").and_then(Value::as_str).unwrap_or("");
    crate::log::info("keys.delete", &[("name", json!(name))]);
    let mut settings = state.settings.lock().await;
    settings.delete_key(name);
    send_json(200, json!({ "keys": settings.masked_keys() }))
}

async fn list_mcp(State(state): State<AppState>) -> Response {
    let settings = state.settings.lock().await;
    send_json(
        200,
        json!({
            "mcpServers": settings.cfg.mcp_servers,
            "projectMcpPath": settings.project_mcp_path().to_string_lossy(),
        }),
    )
}

async fn post_mcp(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let name = parsed.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let entry = parsed.get("entry").filter(|e| !e.is_null()).unwrap_or(&parsed).clone();
    let mut settings = state.settings.lock().await;
    match settings.set_mcp_server(&name, &entry) {
        Ok(name) => {
            let ty = if settings.cfg.mcp_servers.get(&name).and_then(|e| e.get("command")).is_some() {
                "stdio"
            } else {
                "http"
            };
            crate::log::info("mcp.set", &[("name", json!(name)), ("type", json!(ty))]);
            send_json(200, json!({ "name": name, "mcpServers": settings.cfg.mcp_servers }))
        }
        Err(e) => send_json(400, json!({ "error": e })),
    }
}

async fn delete_mcp(State(state): State<AppState>, Query(q): Query<Map<String, Value>>) -> Response {
    let name = q.get("name").and_then(Value::as_str).unwrap_or("");
    crate::log::info("mcp.delete", &[("name", json!(name))]);
    let mut settings = state.settings.lock().await;
    settings.delete_mcp_server(name);
    send_json(200, json!({ "mcpServers": settings.cfg.mcp_servers }))
}

/* ------------------------------------------------------------ functions */

async fn list_fns(State(state): State<AppState>) -> Response {
    send_json(200, json!({ "functions": functions::list_functions(&state.paths) }))
}

async fn post_fn(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let name = parsed.get("name").and_then(Value::as_str).unwrap_or("");
    let code = parsed.get("code").and_then(Value::as_str).unwrap_or("");
    match functions::write_function(&state.paths, name, code) {
        Ok(name) => send_json(
            200,
            json!({ "name": name, "functions": functions::list_functions(&state.paths) }),
        ),
        Err(e) => send_json(400, json!({ "error": e })),
    }
}

async fn delete_fn(State(state): State<AppState>, Query(q): Query<Map<String, Value>>) -> Response {
    let name = q.get("name").and_then(Value::as_str).unwrap_or("");
    match functions::delete_function(&state.paths, name) {
        Ok(name) => send_json(
            200,
            json!({ "name": name, "functions": functions::list_functions(&state.paths) }),
        ),
        Err(e) => send_json(404, json!({ "error": e })),
    }
}

async fn run_fn(State(state): State<AppState>, Path(name): Path<String>, req: Request) -> Response {
    static NAME_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    if !NAME_RE
        .get_or_init(|| regex::Regex::new(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?$").unwrap())
        .is_match(&name)
    {
        // the JS regex route wouldn't match — falls through to static 404
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let args = parsed.get("args").cloned().unwrap_or(json!({}));
    let compact = {
        let store = state.store.lock().await;
        core::compact_state(&store.data)
    };
    match functions::run_function(&state.paths, &name, &args, &compact).await {
        Ok(out) => {
            let mut applied: Vec<Value> = Vec::new();
            if let Some(list) = out.get("actions").and_then(Value::as_array) {
                let mut store = state.store.lock().await;
                for sa in list.iter().take(6) {
                    let mut action = Map::new();
                    action.insert("via".into(), json!(format!("function {}", name)));
                    if let Some(obj) = sa.as_object() {
                        for (k, v) in obj {
                            action.insert(k.clone(), v.clone());
                        }
                    }
                    let r = core::apply_action(&mut store.data, &Value::Object(action));
                    applied.push(json!({
                        "ok": r.get("ok") == Some(&json!(true)),
                        "message": core::strf(&r, "message").unwrap_or(""),
                    }));
                }
                if applied.iter().any(|x| x.get("ok") == Some(&json!(true))) {
                    store.save();
                }
            }
            let version = state.store.lock().await.data.version;
            send_json(200, json!({ "output": out, "applied": applied, "version": version }))
        }
        Err(e) => send_json(400, json!({ "error": e })),
    }
}

/* ------------------------------------------------------------------ jobs */

async fn list_jobs(State(state): State<AppState>) -> Response {
    send_json(200, json!({ "jobs": state.jobs.list() }))
}

async fn post_job(State(state): State<AppState>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let kind = parsed.get("kind").and_then(Value::as_str).unwrap_or("");
    let payload = match kind {
        "generate" => {
            let hint = parsed.get("hint").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if hint.is_empty() {
                return send_json(400, json!({ "error": "hint required" }));
            }
            json!({ "hint": hint })
        }
        "agent" => {
            let message = parsed.get("message").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if message.is_empty() {
                return send_json(400, json!({ "error": "message required" }));
            }
            json!({ "message": message })
        }
        _ => {
            return send_json(
                400,
                json!({ "error": format!("unknown job kind {}", serde_json::to_string(parsed.get("kind").unwrap_or(&Value::Null)).unwrap_or_default()) }),
            )
        }
    };
    match state.jobs.submit(kind, payload) {
        Ok(id) => {
            let status = state.jobs.get(&id).map(|j| j.status).unwrap_or(JobStatus::Queued);
            send_json(200, json!({ "jobId": id, "status": status }))
        }
        Err(e) => send_json(400, json!({ "error": e })),
    }
}

async fn get_job(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.jobs.detail(&id) {
        Some(d) => send_json(200, d),
        None => send_json(404, json!({ "error": format!("no job {}", id) })),
    }
}

async fn cancel_job(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.jobs.cancel(&id) {
        Some(j) => send_json(200, json!({ "job": j })),
        None => send_json(404, json!({ "error": format!("no job {}", id) })),
    }
}

/* ----------------------------------------------------------------- agent */

/// One NDJSON line into the stream channel; false when the client is gone.
async fn send_line(
    tx: &tokio::sync::mpsc::Sender<Result<String, std::convert::Infallible>>,
    obj: Value,
) -> bool {
    tx.send(Ok(format!("{}\n", serde_json::to_string(&obj).unwrap_or_default()))).await.is_ok()
}

async fn handle_agent(State(state): State<AppState>, Query(q): Query<Map<String, Value>>, req: Request) -> Response {
    let parsed = match parse_body(req).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let message = parsed
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let Some(message) = message else {
        return send_json(400, json!({ "error": "message required" }));
    };
    let read_only = parsed.get("readOnly") == Some(&json!(true));
    let stream = q.get("stream").and_then(Value::as_str) == Some("1");

    let job_id = match state.jobs.submit("agent", json!({ "message": message, "readOnly": read_only })) {
        Ok(id) => id,
        Err(e) => return send_json(400, json!({ "error": e })),
    };

    if stream {
        // subscribe synchronously — before the worker can emit progress —
        // then hand the receiver to the forwarding task
        let Some((sub, terminal)) = state.jobs.subscribe(&job_id) else {
            return send_json(400, json!({ "error": "job lost" }));
        };
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::convert::Infallible>>(32);
        let jobs = state.jobs.clone();
        let id = job_id.clone();
        tokio::spawn(async move {
            let mut sub = sub;
            // send with backpressure: a slow-but-alive client must not
            // truncate the stream — stop only on an actual disconnect
            if !send_line(&tx, json!({ "type": "queued", "jobId": id })).await {
                return;
            }
            let finish = |jobs: &crate::jobs::JobQueue, id: &str| -> Option<Value> {
                let o = jobs.outcome(id)?;
                Some(match o.status {
                    JobStatus::Done => {
                        let mut line = Map::new();
                        line.insert("type".into(), json!("result"));
                        line.insert("jobId".into(), json!(id));
                        if let Some(Value::Object(result)) = o.result {
                            line.extend(result);
                        }
                        Value::Object(line)
                    }
                    JobStatus::Cancelled => json!({ "type": "error", "jobId": id, "error": "job cancelled" }),
                    _ => json!({
                        "type": "error",
                        "jobId": id,
                        "error": o.error.unwrap_or_else(|| "job failed".into()),
                        "hint": AGENT_ERROR_HINT,
                    }),
                })
            };
            if terminal {
                if let Some(line) = finish(&jobs, &id) {
                    let _ = send_line(&tx, line).await;
                }
                return;
            }
            while let Some(ev) = sub.recv().await {
                match ev {
                    JobEvent::Progress(detail) => {
                        if !send_line(&tx, json!({ "type": "progress", "detail": detail })).await {
                            return; // client gone — the job keeps running server-side
                        }
                    }
                    JobEvent::Done => {
                        if let Some(line) = finish(&jobs, &id) {
                            let _ = send_line(&tx, line).await;
                        }
                        return;
                    }
                }
            }
        });
        let body = Body::from_stream(ReceiverStream::new(rx));
        let mut res = Response::new(body);
        res.headers_mut().insert("content-type", HeaderValue::from_static("application/x-ndjson; charset=utf-8"));
        res.headers_mut().insert("cache-control", HeaderValue::from_static("no-cache"));
        return res;
    }

    match state.jobs.wait_job(&job_id, std::time::Duration::from_millis(WAIT_JOB_TIMEOUT_MS)).await {
        Ok(outcome) if outcome.status == JobStatus::Done => {
            let mut out = Map::new();
            out.insert("jobId".into(), json!(job_id));
            if let Some(Value::Object(result)) = outcome.result {
                out.extend(result);
            }
            send_json(200, Value::Object(out))
        }
        Ok(outcome) => {
            let code = if outcome.status == JobStatus::Cancelled { 499 } else { 503 };
            let msg = outcome.error.unwrap_or_else(|| "job failed".into());
            send_json(
                code,
                json!({ "jobId": job_id, "error": format!("agent unavailable: {}", msg), "hint": AGENT_ERROR_HINT }),
            )
        }
        Err(e) => send_json(
            503,
            json!({ "jobId": job_id, "error": format!("agent unavailable: {}", e), "hint": AGENT_ERROR_HINT }),
        ),
    }
}

/* ---------------------------------------------------------------- static */

fn mime_for(ext: &str) -> &'static str {
    match ext {
        ".html" => "text/html; charset=utf-8",
        ".js" => "text/javascript; charset=utf-8",
        ".css" => "text/css; charset=utf-8",
        ".json" => "application/json; charset=utf-8",
        ".md" => "text/markdown; charset=utf-8",
        ".png" => "image/png",
        ".svg" => "image/svg+xml",
        ".ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

async fn static_or_404(State(state): State<AppState>, req: Request) -> Response {
    let raw = req.uri().path();
    let decoded = percent_decode(raw);
    let rel = if decoded == "/" { "/index.html".to_string() } else { decoded };

    // lexical normalization (resolve . and ..), then confinement to public/
    let mut components: Vec<String> = Vec::new();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            p => components.push(p.to_string()),
        }
    }
    let mut file = state.paths.public.clone();
    for c in &components {
        file.push(c);
    }
    if !file.starts_with(&state.paths.public) {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    match tokio::fs::read(&file).await {
        Ok(data) => {
            let ext = file
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let cache = if [".html", ".js", ".css", ".md", ".json"].contains(&ext.as_str()) {
                "no-cache"
            } else {
                "public, max-age=3600"
            };
            let mut res = Response::new(Body::from(data));
            res.headers_mut().insert("content-type", HeaderValue::from_static(mime_for(&ext)));
            res.headers_mut().insert("cache-control", HeaderValue::from_str(cache).unwrap());
            res
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
