//! Port of server-functions.js — user-generated JS functions in
//! data/functions/<name>.js. Contract: read JSON { args, state } on stdin,
//! print ONE JSON result { message, actions? } on stdout (20 s timeout).
//! Node itself remains the runtime for user functions.

use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::Paths;

pub const RUN_TIMEOUT_MS: u64 = 20_000;

fn slug_error(name: &str) -> Option<&'static str> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let ok = RE
        .get_or_init(|| regex::Regex::new(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?$").unwrap())
        .is_match(name);
    if ok {
        None
    } else {
        Some("function name: lowercase slug (a-z 0-9 -)")
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FunctionRecord {
    pub name: String,
    pub description: String,
    pub lines: usize,
}

/* -------------------------------------------------- mtime-validated cache
 *
 * list_functions runs per /api/functions request and per agent turn. The
 * directory listing is re-read every call (cheap); file contents are cached
 * per path and only re-read when (mtime, len) change. The server's own
 * writes (write/delete_function) invalidate their entries explicitly.
 */

type FunctionCache = Mutex<std::collections::HashMap<std::path::PathBuf, (crate::FileStamp, String, usize)>>;

static FUNCTION_CACHE: OnceLock<FunctionCache> = OnceLock::new();

fn function_cache() -> &'static FunctionCache {
    FUNCTION_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Drop a cached entry after the server itself rewrote/removed the file.
fn invalidate_function_file(file: &Path) {
    if let Ok(mut map) = function_cache().lock() {
        map.remove(file);
    }
}

/// (description, line count) for one function file, cached by (mtime, len).
/// None when the file is unreadable (not cached).
fn describe_function(file: &Path) -> Option<(String, usize)> {
    static DESC_RE: OnceLock<regex::Regex> = OnceLock::new();
    let stamp = crate::file_stamp(file)?;
    let map = function_cache().lock().ok();
    if let Some(map) = &map {
        if let Some((s, description, lines)) = map.get(file) {
            if *s == stamp {
                return Some((description.clone(), *lines));
            }
        }
    }
    let code = std::fs::read_to_string(file).ok()?; // I/O failures are not cached
    let desc_re = DESC_RE.get_or_init(|| regex::Regex::new(r"(?m)^//\s*@description\s+(.+)$").unwrap());
    let description = desc_re
        .captures(&code)
        .map(|m| m[1].trim().to_string())
        .unwrap_or_default();
    let lines = code.split('\n').count();
    if let Some(mut map) = map {
        map.insert(file.to_path_buf(), (stamp, description.clone(), lines));
    }
    Some((description, lines))
}

pub fn list_functions(paths: &Paths) -> Vec<FunctionRecord> {
    let mut out = Vec::new();
    let dir = paths.functions_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".js") {
            continue;
        }
        let Some((description, lines)) = describe_function(&dir.join(&name)) else { continue };
        out.push(FunctionRecord {
            name: name.trim_end_matches(".js").to_string(),
            description,
            lines,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn read_function(paths: &Paths, name: &str) -> Result<(String, String), String> {
    if let Some(e) = slug_error(name) {
        return Err(e.into());
    }
    let code = std::fs::read_to_string(paths.functions_dir().join(format!("{}.js", name)))
        .map_err(|e| e.to_string())?;
    Ok((name.to_string(), code))
}

pub fn write_function(paths: &Paths, name: &str, code: &str) -> Result<String, String> {
    if let Some(e) = slug_error(name) {
        return Err(e.into());
    }
    if code.trim().is_empty() {
        return Err("function code required".into());
    }
    std::fs::create_dir_all(paths.functions_dir()).map_err(|e| e.to_string())?;
    let file = paths.functions_dir().join(format!("{}.js", name));
    std::fs::write(&file, code).map_err(|e| e.to_string())?;
    invalidate_function_file(&file);
    Ok(name.to_string())
}

pub fn delete_function(paths: &Paths, name: &str) -> Result<String, String> {
    if let Some(e) = slug_error(name) {
        return Err(e.into());
    }
    let file = paths.functions_dir().join(format!("{}.js", name));
    std::fs::remove_file(&file).map_err(|e| e.to_string())?;
    invalidate_function_file(&file);
    Ok(name.to_string())
}

/// Run one function in a child process: JSON in, JSON out, hard timeout.
pub async fn run_function(paths: &Paths, name: &str, args: &Value, state: &Value) -> Result<Value, String> {
    read_function(paths, name)?; // Err if missing
    let payload = serde_json::to_string(&json!({ "args": if args.is_null() { json!({}) } else { args.clone() }, "state": state }))
        .map_err(|e| e.to_string())?;
    let file = paths.functions_dir().join(format!("{}.js", name));

    let mut child = tokio::process::Command::new("node")
        .arg(&file)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn node: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    let wait = child.wait_with_output();
    let output = match tokio::time::timeout(std::time::Duration::from_millis(RUN_TIMEOUT_MS), wait).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => {
            // timed out — the child is reaped by tokio on drop after kill
            return Err(format!("function {} timed out after {}s", name, RUN_TIMEOUT_MS / 1000));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let suffix: String = stderr.chars().take(300).collect();
        return Err(format!(
            "function {} exited {}{}",
            name,
            output.status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into()),
            if suffix.is_empty() { String::new() } else { format!(": {}", suffix) }
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout.trim().split('\n').filter(|l| !l.is_empty()).next_back().unwrap_or("");
    match serde_json::from_str::<Value>(last_line) {
        Ok(v) => Ok(v),
        Err(_) => Ok(json!({
            "message": stdout.trim().chars().take(2000).collect::<String>(),
            "actions": [],
        })),
    }
}
