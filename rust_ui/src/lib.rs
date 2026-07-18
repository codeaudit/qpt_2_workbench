//! QPT Workbench — Rust port of the qpt-ui Node backend.
//!
//! Module layout mirrors the original files 1:1:
//! - `core`      ← qpt-core.js   (domain mechanics: gate, rules, promotion, actions)
//! - `data`      ← qpt-data.js   (boards structure + seed cards, embedded JSON)
//! - `store`     ← server-store.js
//! - `cli_exec`  ← cli-exec.js
//! - `jobs`      ← server-jobs.js
//! - `skills`    ← server-skills.js
//! - `settings`  ← server-settings.js
//! - `functions` ← server-functions.js
//! - `agent`     ← the /api/agent section of server.js
//! - `log`       ← server-log.js
//! - `http`      ← the routing/static section of server.js

pub mod agent;
pub mod cli_exec;
pub mod core;
pub mod data;
pub mod functions;
pub mod http;
pub mod jobs;
pub mod log;
pub mod settings;
pub mod skills;
pub mod store;

use std::path::PathBuf;

/// Filesystem locations the app cares about. In the Node version these were
/// derived from `__dirname` (the qpt-ui dir) and its parent; here they derive
/// from the directory the binary runs in (the crate root), matching that layout.
#[derive(Clone, Debug)]
pub struct Paths {
    /// the qpt_rust_ui dir (equivalent of qpt-ui in the Node layout)
    pub root: PathBuf,
    /// parent of root — cwd for agent spawns and home of `.kimi-code/mcp.json`
    pub parent: PathBuf,
    pub data: PathBuf,
    pub public: PathBuf,
}

impl Paths {
    pub fn from_root(root: PathBuf) -> Paths {
        let parent = root.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| root.clone());
        let data = root.join("data");
        let public = root.join("public");
        Paths { root, parent, data, public }
    }

    /// Resolve from the current working directory.
    pub fn cwd() -> Paths {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Paths::from_root(cwd)
    }

    pub fn store_file(&self) -> PathBuf {
        self.data.join("store.json")
    }
    pub fn config_file(&self) -> PathBuf {
        self.data.join("config.json")
    }
    pub fn skills_dir(&self) -> PathBuf {
        self.data.join("skills")
    }
    pub fn functions_dir(&self) -> PathBuf {
        self.data.join("functions")
    }
    pub fn log_file(&self) -> PathBuf {
        self.data.join("server.log")
    }
}

/// `$KIMI_CODE_HOME` or `~/.kimi-code`.
pub fn kimi_code_home() -> PathBuf {
    if let Ok(h) = std::env::var("KIMI_CODE_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    home_dir().join(".kimi-code")
}

pub fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

/// Expand a leading `~` in a path string (used for extra_skill_dirs).
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        home_dir().join(rest)
    } else if p == "~" {
        home_dir()
    } else {
        PathBuf::from(p)
    }
}

/// `Date.now().toString(36)` — used in job ids.
pub fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Serialize the way Node's `JSON.stringify(value, null, 1)` does (1-space indent).
pub fn json_indent1<T: serde::Serialize>(value: &T) -> String {
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    value.serialize(&mut ser).expect("json serialization");
    String::from_utf8(buf).expect("json utf8")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// JS `parseFloat` semantics: parse the longest valid numeric prefix.
/// Returns None when JS would produce NaN.
pub fn js_parse_float(s: &str) -> Option<f64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?").unwrap());
    let s = s.trim_start();
    let m = re.find(s)?;
    m.as_str().parse::<f64>().ok()
}

/// JS `parseInt(_, 10)` semantics: longest valid integer prefix.
pub fn js_parse_int(s: &str) -> Option<i64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"^[+-]?\d+").unwrap());
    let s = s.trim_start();
    let m = re.find(s)?;
    m.as_str().parse::<i64>().ok()
}

/// (mtime, len) fingerprint of a file — the validation key for the
/// read/parse caches in `skills`, `functions`, and `agent`. None when the
/// file cannot be stat'ed (which callers treat like a read failure).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime: Option<std::time::SystemTime>,
    pub len: u64,
}

pub fn file_stamp(path: &std::path::Path) -> Option<FileStamp> {
    let m = std::fs::metadata(path).ok()?;
    Some(FileStamp { mtime: m.modified().ok(), len: m.len() })
}
