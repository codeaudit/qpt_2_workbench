//! Port of the agent-bridge section of server.js — POST /api/agent.
//!
//! The agent replies with a strict JSON action plan; the server executes it
//! through the same spec-enforcing core as every other surface, so the agent
//! cannot bypass the Quality Gate, horizon, diagnosis, or genesis rules.
//!
//! Backend: `compat` only — one stateless `kimi -p --output-format
//! stream-json` turn per request (the npm-SDK `sdk` wire backend is not
//! portable; current CLIs don't ship --wire anyway).

use serde_json::{json, Map, Value};
use std::sync::{Arc, RwLock};

use crate::jobs::{JobCtx, JobError, JobQueue};
use crate::settings::Settings;
use crate::store::Store;
use crate::{core, functions, skills, Paths};

pub const TURN_TIMEOUT_MS: u64 = 150_000;

/* ------------------------------------------------------- shared state */

#[derive(Clone)]
pub struct AppState {
    pub paths: Paths,
    pub store: Arc<tokio::sync::Mutex<Store>>,
    pub settings: Arc<tokio::sync::Mutex<Settings>>,
    pub jobs: JobQueue,
    pub mock: bool,
    pub kimi_executable: String,
    pub current_model: Arc<RwLock<Option<String>>>,
}

impl AppState {
    pub fn new(paths: Paths, store: Store, settings: Settings, jobs: JobQueue) -> AppState {
        let mock = std::env::var("QPT_AGENT_MOCK").as_deref() == Ok("1");
        AppState::with_mock(paths, store, settings, jobs, mock)
    }

    pub fn with_mock(paths: Paths, store: Store, settings: Settings, jobs: JobQueue, mock: bool) -> AppState {
        let app = AppState {
            paths,
            store: Arc::new(tokio::sync::Mutex::new(store)),
            settings: Arc::new(tokio::sync::Mutex::new(settings)),
            jobs,
            mock,
            kimi_executable: std::env::var("KIMI_EXECUTABLE")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "kimi".into()),
            current_model: Arc::new(RwLock::new(
                std::env::var("KIMI_MODEL").ok().filter(|s| !s.is_empty()),
            )),
        };
        register_handlers(&app);
        app
    }

    pub fn model(&self) -> Option<String> {
        self.current_model.read().ok().and_then(|m| m.clone())
    }

    /// Probe the CLI (logging only — the compat backend needs no detection).
    pub async fn probe_kimi(&self) {
        let exe = self.kimi_executable.clone();
        let probe = tokio::process::Command::new(&exe)
            .arg("--help")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        match tokio::time::timeout(std::time::Duration::from_millis(8000), probe).await {
            Ok(Ok(out)) if out.status.success() => {
                let help = String::from_utf8_lossy(&out.stdout);
                let backend = if help.contains("--wire") { "sdk" } else { "compat" };
                crate::log::info(
                    "agent.backend",
                    &[("backend", json!(backend)), ("executable", json!(exe))],
                );
            }
            e => {
                let reason = match e {
                    Ok(Ok(out)) => format!("exited {:?}", out.status.code()),
                    Ok(Err(err)) => err.to_string(),
                    Err(_) => "probe timed out".into(),
                };
                crate::log::warn(
                    "agent.backend.fallback",
                    &[("backend", json!("compat")), ("reason", json!(reason.chars().take(120).collect::<String>()))],
                );
            }
        }
    }
}

/* ------------------------------------------------------- agent protocol */

pub const INSTRUCTIONS: &str = r#"You are the operator agent for QPT Workbench, an interactive kanban that RUNS the Quaternion Process Theory 2.7 specification. The user sends you natural-language requests plus the current board state as JSON. You decide which board actions to take.

BOARDS AND COLUMNS
- protocol: initiation → encounter → gate → articulation → closure (the Generative Protocol, §14)
- dialectic: explore → integrate → consolidate → synthesize (the Scheduled Dialectic, §19)
- resolution: phase0 → phase1 → phase2 → phase3 → phase4 → phase5 (the Resolution Procedure, §29)

THE SEMANTICS THE UI WILL ENFORCE (your actions are refused if they violate these)
- A13 horizon = 1: cards move forward ONE column at a time. Backward jumps are free.
- A9 dual gate: a protocol card may advance gate → articulation only when its verdict is Living ((○)-recruited source, [□]-grounded target, and S = rho − delta − gamma·ln(k) ≥ theta). Otherwise edit the card's metrics/trajectory first.
- §14: a card failing the gate must have its pathology set (structural|attentional|content|scalar|temporal) before it may leave the gate column backward.
- A7: entering synthesize requires card field genesis (an emergent property present in neither parent).
- Promotion paths: protocol closure → dialectic explore; dialectic synthesize → protocol initiation; a protocol card with a death mode (fossil|residue|imposition) → resolution phase0; resolution phase5 → protocol initiation.

ACTION VOCABULARY (reply with ONLY a JSON object, no prose outside it)
{"reply": "short explanation of what you did and why, with spec refs",
 "actions": [
  {"action": "set_board", "board": "protocol|dialectic|resolution"},
  {"action": "create_card", "board": "...", "title": "...", "source": "recruited|initiated", "target": "grounded|terminated", "kind": "position|note", "reliability": 0.7, "tags": ["..."], "note": "..."},
  {"action": "move_card", "id": "...", "column": "..."},
  {"action": "edit_card", "id": "...", "patch": {"title"|"note"|"rho"|"delta"|"gamma"|"k"|"theta"|"source"|"target"|"scale"|"pathology"|"death"|"reliability"|"genesis"|"tags": "..."}},
  {"action": "promote_card", "id": "...", "board": "dialectic|protocol|resolution"},
  {"action": "evaluate_card", "id": "..."},
  {"action": "run_function", "name": "...", "args": {...}}
 ]}
Use only the fields each action needs. create_card: source/target only matter on protocol; kind/reliability only on dialectic; tags only on resolution. Card ids in the state are current — use them verbatim. Cards also carry a stable CamelCase HANDLE (e.g. OnboardingDropoff): when the user writes @Handle, that is the card they mean — address it by its id in actions. At most 12 actions; order them so each is legal when it runs (remember horizon = 1: multi-column journeys need one move per column).

EXECUTION: your actions run on the canonical server-side store via the shared domain core — the same code the UI uses. Anything illegal comes back refused in the results.

USER FUNCTIONS: state.functions lists user-generated functions (name, description). Call them with run_function — they compute and may return further actions, which execute through the same core. Prefer a function over hand-rolling a procedure when one matches.

MCP: if the CLI session exposes MCP tools (state.mcpServers lists configured servers), you may use them for reading/investigating; your REQUIRED output channel remains the JSON plan above.

SKILLS: state.skills lists user-authored instruction sets. When a skill's description matches the request, follow its content — skills are the user's way of teaching you their house rules. Users can also drive the store directly through a text CLI (help · state · cards · create · move · edit · evaluate · promote · skills …) exposed at POST /api/cli; you do not need it — your channel is the action vocabulary above.

You may read files in your working directory for exact semantics: QPT_2x_Consolidated_Specification_r2.7.md (the full spec) and qpt-ui/qpt-data.js (seeded content)."#;

const BOARDS: [&str; 3] = ["protocol", "dialectic", "resolution"];

/* ------------------------------------------------------------ models */

fn toml_val(s: &str) -> Value {
    let s = s.trim();
    if s.starts_with('"') {
        let end = s.rfind('"').filter(|&i| i > 0).map(|i| i as i64).unwrap_or(0);
        let inner = if end > 0 { &s[1..end as usize] } else { "" };
        return json!(inner);
    }
    if s == "true" {
        return json!(true);
    }
    if s == "false" {
        return json!(false);
    }
    if s.starts_with('[') {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| regex::Regex::new(r#""([^"]*)""#).unwrap());
        let arr: Vec<Value> = re.captures_iter(s).map(|m| json!(m[1])).collect();
        return Value::Array(arr);
    }
    match s.parse::<f64>() {
        Ok(n) => json!(n),
        Err(_) => json!(s),
    }
}

pub fn parse_models_toml(text: &str) -> (Vec<Value>, Option<String>) {
    static SEC_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static KV_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let mut models: Vec<Map<String, Value>> = Vec::new();
    let mut default_model: Option<String> = None;
    let mut cur: Option<usize> = None;
    let sec_re = SEC_RE.get_or_init(|| regex::Regex::new(r#"^\[models\."([^"]+)"\]\s*$"#).unwrap());
    let kv_re = KV_RE.get_or_init(|| regex::Regex::new(r"^([A-Za-z_]+)\s*=\s*(.+)$").unwrap());
    for raw in text.split('\n') {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(sec) = sec_re.captures(line) {
            let mut m = Map::new();
            m.insert("id".into(), json!(sec[1]));
            models.push(m);
            cur = Some(models.len() - 1);
            continue;
        }
        if line.starts_with('[') {
            cur = None;
            continue;
        }
        let Some(kv) = kv_re.captures(line) else { continue };
        if let Some(i) = cur {
            models[i].insert(kv[1].to_string(), toml_val(&kv[2]));
        } else if &kv[1] == "default_model" {
            default_model = toml_val(&kv[2]).as_str().map(str::to_string);
        }
    }
    let models: Vec<Value> = models
        .into_iter()
        .map(|m| {
            let mut out = Map::new();
            let id = m.get("id").cloned().unwrap_or(Value::Null);
            out.insert("id".into(), id.clone());
            out.insert(
                "name".into(),
                m.get("display_name").cloned().unwrap_or(id),
            );
            for (src, dst) in [
                ("provider", "provider"),
                ("model", "model"),
                ("max_context_size", "maxContextSize"),
                ("default_effort", "defaultEffort"),
            ] {
                if let Some(v) = m.get(src) {
                    out.insert(dst.into(), v.clone());
                }
            }
            out.insert(
                "capabilities".into(),
                m.get("capabilities").cloned().unwrap_or(json!([])),
            );
            Value::Object(out)
        })
        .collect();
    (models, default_model)
}

/// config.toml → (models, default_model), re-read and re-parsed only when
/// the file's (mtime, len) changes. None when the file is unreadable.
fn cli_models_cached() -> Option<(Vec<Value>, Option<String>)> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(crate::FileStamp, Vec<Value>, Option<String>)>>> =
        std::sync::OnceLock::new();
    let path = crate::kimi_code_home().join("config.toml");
    let stamp = crate::file_stamp(&path)?;
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = cache.lock().ok()?;
    if let Some((s, models, default_model)) = guard.as_ref() {
        if *s == stamp {
            return Some((models.clone(), default_model.clone()));
        }
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let (models, default_model) = parse_models_toml(&text);
    *guard = Some((stamp, models.clone(), default_model.clone()));
    Some((models, default_model))
}

/// `{models, defaultModel, current, source}` for GET /api/models.
pub async fn list_models(app: &AppState) -> Value {
    let (models, default_model) = match cli_models_cached() {
        Some((m, d)) => {
            if m.is_empty() {
                (m, d)
            } else {
                let current = app.model().or_else(|| d.clone());
                return json!({
                    "models": m,
                    "defaultModel": d,
                    "current": current,
                    "source": "config.toml",
                });
            }
        }
        None => (Vec::new(), None),
    };
    let current = app.model().or_else(|| default_model.clone());
    json!({
        "models": models,
        "defaultModel": default_model,
        "current": current,
        "source": "none",
    })
}

/* --------------------------------------------------------- prompt build */

fn build_prompt(
    message: &str,
    state: &Value,
    include_instructions: bool,
    instructions: Option<&str>,
    system_prompt: Option<&str>,
) -> String {
    let mut out = String::new();
    if include_instructions {
        out.push_str(instructions.or(system_prompt).unwrap_or(INSTRUCTIONS));
        out.push_str("\n\n");
    }
    out.push_str("CURRENT STATE (JSON):\n");
    out.push_str(&serde_json::to_string(state).unwrap_or_default());
    out.push_str("\n\nUSER REQUEST:\n");
    out.push_str(message);
    out.push_str("\n\nRespond with ONLY the JSON object described.");
    out
}

/* ------------------------------------------------- compat CLI spawn */

/// One stateless prompt per request via the CLI's stream-json mode.
/// Synchronous (run inside spawn_blocking): stdout line parsing, heartbeat,
/// timeout and cancellation via SIGKILL.
fn run_agent_compat_sync(
    app: &AppState,
    prompt: String,
    env_extra: Vec<(String, String)>,
    on_progress: impl Fn(&str) + Send + Sync + 'static,
    cancel_reg: Option<impl Fn(crate::jobs::CancelFn) + Send>,
) -> Result<String, String> {
    let mut cmd = std::process::Command::new(&app.kimi_executable);
    cmd.arg("-p")
        .arg(&prompt)
        .arg("--output-format")
        .arg("stream-json")
        .current_dir(&app.paths.parent)
        .envs(env_extra)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(model) = app.model() {
        cmd.arg("--model").arg(&model);
    }
    crate::log::debug(
        "agent.spawn",
        &[
            ("backend", json!("compat")),
            ("model", json!(app.model().unwrap_or_else(|| "default".into()))),
            ("promptBytes", json!(prompt.len())),
        ],
    );
    let mut child = cmd.spawn().map_err(|e| format!("spawn {}: {}", app.kimi_executable, e))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let child = Arc::new(std::sync::Mutex::new(child));

    if let Some(reg) = cancel_reg {
        let c = child.clone();
        reg(Arc::new(move || {
            if let Ok(mut c) = c.lock() {
                let _ = c.kill();
            }
        }));
    }

    let t0 = std::time::Instant::now();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let on_progress = Arc::new(on_progress);

    // stderr drain (prevents pipe blockage)
    let stderr_handle = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = std::io::BufReader::new(stderr).read_to_string(&mut s);
        s
    });

    // heartbeat + timeout watcher: wake every 100ms so a finished turn joins
    // promptly, while keeping one heartbeat per 2.5s of wall time
    let hb = {
        let done = done.clone();
        let timed_out = timed_out.clone();
        let child = child.clone();
        let on_progress = on_progress.clone();
        std::thread::spawn(move || loop {
            for _ in 0..25 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if done.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
            }
            let secs = (t0.elapsed().as_millis() as f64 / 1000.0).round() as u64;
            on_progress(&format!("reasoning… {}s", secs));
            if t0.elapsed().as_millis() as u64 > TURN_TIMEOUT_MS {
                timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Ok(mut c) = child.lock() {
                    let _ = c.kill();
                }
                break;
            }
        })
    };

    // stdout NDJSON parse
    let mut parts: Vec<String> = Vec::new();
    {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let on_ev = |line: &str, parts: &mut Vec<String>| {
            let line = line.trim();
            if line.is_empty() {
                return;
            }
            let Ok(ev) = serde_json::from_str::<Value>(line) else { return };
            if ev.get("role").and_then(Value::as_str) == Some("assistant") {
                let content = ev.get("content").cloned().unwrap_or(Value::Null);
                let texts: Vec<String> = match &content {
                    Value::String(s) => vec![s.clone()],
                    Value::Array(arr) => arr
                        .iter()
                        .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|p| p.get("text").and_then(Value::as_str).map(str::to_string))
                        .collect(),
                    _ => Vec::new(),
                };
                if !texts.is_empty() {
                    if parts.is_empty() {
                        on_progress("drafting the plan…");
                    }
                    parts.push(texts.join(""));
                }
                if let Value::Array(arr) = content {
                    for p in arr {
                        if p.get("type").and_then(Value::as_str) == Some("tool_use") {
                            let name = p.get("name").and_then(Value::as_str).unwrap_or("unknown");
                            on_progress(&format!("calling tool: {}", name));
                        }
                    }
                }
            } else if ev.get("role").and_then(Value::as_str) != Some("meta") {
                if let Some(t) = ev.get("type").and_then(Value::as_str) {
                    on_progress(&format!("event: {}", t));
                }
            }
        };
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => on_ev(&line, &mut parts),
                Err(_) => break,
            }
        }
    }

    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = hb.join();
    let status = child.lock().ok().and_then(|mut c| c.wait().ok());
    let err_buf = stderr_handle.join().unwrap_or_default();
    let code = status.and_then(|s| s.code());
    crate::log::debug(
        "agent.spawn.exit",
        &[
            ("code", json!(code.map(|c| c as i64))),
            ("ms", json!(t0.elapsed().as_millis() as u64)),
            ("replyBytes", json!(parts.join("\n").len())),
        ],
    );
    if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("agent turn timed out".into());
    }
    if code != Some(0) {
        crate::log::warn(
            "agent.spawn.failed",
            &[("code", json!(code)), ("stderr", json!(err_buf.chars().take(200).collect::<String>()))],
        );
        let suffix = if err_buf.is_empty() {
            String::new()
        } else {
            format!(": {}", err_buf.chars().take(300).collect::<String>())
        };
        return Err(format!(
            "CLI exited with code {}{}",
            code.map(|c| c.to_string()).unwrap_or_else(|| "null".into()),
            suffix
        ));
    }
    Ok(parts.join("\n"))
}

async fn run_agent(
    app: &AppState,
    message: &str,
    state: &Value,
    instructions: Option<String>,
    ctx: &JobCtx,
) -> Result<String, JobError> {
    let system_prompt = app.settings.lock().await.cfg.system_prompt.clone();
    let prompt = build_prompt(message, state, true, instructions.as_deref(), system_prompt.as_deref());
    let env_extra = app.settings.lock().await.env_for_agent();
    let app2 = app.clone();
    let ctx2 = ctx.clone();
    let ctx3 = ctx.clone();
    let on_progress = move |d: &str| ctx2.note(d);
    let cancel_reg = move |f: crate::jobs::CancelFn| ctx3.set_cancel(f);
    tokio::task::spawn_blocking(move || run_agent_compat_sync(&app2, prompt, env_extra, on_progress, Some(cancel_reg)))
        .await
        .map_err(|e| JobError::msg(format!("agent task: {}", e)))?
        .map_err(JobError::msg)
}

/* ------------------------------------------------------- plan handling */

fn extract_json(text: &str) -> Option<Value> {
    static FENCE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let fence = FENCE_RE.get_or_init(|| regex::Regex::new(r"(?s)```(?:json)?\s*(.*?)```").unwrap());
    let candidate = fence.captures(text).map(|m| m[1].to_string()).unwrap_or_else(|| text.to_string());
    let start = candidate.find('{')?;
    let end = candidate.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&candidate[start..=end]).ok()
}

fn validate_plan(plan: &Value) -> Value {
    static FN_NAME_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut actions: Vec<Value> = Vec::new();
    let fn_name_re = FN_NAME_RE.get_or_init(|| regex::Regex::new(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?$").unwrap());
    let list: Vec<Value> = plan
        .get("actions")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(12).cloned().collect())
        .unwrap_or_default();
    for a in list {
        let Some(action) = a.get("action").and_then(Value::as_str) else {
            warnings.push(json!("dropped malformed action"));
            continue;
        };
        match action {
            "set_board" => {
                let board = a.get("board").and_then(Value::as_str).unwrap_or("");
                if BOARDS.contains(&board) {
                    actions.push(json!({ "action": "set_board", "board": board }));
                } else {
                    warnings.push(json!(format!("set_board: unknown board {}", serde_json::to_string(a.get("board").unwrap_or(&Value::Null)).unwrap_or_default())));
                }
            }
            "create_card" => {
                let board = a.get("board").and_then(Value::as_str).unwrap_or("");
                let title = a.get("title").and_then(Value::as_str);
                if !BOARDS.contains(&board) || title.map(|t| t.trim().is_empty()).unwrap_or(true) {
                    warnings.push(json!("create_card: needs board + title"));
                    continue;
                }
                let title: String = title.unwrap().chars().take(200).collect();
                let mut c = Map::new();
                c.insert("action".into(), json!("create_card"));
                c.insert("board".into(), json!(board));
                c.insert("title".into(), json!(title));
                if let Some(s) = a.get("source").and_then(Value::as_str) {
                    if ["recruited", "initiated"].contains(&s) {
                        c.insert("source".into(), json!(s));
                    }
                }
                if let Some(t) = a.get("target").and_then(Value::as_str) {
                    if ["grounded", "terminated"].contains(&t) {
                        c.insert("target".into(), json!(t));
                    }
                }
                if let Some(k) = a.get("kind").and_then(Value::as_str) {
                    if ["position", "note"].contains(&k) {
                        c.insert("kind".into(), json!(k));
                    }
                }
                if let Some(r) = a.get("reliability").and_then(Value::as_f64) {
                    c.insert("reliability".into(), json!(r.clamp(0.0, 1.0)));
                }
                if let Some(tags) = a.get("tags").and_then(Value::as_array) {
                    let tags: Vec<Value> = tags.iter().filter(|t| t.is_string()).take(8).cloned().collect();
                    c.insert("tags".into(), Value::Array(tags));
                }
                if let Some(note) = a.get("note").and_then(Value::as_str) {
                    let note: String = note.chars().take(2000).collect();
                    c.insert("note".into(), json!(note));
                }
                actions.push(Value::Object(c));
            }
            "move_card" => {
                match (a.get("id").and_then(Value::as_str), a.get("column").and_then(Value::as_str)) {
                    (Some(id), Some(col)) => actions.push(json!({ "action": "move_card", "id": id, "column": col })),
                    _ => warnings.push(json!("move_card: needs id + column")),
                }
            }
            "edit_card" => {
                let id = a.get("id").and_then(Value::as_str);
                let patch = a.get("patch").filter(|p| p.is_object());
                match (id, patch) {
                    (Some(id), Some(p)) => actions.push(json!({ "action": "edit_card", "id": id, "patch": p })),
                    _ => warnings.push(json!("edit_card: needs id + patch")),
                }
            }
            "promote_card" => {
                let id = a.get("id").and_then(Value::as_str);
                let board = a.get("board").and_then(Value::as_str);
                match (id, board) {
                    (Some(id), Some(b)) if BOARDS.contains(&b) => {
                        actions.push(json!({ "action": "promote_card", "id": id, "board": b }))
                    }
                    _ => warnings.push(json!("promote_card: needs id + valid board")),
                }
            }
            "evaluate_card" => match a.get("id").and_then(Value::as_str) {
                Some(id) => actions.push(json!({ "action": "evaluate_card", "id": id })),
                None => warnings.push(json!("evaluate_card: needs id")),
            },
            "run_function" => {
                let name = a.get("name").and_then(Value::as_str);
                let valid = name.map(|n| fn_name_re.is_match(n)).unwrap_or(false);
                if valid {
                    let args = a.get("args").filter(|x| x.is_object()).cloned().unwrap_or(json!({}));
                    actions.push(json!({ "action": "run_function", "name": name.unwrap(), "args": args }));
                } else {
                    warnings.push(json!("run_function: needs a valid function name"));
                }
            }
            other => warnings.push(json!(format!("unknown action {}", serde_json::to_string(&json!(other)).unwrap_or_default()))),
        }
    }
    let reply = plan
        .get("reply")
        .and_then(Value::as_str)
        .unwrap_or("Done.")
        .to_string();
    json!({ "reply": reply, "actions": actions, "warnings": warnings })
}

/* ------------------------------------------- reference expansion (@ / /)
 *
 * Chat payloads carry plain text; tokens like @p1 or /grounding-protocol are
 * resolved HERE, server-side, so the model receives the actual context:
 * full card records for @references, full SKILL.md content for /invocations.
 */

struct Refs {
    cards: Vec<Value>,
    skills: Vec<Value>,
}

async fn expand_references(app: &AppState, message: &str) -> Refs {
    static AT_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static SLASH_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let mut refs = Refs { cards: Vec::new(), skills: Vec::new() };
    if message.is_empty() {
        return refs;
    }
    let mut used_cards: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut used_skills: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Cheap pre-filter: both token kinds need their literal marker char, so a
    // message without it expands to nothing — skip the store lock / disk scan.
    if message.contains('@') {
        let store = app.store.lock().await;
        let re = AT_RE.get_or_init(|| regex::Regex::new(r"@([\w-]+)").unwrap());
        for m in re.captures_iter(message) {
            if let Some(hit) = core::find_card(&store.data.cards, &m[1]) {
                let id = core::strf(hit, "id").unwrap_or("").to_string();
                if !used_cards.contains(&id) {
                    used_cards.insert(id);
                    refs.cards.push(hit.clone());
                }
            }
        }
    }

    if message.contains('/') {
        let re = SLASH_RE.get_or_init(|| regex::Regex::new(r"(?:^|\s)/([a-z0-9][a-z0-9-]*)").unwrap());
        let (scopes, _) = skills::list_all_skills(&app.paths);
        let flat: Vec<(&skills::SkillRecord, &str)> = scopes
            .iter()
            .flat_map(|sc| sc.skills.iter().map(move |s| (s, sc.scope.as_str())))
            .collect();
        for m in re.captures_iter(message) {
            let name = &m[1];
            if let Some((hit, scope)) = flat.iter().find(|(s, _)| s.id == name || s.name == name) {
                if !used_skills.contains(&hit.id) {
                    used_skills.insert(hit.id.clone());
                    refs.skills.push(json!({
                        "id": hit.id,
                        "name": hit.name,
                        "scope": scope,
                        "content": hit.content,
                    }));
                }
            }
        }
    }
    refs
}

fn references_block(refs: &Refs) -> String {
    let mut out = String::new();
    if !refs.cards.is_empty() {
        let enriched: Vec<Value> = refs
            .cards
            .iter()
            .map(|c| {
                let mut c = c.clone();
                let ev = core::evaluate(&c);
                c["computedVerdict"] = ev.map(|e| json!(e.key)).unwrap_or(Value::Null);
                if !core::is_null(&c, "rho") {
                    if let Some(s) = core::score_of(&c) {
                        let rounded: f64 = format!("{:.3}", s).parse().unwrap_or(s);
                        c["computedS"] = json!(rounded);
                    }
                }
                c
            })
            .collect();
        out.push_str("\n\nREFERENCED CARDS (full records — the user is pointing at these):\n");
        out.push_str(&crate::json_indent1(&enriched));
    }
    if !refs.skills.is_empty() {
        out.push_str("\n\nINVOKED SKILLS — the user invoked these explicitly; their instructions are binding for this turn:\n");
        let parts: Vec<String> = refs
            .skills
            .iter()
            .map(|s| {
                format!(
                    "--- {} ({}) ---\n{}",
                    core::strf(s, "id").unwrap_or(""),
                    core::strf(s, "scope").unwrap_or(""),
                    core::strf(s, "content").unwrap_or("")
                )
            })
            .collect();
        out.push_str(&parts.join("\n\n"));
    }
    out
}

async fn agent_state(app: &AppState) -> Value {
    // Skills/functions live on disk: scan them off the async worker and
    // BEFORE taking the store lock (sync dir scans + file reads).
    let paths = app.paths.clone();
    let scanned = tokio::task::spawn_blocking(move || {
        (skills::list_skills(&paths).0, functions::list_functions(&paths))
    })
    .await;
    let (skills_list, functions_list) = scanned.unwrap_or_else(|_| (Vec::new(), Vec::new()));
    let skills_json: Vec<Value> = skills_list
        .iter()
        .map(|sk| {
            json!({
                "id": sk.id,
                "name": sk.name,
                "description": sk.description,
                "content": sk.content,
            })
        })
        .collect();
    let store = app.store.lock().await;
    let mut s = core::compact_state(&store.data);
    drop(store);
    s["skills"] = Value::Array(skills_json);
    s["functions"] = json!(functions_list);
    let mcp_names: Vec<String> = app.settings.lock().await.cfg.mcp_servers.keys().cloned().collect();
    s["mcpServers"] = json!(mcp_names);
    s
}

/* --------------------------------------------------------- plan execution */

/// Execute the validated plan on the canonical store via the shared core.
/// dry_run: explain/read-only invocations — actions are reported as
/// suggestions, never applied.
pub async fn execute_plan(app: &AppState, validated: Value, dry_run: bool) -> Value {
    let actions = validated.get("actions").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut results: Vec<Value> = Vec::new();
    let mut changed = false;

    if dry_run {
        for a in &actions {
            results.push(json!({
                "action": a.get("action").cloned().unwrap_or(Value::Null),
                "ok": true,
                "message": "suggested — not executed (read-only)",
            }));
        }
        let version = app.store.lock().await.data.version;
        let mut out = validated.as_object().cloned().unwrap_or_default();
        out.insert("results".into(), Value::Array(results));
        out.insert("version".into(), json!(version));
        out.insert("dryRun".into(), json!(true));
        return Value::Object(out);
    }

    // Process the plan in order, in locked segments: store actions apply
    // under the lock; a run_function only snapshots state under the lock,
    // then its node subprocess runs WITHOUT the lock held, and the actions
    // it returns are applied under a fresh lock. Plan order is preserved.
    let mut i = 0;
    while i < actions.len() {
        let mut pending: Option<(String, Value, Value)> = None;
        {
            let mut store = app.store.lock().await;
            while i < actions.len() {
                let a = &actions[i];
                if a.get("action").and_then(Value::as_str) == Some("run_function") {
                    let name = core::strf(a, "name").unwrap_or("").to_string();
                    let args = a.get("args").cloned().unwrap_or(json!({}));
                    pending = Some((name, args, core::compact_state(&store.data)));
                    i += 1;
                    break;
                }
                let mut action = serde_json::Map::new();
                action.insert("via".into(), json!("the Kimi agent"));
                if let Some(obj) = a.as_object() {
                    for (k, v) in obj {
                        action.insert(k.clone(), v.clone());
                    }
                }
                let r = core::apply_action(&mut store.data, &Value::Object(action));
                let ok = r.get("ok") == Some(&json!(true));
                let message = core::strf(&r, "message").unwrap_or("").to_string();
                if !ok {
                    crate::log::warn(
                        "action.refused",
                        &[
                            ("action", a.get("action").cloned().unwrap_or(Value::Null)),
                            ("id", a.get("id").cloned().unwrap_or(Value::Null)),
                            ("reason", json!(message.chars().take(160).collect::<String>())),
                        ],
                    );
                } else {
                    crate::log::debug(
                        "action.ok",
                        &[("action", a.get("action").cloned().unwrap_or(Value::Null)), ("id", a.get("id").cloned().unwrap_or(Value::Null))],
                    );
                }
                results.push(json!({
                    "action": a.get("action").cloned().unwrap_or(Value::Null),
                    "ok": ok,
                    "message": message,
                }));
                if ok {
                    changed = true;
                }
                i += 1;
            }
        }
        let Some((name, args, state)) = pending else { break };
        let outcome = functions::run_function(&app.paths, &name, &args, &state).await;
        let mut store = app.store.lock().await;
        match outcome {
            Ok(out) => {
                let mut sub: Vec<String> = Vec::new();
                if let Some(list) = out.get("actions").and_then(Value::as_array) {
                    for sa in list.iter().take(6) {
                        let mut action = serde_json::Map::new();
                        action.insert("via".into(), json!(format!("function {}", name)));
                        if let Some(obj) = sa.as_object() {
                            for (k, v) in obj {
                                action.insert(k.clone(), v.clone());
                            }
                        }
                        let r = core::apply_action(&mut store.data, &Value::Object(action));
                        sub.push(core::strf(&r, "message").unwrap_or("").to_string());
                        if r.get("ok") == Some(&json!(true)) {
                            changed = true;
                        }
                    }
                }
                crate::log::info("function.run", &[("name", json!(name)), ("ok", json!(true))]);
                let base = out
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{} done", name));
                let msg = if sub.is_empty() {
                    base
                } else {
                    format!("{} [{}]", base, sub.join("; "))
                };
                results.push(json!({ "action": "run_function", "ok": true, "message": msg }));
            }
            Err(e) => {
                crate::log::warn(
                    "function.run.failed",
                    &[("name", json!(name)), ("error", json!(e.chars().take(200).collect::<String>()))],
                );
                results.push(json!({ "action": "run_function", "ok": false, "message": e }));
            }
        }
    }
    if changed {
        app.store.lock().await.save();
    }
    let version = app.store.lock().await.data.version;
    let mut out = validated.as_object().cloned().unwrap_or_default();
    out.insert("results".into(), Value::Array(results));
    out.insert("version".into(), json!(version));
    Value::Object(out)
}

/* ------------------------------------------------------------- produce */

pub async fn produce(app: &AppState, message: &str, ctx: &JobCtx, dry_run: bool) -> Result<Value, JobError> {
    let refs = expand_references(app, message).await;
    let refs_out = json!({
        "skills": refs.skills.iter().map(|s| s.get("id").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "cards": refs.cards.iter().map(|c| c.get("id").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
    });

    if app.mock {
        ctx.note("mock backend: generating canned plan…");
        let delay: u64 = std::env::var("QPT_MOCK_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        let mut plan = json!({
            "reply": "[mock] Pipeline check: I created a living transformation card on the protocol board. Run without QPT_AGENT_MOCK for real reasoning.",
            "actions": [{
                "action": "create_card",
                "board": "protocol",
                "title": "Mock agent card — the bridge works",
                "source": "recruited",
                "target": "grounded",
                "note": "Created by the mock agent path (QPT_AGENT_MOCK=1)."
            }],
            "warnings": [],
        });
        plan["refs"] = refs_out;
        return Ok(execute_plan(app, plan, dry_run).await);
    }

    let t0 = std::time::Instant::now();
    let full_message = format!("{}{}", message, references_block(&refs));
    let state = agent_state(app).await;
    let text = run_agent(app, &full_message, &state, None, ctx).await?;
    let plan = extract_json(&text);
    let mut validated = match &plan {
        Some(p) => validate_plan(p),
        None => json!({
            "reply": format!("The agent did not return a parseable plan. Raw reply: {}", text.chars().take(1500).collect::<String>()),
            "actions": [],
            "warnings": ["unparseable plan"],
        }),
    };
    validated["refs"] = refs_out.clone();
    let refs_count = refs_out
        .get("skills")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0)
        + refs_out.get("cards").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
    crate::log::info(
        "agent.turn",
        &[
            ("ms", json!(t0.elapsed().as_millis() as u64)),
            ("msgBytes", json!(message.len())),
            ("actions", json!(validated.get("actions").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0))),
            ("refs", json!(if refs_count > 0 { Some(refs_count) } else { None })),
            ("parsed", json!(plan.is_some())),
        ],
    );
    if plan.is_none() {
        crate::log::warn(
            "agent.plan.unparseable",
            &[("tail", json!(text.chars().rev().take(200).collect::<String>().chars().rev().collect::<String>()))],
        );
    }
    Ok(execute_plan(app, validated, dry_run).await)
}

/* ---------------------------------------------------------- job handlers */

async fn generate(app: &AppState, ctx: JobCtx) -> Result<Value, JobError> {
    let payload = ctx.payload();
    let hint = payload.get("hint").and_then(Value::as_str).unwrap_or("").to_string();

    if app.mock {
        ctx.note("mock backend: generating canned draft…");
        let cleaned: String = hint
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c.is_whitespace() { c } else { ' ' })
            .collect();
        let words: Vec<&str> = cleaned.split_whitespace().take(3).collect();
        let name = {
            let s = skills::slugify(&words.join("-"));
            if s.is_empty() { "generated-skill".to_string() } else { s }
        };
        return Ok(json!({
            "draft": {
                "name": name,
                "description": format!("[mock] Skill for: {}. Use when the request involves it.", hint.chars().take(120).collect::<String>()),
                "content": format!("# {}\n\n1. Understand the request.\n2. Apply the procedure.\n3. Report the result.\n\n[mock draft — run without QPT_AGENT_MOCK for a real generation]", hint.chars().take(80).collect::<String>()),
            }
        }));
    }

    let generator = skills::read_skill(&app.paths, skills::GENERATOR_SKILL_ID).map_err(JobError::msg)?;
    let (scopes, _) = skills::list_all_skills(&app.paths);
    let taken: Vec<String> = scopes
        .iter()
        .flat_map(|sc| sc.skills.iter().map(|s| s.name.to_lowercase()))
        .collect();
    let message = format!(
        "HINT for the new skill: {}\n\nTAKEN skill names (do not reuse): {}\n\nWrite the skill now, per your output contract.",
        hint,
        if taken.is_empty() { "(none)".to_string() } else { taken.join(", ") }
    );
    let text = run_agent(app, &message, &json!({}), Some(generator.content), &ctx).await?;
    let plan = extract_json(&text);
    let draft = plan
        .as_ref()
        .and_then(|p| p.get("draft").filter(|d| d.is_object()).cloned())
        .or(plan.clone());
    let valid = draft
        .as_ref()
        .map(|d| {
            d.get("name").and_then(Value::as_str).is_some()
                && d.get("description").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false)
                && d.get("content").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false)
        })
        .unwrap_or(false);
    let Some(draft) = draft.filter(|_| valid) else {
        let mut e = JobError::msg("the generator returned no usable draft");
        e.raw = Some(text.chars().take(1200).collect());
        return Err(e);
    };
    let name = skills::slugify(core::strf(&draft, "name").unwrap_or(""));
    if taken.contains(&name) {
        let mut e = JobError::msg(format!("generated name collides with an existing skill: {}", name));
        e.status = Some(409);
        let mut d = draft.as_object().cloned().unwrap_or_default();
        d.insert("name".into(), json!(name));
        e.draft = Some(Value::Object(d));
        return Err(e);
    }
    Ok(json!({
        "draft": {
            "name": name,
            "description": core::strf(&draft, "description").unwrap_or(""),
            "content": core::strf(&draft, "content").unwrap_or(""),
        }
    }))
}

pub fn register_handlers(app: &AppState) {
    let a = app.clone();
    app.jobs.register_handler("agent", move |ctx: JobCtx| {
        let app = a.clone();
        async move {
            let payload = ctx.payload();
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let dry_run = payload.get("readOnly") == Some(&json!(true));
            produce(&app, &message, &ctx, dry_run).await
        }
    });
    let a = app.clone();
    app.jobs.register_handler("generate", move |ctx: JobCtx| {
        let app = a.clone();
        async move { generate(&app, ctx).await }
    });
}
