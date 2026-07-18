//! QPT Workbench server binary (port of server.js).
//! Startup: openStore → ensureSkillGenerator → loadSettings → listen → probe CLI.

use qpt::agent::AppState;
use qpt::{http, jobs, settings, skills, store::Store, Paths};

#[tokio::main]
async fn main() {
    let paths = Paths::cwd();
    let store = Store::open(&paths);
    skills::ensure_skill_generator(&paths);
    let settings = settings::load_settings(&paths);
    let job_queue = jobs::JobQueue::new();
    let state = AppState::new(paths.clone(), store, settings, job_queue);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);

    let app = http::router(state.clone());
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            qpt::log::error("server.error", &[("message", serde_json::json!(format!("bind {}: {}", addr, e)))]);
            std::process::exit(1);
        }
    };
    qpt::log::info(
        "server.start",
        &[
            ("port", serde_json::json!(port)),
            ("mock", serde_json::json!(state.mock)),
            ("executable", serde_json::json!(state.kimi_executable)),
        ],
    );
    eprintln!("QPT Workbench → http://localhost:{}", port);

    // Fire-and-forget CLI probe (logging only — the compat backend needs no detection).
    let st = state.clone();
    tokio::spawn(async move {
        st.probe_kimi().await;
    });

    if let Err(e) = axum::serve(listener, app).await {
        qpt::log::error("server.error", &[("message", serde_json::json!(e.to_string()))]);
        std::process::exit(1);
    }
}
