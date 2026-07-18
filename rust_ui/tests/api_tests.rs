//! HTTP API tests: boots the real axum router against a scratch data dir
//! (seeded fresh) on an ephemeral port, in mock agent mode.

use qpt::agent::AppState;
use qpt::{jobs, settings, skills, store::Store, Paths};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

/// A scratch app root: <tmp>/qpt-api-test-<pid>-<n>/app with data/ + public/.
/// The parent dir isolates the MCP sync target (.kimi-code/mcp.json).
fn scratch_root() -> (std::path::PathBuf, Paths) {
    let n = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("qpt-api-test-{}-{}", std::process::id(), n));
    let root = base.join("app");
    let public = root.join("public");
    std::fs::create_dir_all(&public).unwrap();
    std::fs::write(public.join("index.html"), "<h1>qpt</h1>").unwrap();
    let paths = Paths::from_root(root);
    std::fs::create_dir_all(paths.data.clone()).unwrap();
    (base, paths)
}

struct TestServer {
    base_url: String,
    _base: std::path::PathBuf, // keep alive for the test's duration
}

async fn boot(mock: bool) -> TestServer {
    let (base, paths) = scratch_root();
    let store = Store::open(&paths);
    skills::ensure_skill_generator(&paths);
    let settings = settings::load_settings(&paths);
    let jobs = jobs::JobQueue::new();
    let app = AppState::with_mock(paths, store, settings, jobs, mock);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, qpt::http::router(app)).await.unwrap();
    });
    TestServer { base_url: format!("http://127.0.0.1:{}", port), _base: base }
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn api_flow() {
    let srv = boot(true).await;
    let c = client();

    // health
    let r: Value = c.get(format!("{}/api/health", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["ok"], json!(true));
    assert_eq!(r["mock"], json!(true));
    assert_eq!(r["backend"], json!("mock"));
    assert_eq!(r["storeVersion"], json!(0));

    // seeded state
    let r: Value = c.get(format!("{}/api/state", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["boardId"], json!("protocol"));
    assert_eq!(r["cards"].as_object().unwrap().len(), 21);
    assert_eq!(r["cards"]["p1"]["handle"], json!("OnboardingDropOff"));

    // CLI through HTTP (including a refusal → 400 with ok:false)
    let r = c.post(format!("{}/api/cli", srv.base_url))
        .json(&json!({"command": "evaluate p1"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["output"], json!("Living (S 0.660)"));
    let r = c.post(format!("{}/api/cli", srv.base_url))
        .json(&json!({"command": "move p1 gate"})).send().await.unwrap();
    let body: Value = r.json().await.unwrap();
    assert!(body["ok"].as_bool().unwrap());
    // p1 is Living: gate → articulation is the A9 happy path
    let r = c.post(format!("{}/api/cli", srv.base_url))
        .json(&json!({"command": "move p1 articulation"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    // A13 refusal: a fresh card at initiation cannot skip to the gate
    let r = c.post(format!("{}/api/cli", srv.base_url))
        .json(&json!({"command": "create --board protocol --title \"a13 test card\""})).send().await.unwrap();
    let body: Value = r.json().await.unwrap();
    assert!(body["ok"].as_bool().unwrap());
    let r = c.post(format!("{}/api/cli", srv.base_url))
        .json(&json!({"command": "move custom-0 gate"})).send().await.unwrap();
    assert_eq!(r.status(), 400);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["ok"], json!(false));
    assert!(body["output"].as_str().unwrap().starts_with("A13"));

    // state sync: version conflict → 409 with full state
    let r = c.post(format!("{}/api/state", srv.base_url))
        .json(&json!({"clientVersion": 999, "cards": {}})).send().await.unwrap();
    assert_eq!(r.status(), 409);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"], json!("version conflict"));
    assert!(body["cards"].as_object().unwrap().len() > 0);

    // state sync: matching version applies
    let ver: i64 = c.get(format!("{}/api/state", srv.base_url)).send().await.unwrap()
        .json::<Value>().await.unwrap()["version"].as_i64().unwrap();
    let r = c.post(format!("{}/api/state", srv.base_url))
        .json(&json!({"clientVersion": ver, "boardId": "dialectic"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let state: Value = c.get(format!("{}/api/state", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(state["boardId"], json!("dialectic"));

    // mock agent turn (one-shot)
    let r: Value = c.post(format!("{}/api/agent", srv.base_url))
        .json(&json!({"message": "create something"})).send().await.unwrap().json().await.unwrap();
    assert!(r["jobId"].as_str().unwrap().starts_with("job-"));
    assert_eq!(r["actions"][0]["action"], json!("create_card"));
    assert_eq!(r["results"][0]["ok"], json!(true));
    assert_eq!(r["refs"], json!({"skills": [], "cards": []}));

    // readOnly (explain path): actions come back as suggestions, store untouched
    let before: i64 = c.get(format!("{}/api/state", srv.base_url)).send().await.unwrap()
        .json::<Value>().await.unwrap()["version"].as_i64().unwrap();
    let r: Value = c.post(format!("{}/api/agent", srv.base_url))
        .json(&json!({"message": "explain this", "readOnly": true})).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["dryRun"], json!(true));
    assert_eq!(r["results"][0]["message"], json!("suggested — not executed (read-only)"));
    let after: i64 = c.get(format!("{}/api/state", srv.base_url)).send().await.unwrap()
        .json::<Value>().await.unwrap()["version"].as_i64().unwrap();
    assert_eq!(before, after);

    // NDJSON stream: queued → progress → result
    let res = c.post(format!("{}/api/agent?stream=1", srv.base_url))
        .json(&json!({"message": "stream test"})).send().await.unwrap();
    assert_eq!(res.headers()["content-type"], "application/x-ndjson; charset=utf-8");
    let text = res.text().await.unwrap();
    let lines: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    assert_eq!(lines[0]["type"], json!("queued"));
    assert!(lines.iter().any(|l| l["type"] == json!("progress")));
    assert_eq!(lines.last().unwrap()["type"], json!("result"));
    assert!(lines.last().unwrap()["results"].as_array().unwrap().len() == 1);

    // jobs: submit → detail → list → cancel-queued (jobs run fast in mock;
    // cancelling a done job is a no-op that still returns the record)
    let r: Value = c.post(format!("{}/api/jobs", srv.base_url))
        .json(&json!({"kind": "agent", "message": "queued turn"})).send().await.unwrap().json().await.unwrap();
    let job_id = r["jobId"].as_str().unwrap().to_string();
    let r: Value = c.get(format!("{}/api/jobs/{}", srv.base_url, job_id)).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["job"]["id"], json!(job_id));
    let r: Value = c.get(format!("{}/api/jobs", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert!(r["jobs"].as_array().unwrap().iter().any(|j| j["id"] == json!(job_id)));
    let r: Value = c.post(format!("{}/api/jobs/{}/cancel", srv.base_url, job_id))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(r["job"]["id"], json!(job_id));
    let r = c.get(format!("{}/api/jobs/nope", srv.base_url)).send().await.unwrap();
    assert_eq!(r.status(), 404);

    // generate skill draft (mock)
    let r: Value = c.post(format!("{}/api/skills/generate", srv.base_url))
        .json(&json!({"hint": "audit card titles for jargon"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["draft"]["name"], json!("audit-card-titles"));

    // skills CRUD
    let r = c.post(format!("{}/api/skills", srv.base_url))
        .json(&json!({"name": "Test Skill!", "description": "when testing", "content": "do the thing"}))
        .send().await.unwrap();
    assert_eq!(r.status(), 200);
    let created: Value = r.json().await.unwrap();
    assert_eq!(created["skill"]["id"], json!("test-skill")); // slugified
    let r = c.post(format!("{}/api/skills", srv.base_url))
        .json(&json!({"name": "test-skill", "description": "dup", "content": "x"}))
        .send().await.unwrap();
    assert_eq!(r.status(), 409);
    let r: Value = c.get(format!("{}/api/skills", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert!(r["skills"].as_array().unwrap().iter().any(|s| s["id"] == json!("test-skill")));
    assert!(r["skills"].as_array().unwrap().iter().any(|s| s["id"] == json!("skill-generator")));
    // update content
    let r = c.post(format!("{}/api/skills", srv.base_url))
        .json(&json!({"id": "test-skill", "content": "v2 body"})).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let r = c.delete(format!("{}/api/skills?id=test-skill", srv.base_url)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let r = c.delete(format!("{}/api/skills?id=test-skill", srv.base_url)).send().await.unwrap();
    assert_eq!(r.status(), 404);

    // skills/all scopes shape
    let r: Value = c.get(format!("{}/api/skills/all", srv.base_url)).send().await.unwrap().json().await.unwrap();
    let scopes = r["scopes"].as_array().unwrap();
    assert_eq!(scopes[0]["scope"], json!("workbench"));
    assert_eq!(scopes[0]["writable"], json!(true));
    assert!(r["builtin"].as_str().unwrap().contains("Built-in"));

    // API keys: set → masked → never returned raw → delete
    let r: Value = c.post(format!("{}/api/keys", srv.base_url))
        .json(&json!({"name": "TAVILY_API_KEY", "value": "tvly-abcdefghij1234"}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(r["keys"][0]["masked"], json!("tvly…1234"));
    let raw = serde_json::to_string(&r).unwrap();
    assert!(!raw.contains("abcdefghij"));
    let r: Value = c.get(format!("{}/api/keys", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["keys"].as_array().unwrap().len(), 1);
    let r: Value = c.delete(format!("{}/api/keys?name=TAVILY_API_KEY", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["keys"].as_array().unwrap().len(), 0);
    // invalid key name
    let r = c.post(format!("{}/api/keys", srv.base_url))
        .json(&json!({"name": "1-BAD", "value": "x"})).send().await.unwrap();
    assert_eq!(r.status(), 400);

    // settings: system prompt override round-trip
    let r: Value = c.get(format!("{}/api/settings", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["systemPrompt"], Value::Null);
    assert!(r["defaultSystemPrompt"].as_str().unwrap().contains("operator agent"));
    let r: Value = c.put(format!("{}/api/settings", srv.base_url))
        .json(&json!({"systemPrompt": "custom instructions"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["systemPrompt"], json!("custom instructions"));
    let r: Value = c.put(format!("{}/api/settings", srv.base_url))
        .json(&json!({"systemPrompt": ""})).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["systemPrompt"], Value::Null);

    // MCP: add stdio server → synced to the parent's .kimi-code/mcp.json
    let r: Value = c.post(format!("{}/api/mcp", srv.base_url))
        .json(&json!({"name": "fs", "entry": {"command": "npx", "args": ["-y", "@mcp/fs"]}}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(r["mcpServers"]["fs"]["command"], json!("npx"));
    let project_mcp_path = r["mcpServers"].as_object().unwrap();
    assert!(project_mcp_path.contains_key("fs"));

    // functions: write → list → run (needs node; skip the run if absent)
    let r = c.post(format!("{}/api/functions", srv.base_url))
        .json(&json!({"name": "hello", "code": "// @description says hi\nlet i='';process.stdin.on('data',d=>i+=d).on('end',()=>{console.log(JSON.stringify({message:'hi '+((JSON.parse(i).args||{}).who||'world'),actions:[]}))});"}))
        .send().await.unwrap();
    assert_eq!(r.status(), 200);
    let r: Value = c.get(format!("{}/api/functions", srv.base_url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["functions"][0]["name"], json!("hello"));
    assert_eq!(r["functions"][0]["description"], json!("says hi"));
    let node_available = std::process::Command::new("node").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if node_available {
        let r: Value = c.post(format!("{}/api/functions/hello/run", srv.base_url))
            .json(&json!({"args": {"who": "qpt"}})).send().await.unwrap().json().await.unwrap();
        assert_eq!(r["output"]["message"], json!("hi qpt"));
        assert_eq!(r["applied"], json!([]));
    }
    let r = c.delete(format!("{}/api/functions?name=hello", srv.base_url)).send().await.unwrap();
    assert_eq!(r.status(), 200);

    // static files
    let r = c.get(format!("{}/", srv.base_url)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "text/html; charset=utf-8");
    assert_eq!(r.headers()["cache-control"], "no-cache");
    assert_eq!(r.text().await.unwrap(), "<h1>qpt</h1>");
    let r = c.get(format!("{}/missing.js", srv.base_url)).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let r = c.get(format!("{}/..%2f..%2fetc%2fpasswd", srv.base_url)).send().await.unwrap();
    assert!(r.status() == 404 || r.status() == 403);

    // CORS preflight
    let r = c.request(reqwest::Method::OPTIONS, format!("{}/api/state", srv.base_url)).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
}
