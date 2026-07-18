//! Port of server-settings.js — workbench configuration in data/config.json
//! (gitignored): masked API keys injected as env into agent turns, system
//! prompt override, and MCP servers synced into the project-level
//! .kimi-code/mcp.json (other entries preserved).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

use crate::Paths;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "apiKeys", default)]
    pub api_keys: Map<String, Value>,
    #[serde(rename = "systemPrompt", default)]
    pub system_prompt: Option<String>,
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: Map<String, Value>,
    #[serde(rename = "managedMcpNames", default)]
    pub managed_mcp_names: Vec<String>,
    /// preserve unknown top-level fields (the JS version keeps the parsed object)
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub struct Settings {
    pub cfg: Config,
    file: PathBuf,
    project_mcp: PathBuf,
}

impl Settings {
    fn save(&self) {
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.file.with_extension("json.tmp");
        if std::fs::write(&tmp, crate::json_indent1(&self.cfg)).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    /* ---- api keys (values never leave the server; masked in all responses) ---- */

    pub fn masked_keys(&self) -> Vec<Value> {
        self.cfg
            .api_keys
            .iter()
            .map(|(name, value)| {
                let v = value.as_str().unwrap_or("");
                let masked = if v.chars().count() <= 8 {
                    "••••••••".to_string()
                } else {
                    format!("{}…{}", v.chars().take(4).collect::<String>(), v.chars().rev().take(4).collect::<String>().chars().rev().collect::<String>())
                };
                json!({ "name": name, "masked": masked })
            })
            .collect()
    }

    pub fn set_key(&mut self, name: &str, value: &str) -> Result<String, String> {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        if !RE.get_or_init(|| regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap()).is_match(name) {
            return Err("key name must be an env-var style identifier".into());
        }
        if value.trim().is_empty() {
            return Err("key value required".into());
        }
        self.cfg.api_keys.insert(name.to_string(), json!(value.trim()));
        self.save();
        Ok(name.to_string())
    }

    pub fn delete_key(&mut self, name: &str) {
        self.cfg.api_keys.remove(name);
        self.save();
    }

    /// Injected into the child process environment for agent turns.
    pub fn env_for_agent(&self) -> Vec<(String, String)> {
        self.cfg
            .api_keys
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect()
    }

    /* ---- system prompt override ---- */

    pub fn set_system_prompt(&mut self, text: Option<&str>) {
        self.cfg.system_prompt = match text {
            Some(t) if !t.trim().is_empty() => Some(t.to_string()),
            _ => None,
        };
        self.save();
    }

    /* ---- MCP servers (synced to the project-level mcp.json) ---- */

    pub fn project_mcp_path(&self) -> &PathBuf {
        &self.project_mcp
    }

    fn sync_mcp_file(&mut self) {
        let mut existing: Value = std::fs::read_to_string(&self.project_mcp)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({ "mcpServers": {} }));
        if existing.get("mcpServers").and_then(Value::as_object).is_none() {
            existing["mcpServers"] = json!({});
        }
        {
            let servers = existing["mcpServers"].as_object_mut().unwrap();
            // preserve entries we do not manage; replace ours wholesale
            for name in &self.cfg.managed_mcp_names {
                servers.remove(name);
            }
            for (k, v) in &self.cfg.mcp_servers {
                servers.insert(k.clone(), v.clone());
            }
        }
        self.cfg.managed_mcp_names = self.cfg.mcp_servers.keys().cloned().collect();
        if let Some(dir) = self.project_mcp.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::copy(&self.project_mcp, self.project_mcp.with_extension("json.bak"));
        let _ = std::fs::write(&self.project_mcp, crate::json_indent1(&existing));
        self.save();
    }

    fn validate_mcp_entry(name: &str, entry: &Value) -> Result<Value, String> {
        static NAME_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        if !NAME_RE
            .get_or_init(|| regex::Regex::new(r"(?i)^[a-z0-9][a-z0-9-_]*$").unwrap())
            .is_match(name)
        {
            return Err("server name: slug (letters, numbers, - _)".into());
        }
        if !entry.is_object() {
            return Err("server entry required".into());
        }
        if let Some(cmd) = entry.get("command").and_then(Value::as_str) {
            let args: Vec<Value> = match entry.get("args") {
                Some(Value::Array(a)) => a.iter().map(|v| match v {
                    Value::String(s) => json!(s),
                    other => json!(other.to_string()),
                }).collect(),
                Some(Value::String(s)) => s.split(' ').map(|x| json!(x)).collect(),
                _ => Vec::new(),
            };
            let mut out = Map::new();
            out.insert("command".into(), json!(cmd));
            out.insert("args".into(), Value::Array(args));
            if let Some(env) = entry.get("env") {
                out.insert("env".into(), env.clone());
            }
            if let Some(cwd) = entry.get("cwd").and_then(Value::as_str) {
                out.insert("cwd".into(), json!(cwd));
            }
            if entry.get("enabled") == Some(&json!(false)) {
                out.insert("enabled".into(), json!(false));
            }
            return Ok(Value::Object(out));
        }
        if let Some(url) = entry.get("url").and_then(Value::as_str) {
            let mut out = Map::new();
            out.insert("url".into(), json!(url));
            if entry.get("transport") == Some(&json!("sse")) {
                out.insert("transport".into(), json!("sse"));
            }
            if let Some(headers) = entry.get("headers") {
                out.insert("headers".into(), headers.clone());
            }
            if let Some(b) = entry.get("bearerTokenEnvVar").and_then(Value::as_str) {
                out.insert("bearerTokenEnvVar".into(), json!(b));
            }
            if entry.get("enabled") == Some(&json!(false)) {
                out.insert("enabled".into(), json!(false));
            }
            return Ok(Value::Object(out));
        }
        Err("server needs either command (stdio) or url (http/sse)".into())
    }

    pub fn set_mcp_server(&mut self, name: &str, entry: &Value) -> Result<String, String> {
        let validated = Self::validate_mcp_entry(name, entry)?;
        self.cfg.mcp_servers.insert(name.to_string(), validated);
        self.sync_mcp_file();
        Ok(name.to_string())
    }

    pub fn delete_mcp_server(&mut self, name: &str) {
        self.cfg.mcp_servers.remove(name);
        self.sync_mcp_file();
    }
}

pub fn load_settings(paths: &Paths) -> Settings {
    let file = paths.config_file();
    let cfg: Config = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Settings {
        cfg,
        file,
        project_mcp: paths.parent.join(".kimi-code").join("mcp.json"),
    }
}
