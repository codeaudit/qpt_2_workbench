//! Port of server-skills.js — skills on disk in the Agent Skills open format
//! (data/skills/<name>/SKILL.md), plus cross-scope browsing (workbench, user,
//! project, extra) and the skill-generator meta-skill.

use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::Paths;

pub const GENERATOR_SKILL_ID: &str = "skill-generator";

/* ------------------------------------------------------- frontmatter io */

fn unquote(v: &str) -> String {
    let t = v.trim();
    let bytes = t.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        if let Ok(Value::String(s)) = serde_json::from_str::<Value>(t) {
            return s;
        }
        return t[1..t.len() - 1].to_string();
    }
    t.to_string()
}

fn yaml_scalar(v: &str) -> String {
    static RE_NL: OnceLock<regex::Regex> = OnceLock::new();
    static RE_BAD: OnceLock<regex::Regex> = OnceLock::new();
    let re_nl = RE_NL.get_or_init(|| regex::Regex::new(r"\s*\n\s*").unwrap());
    let s = re_nl.replace_all(v, " ").trim().to_string();
    let starts_ok = s
        .chars()
        .next()
        .map(|c| c.is_ascii_alphanumeric() || c == '(')
        .unwrap_or(false);
    let has_bad = RE_BAD
        .get_or_init(|| regex::Regex::new(r"[:#]\s").unwrap())
        .is_match(&s);
    if starts_ok && !has_bad {
        s
    } else {
        serde_json::to_string(&s).unwrap_or_default()
    }
}

fn dump_skill_md(
    name: &str,
    description: &str,
    license: Option<&str>,
    compatibility: Option<&str>,
    metadata: Option<&Map<String, Value>>,
    allowed_tools: Option<&str>,
    body: &str,
) -> String {
    let mut lines = vec![
        "---".to_string(),
        format!("name: {}", yaml_scalar(name)),
        format!("description: {}", yaml_scalar(description)),
    ];
    if let Some(l) = license {
        lines.push(format!("license: {}", yaml_scalar(l)));
    }
    if let Some(c) = compatibility {
        lines.push(format!("compatibility: {}", yaml_scalar(c)));
    }
    if let Some(m) = metadata {
        if !m.is_empty() {
            lines.push("metadata:".into());
            for (k, v) in m {
                let s = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                lines.push(format!("  {}: {}", k, serde_json::to_string(&s).unwrap_or_default()));
            }
        }
    }
    if let Some(a) = allowed_tools {
        lines.push(format!("allowed-tools: {}", yaml_scalar(a)));
    }
    lines.push("---".into());
    lines.push(String::new());
    format!("{}{}\n", lines.join("\n"), body)
}

#[derive(Clone)]
struct ParsedSkill {
    fm: Map<String, Value>,
    body: String,
}

fn parse_skill_md(text: &str) -> Option<ParsedSkill> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    static NESTED_RE: OnceLock<regex::Regex> = OnceLock::new();
    static KV_RE: OnceLock<regex::Regex> = OnceLock::new();
    static BODY_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?s)\A---\r?\n(.*?)\r?\n---\r?\n?(.*)\z").unwrap());
    let m = re.captures(text)?;
    let header = m.get(1)?.as_str();
    let mut fm = Map::new();
    let mut in_metadata = false;
    let mut metadata = Map::new();
    let nested_re = NESTED_RE.get_or_init(|| regex::Regex::new(r"^\s+([^:]+):\s*(.*)$").unwrap());
    let kv_re = KV_RE.get_or_init(|| regex::Regex::new(r"^([A-Za-z_-]+):\s*(.*)$").unwrap());
    for line in header.split('\n') {
        if in_metadata {
            if let Some(n) = nested_re.captures(line) {
                metadata.insert(n[1].trim().to_string(), Value::String(unquote(&n[2])));
                continue;
            }
        }
        let Some(kv) = kv_re.captures(line) else { continue };
        if &kv[1] == "metadata" {
            in_metadata = true;
        } else {
            fm.insert(kv[1].to_string(), Value::String(unquote(&kv[2])));
        }
    }
    if in_metadata {
        fm.insert("metadata".into(), Value::Object(metadata));
    }
    let body = BODY_RE
        .get_or_init(|| regex::Regex::new(r"^\s*\n").unwrap())
        .replace(m.get(2)?.as_str(), "")
        .to_string();
    Some(ParsedSkill { fm, body })
}

/* -------------------------------------------------- mtime-validated caches
 *
 * The list endpoints and the agent turn re-scan the same files constantly.
 * Directory listings are re-read on every call (cheap); file CONTENTS are
 * cached per path and only re-read/re-parsed when (mtime, len) change.
 * The server's own CRUD writes invalidate their entries explicitly so HTTP
 * mutations are visible immediately even on coarse-mtime filesystems.
 */

type SkillParseCache = Mutex<std::collections::HashMap<PathBuf, (crate::FileStamp, Option<ParsedSkill>)>>;

static SKILL_PARSE_CACHE: OnceLock<SkillParseCache> = OnceLock::new();

fn skill_parse_cache() -> &'static SkillParseCache {
    SKILL_PARSE_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Drop a cached parse after the server itself rewrote/removed the file.
fn invalidate_skill_file(file: &Path) {
    if let Ok(mut map) = skill_parse_cache().lock() {
        map.remove(file);
    }
}

/// Read + parse a SKILL.md, cached by (mtime, len). Returns None when the
/// file is unreadable (not cached); the inner Option is None when the file
/// reads fine but has no valid frontmatter.
fn parse_skill_file(file: &Path) -> Option<(Option<ParsedSkill>, u64)> {
    let stamp = crate::file_stamp(file)?;
    let mtime_ms = stamp
        .mtime
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let map = skill_parse_cache().lock().ok();
    if let Some(map) = &map {
        if let Some((s, parsed)) = map.get(file) {
            if *s == stamp {
                return Some((parsed.clone(), mtime_ms));
            }
        }
    }
    let text = std::fs::read_to_string(file).ok()?; // I/O failures are not cached
    let parsed = parse_skill_md(&text);
    if let Some(mut map) = map {
        map.insert(file.to_path_buf(), (stamp, parsed.clone()));
    }
    Some((parsed, mtime_ms))
}

/// Read the CLI's config.toml (shared with the model listing), contents
/// cached by (mtime, len).
pub fn read_cli_config_toml() -> Option<String> {
    static CACHE: OnceLock<Mutex<Option<(crate::FileStamp, String)>>> = OnceLock::new();
    let path = crate::kimi_code_home().join("config.toml");
    let stamp = crate::file_stamp(&path)?;
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().ok()?;
    if let Some((s, text)) = guard.as_ref() {
        if *s == stamp {
            return Some(text.clone());
        }
    }
    let text = std::fs::read_to_string(&path).ok()?;
    *guard = Some((stamp, text.clone()));
    Some(text)
}

/* ------------------------------------------------------------ validation */

fn name_error(name: &str) -> Option<String> {
    static NAME_RE: OnceLock<regex::Regex> = OnceLock::new();
    if name.is_empty() {
        return Some("name is required".into());
    }
    if name.chars().count() > 64 {
        return Some("name must be ≤ 64 characters".into());
    }
    if !NAME_RE
        .get_or_init(|| regex::Regex::new(r"^[a-z0-9-]+$").unwrap())
        .is_match(name)
    {
        return Some("name: lowercase letters, numbers, and hyphens only".into());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Some("name must not start or end with a hyphen".into());
    }
    if name.contains("--") {
        return Some("name must not contain consecutive hyphens".into());
    }
    None
}

fn desc_error(d: &str) -> Option<String> {
    if d.trim().is_empty() {
        return Some("description is required (non-empty, ≤ 1024 chars)".into());
    }
    if d.chars().count() > 1024 {
        return Some("description must be ≤ 1024 characters".into());
    }
    None
}

pub fn slugify(s: &str) -> String {
    static NON_SLUG_RE: OnceLock<regex::Regex> = OnceLock::new();
    static DASHES_RE: OnceLock<regex::Regex> = OnceLock::new();
    let s = s.to_lowercase();
    let s = NON_SLUG_RE.get_or_init(|| regex::Regex::new(r"[^a-z0-9]+").unwrap()).replace_all(&s, "-");
    let s = DASHES_RE.get_or_init(|| regex::Regex::new(r"-{2,}").unwrap()).replace_all(&s, "-");
    s.trim_matches('-').to_string()
}

/* ------------------------------------------------------------------ crud */

#[derive(Clone, Debug, Serialize)]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "allowedTools")]
    pub allowed_tools: Option<String>,
    pub updated: u64,
    // external-scope / aggregate fields (absent on plain workbench records)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub type_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "whenToUse")]
    pub when_to_use: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed: Option<bool>,
}

fn skill_dir(paths: &Paths, id: &str) -> Result<PathBuf, String> {
    // ids are validated slugs — never allow path traversal
    match name_error(id) {
        Some(e) => Err(format!("invalid skill name: {}", e)),
        None => Ok(paths.skills_dir().join(id)),
    }
}

fn mtime_ms(file: &Path) -> u64 {
    std::fs::metadata(file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn to_record(id: &str, parsed: ParsedSkill, mtime: u64) -> SkillRecord {
    let ParsedSkill { fm, body } = parsed;
    let fm_str = |k: &str| fm.get(k).and_then(Value::as_str).map(str::to_string);
    SkillRecord {
        id: id.to_string(),
        name: fm_str("name").filter(|s| !s.is_empty()).unwrap_or_else(|| id.to_string()),
        description: fm_str("description").unwrap_or_default(),
        content: body,
        license: fm_str("license"),
        compatibility: fm_str("compatibility"),
        metadata: fm.get("metadata").and_then(Value::as_object).cloned(),
        allowed_tools: fm_str("allowed-tools"),
        updated: mtime,
        scope: None,
        writable: None,
        location: None,
        type_: None,
        when_to_use: None,
        shadowed: None,
    }
}

pub fn list_skills(paths: &Paths) -> (Vec<SkillRecord>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let dir = paths.skills_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return (out, warnings),
    };
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let file = dir.join(&name).join("SKILL.md");
        let Some((parsed, mtime)) = parse_skill_file(&file) else { continue };
        match parsed {
            None => warnings.push(format!("{}: no YAML frontmatter", name)),
            Some(parsed) => {
                if let Some(fm_name) = parsed.fm.get("name").and_then(Value::as_str) {
                    if !fm_name.is_empty() && fm_name != name {
                        warnings.push(format!(
                            "{}: name field {} ≠ directory name (spec)",
                            name,
                            serde_json::to_string(&fm_name).unwrap_or_default()
                        ));
                    }
                }
                out.push(to_record(&name, parsed, mtime));
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    (out, warnings)
}

pub fn read_skill(paths: &Paths, id: &str) -> Result<SkillRecord, String> {
    let file = skill_dir(paths, id)?.join("SKILL.md");
    let text = std::fs::read_to_string(&file)
        .map_err(|_| format!("ENOENT: no such file or directory, open '{}'", file.display()))?;
    let parsed = parse_skill_md(&text)
        .ok_or_else(|| format!("skill {} has no valid frontmatter", id))?;
    Ok(to_record(id, parsed, mtime_ms(&file)))
}

pub fn create_skill(paths: &Paths, fields: &Value) -> Result<SkillRecord, String> {
    let name = fields.get("name").and_then(Value::as_str).unwrap_or("");
    let id = if name_error(name).is_none() { name.to_string() } else { slugify(name) };
    if let Some(ne) = name_error(&id) {
        return Err(ne);
    }
    let description = fields.get("description").and_then(Value::as_str).unwrap_or("");
    if let Some(de) = desc_error(description) {
        return Err(de);
    }
    if skill_dir(paths, &id)?.exists() {
        return Err(format!("skill {} already exists", id));
    }
    let content = fields.get("content").and_then(Value::as_str).unwrap_or("");
    if content.split('\n').count() > 500 {
        // spec recommendation: keep SKILL.md under 500 lines — warn, don't block
        crate::log::warn("skills", &[("id", Value::String(format!("{} body exceeds 500 lines (spec recommendation)", id)))]);
    }
    let metadata = fields.get("metadata").and_then(Value::as_object).cloned();
    std::fs::create_dir_all(skill_dir(paths, &id)?).map_err(|e| e.to_string())?;
    let text = dump_skill_md(
        &id,
        description.trim(),
        fields.get("license").and_then(Value::as_str),
        fields.get("compatibility").and_then(Value::as_str),
        metadata.as_ref(),
        fields.get("allowedTools").and_then(Value::as_str),
        content,
    );
    let file = skill_dir(paths, &id)?.join("SKILL.md");
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    invalidate_skill_file(&file);
    read_skill(paths, &id)
}

pub fn update_skill(paths: &Paths, id: &str, patch: &Value) -> Result<SkillRecord, String> {
    let current = read_skill(paths, id)?; // Err if missing
    let str_patch = |k: &str| patch.get(k).and_then(Value::as_str).map(str::to_string);
    let next_name = str_patch("name").unwrap_or_else(|| current.name.clone());
    let next_desc = str_patch("description").unwrap_or_else(|| current.description.clone());
    let next_license = if patch.get("license").is_some() {
        str_patch("license")
    } else {
        current.license.clone()
    };
    let next_compat = if patch.get("compatibility").is_some() {
        str_patch("compatibility")
    } else {
        current.compatibility.clone()
    };
    let next_meta = if patch.get("metadata").is_some() {
        patch.get("metadata").and_then(Value::as_object).cloned()
    } else {
        current.metadata.clone()
    };
    let next_tools = if patch.get("allowedTools").is_some() {
        str_patch("allowedTools")
    } else {
        current.allowed_tools.clone()
    };
    let next_body = str_patch("content").unwrap_or_else(|| current.content.clone());

    if let Some(ne) = name_error(&next_name) {
        return Err(ne);
    }
    if let Some(de) = desc_error(&next_desc) {
        return Err(de);
    }
    if next_name != id {
        // spec: name must match the parent directory — rename it
        let target = skill_dir(paths, &next_name)?;
        if target.exists() {
            return Err(format!("skill {} already exists", next_name));
        }
        std::fs::rename(skill_dir(paths, id)?, &target).map_err(|e| e.to_string())?;
        invalidate_skill_file(&skill_dir(paths, id)?.join("SKILL.md"));
    }
    let text = dump_skill_md(
        &next_name,
        &next_desc,
        next_license.as_deref(),
        next_compat.as_deref(),
        next_meta.as_ref(),
        next_tools.as_deref(),
        &next_body,
    );
    let file = skill_dir(paths, &next_name)?.join("SKILL.md");
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    invalidate_skill_file(&file);
    read_skill(paths, &next_name)
}

pub fn delete_skill(paths: &Paths, id: &str) -> Result<String, String> {
    let dir = skill_dir(paths, id)?;
    if !dir.exists() {
        return Err(format!("ENOENT: no such file or directory, stat '{}'", dir.display()));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    invalidate_skill_file(&dir.join("SKILL.md"));
    Ok(id.to_string())
}

/* ------------------------------------------- browse skills across scopes
 *
 * Kimi Code CLI discovery: Project > User > Extra > Built-in
 *   user:    $KIMI_CODE_HOME/skills (~/.kimi-code/skills), ~/.agents/skills
 *   project: <nearest .git ancestor>/.kimi-code/skills, .agents/skills
 *   extra:   extra_skill_dirs in config.toml
 * Flat-form <name>.md files and extra frontmatter (type, whenToUse) are
 * parsed leniently, as in the JS version.
 */

fn list_dir_skills(dir: &Path, scope: &str, writable: bool) -> Vec<SkillRecord> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    // Node's fs.readdir comes back name-sorted on the filesystems we target;
    // sort to match deterministically.
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        let (file, id) = if ft.is_dir() {
            let candidate = dir.join(&name).join("SKILL.md");
            if !candidate.exists() {
                continue;
            }
            (candidate, name)
        } else if ft.is_file() && name.ends_with(".md") {
            (dir.join(&name), name.trim_end_matches(".md").to_string())
        } else {
            continue;
        };
        let Some((Some(parsed), mtime)) = parse_skill_file(&file) else { continue };
        let first_line = parsed
            .body
            .split('\n')
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .chars()
            .take(240)
            .collect::<String>();
        let fm_str = |k: &str| parsed.fm.get(k).and_then(Value::as_str).map(str::to_string);
        let mut rec = SkillRecord {
            id: id.clone(),
            name: fm_str("name").filter(|s| !s.is_empty()).unwrap_or(id),
            description: fm_str("description").unwrap_or(first_line),
            content: parsed.body,
            license: None,
            compatibility: None,
            metadata: None,
            allowed_tools: None,
            updated: mtime,
            scope: Some(scope.to_string()),
            writable: Some(writable),
            location: Some(file.to_string_lossy().to_string()),
            type_: fm_str("type"),
            when_to_use: fm_str("whenToUse")
                .or_else(|| fm_str("when-to-use"))
                .or_else(|| fm_str("when_to_use")),
            shadowed: None,
        };
        rec.shadowed = None;
        out.push(rec);
    }
    out
}

fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return Some(d);
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

fn extra_skill_dirs() -> Vec<PathBuf> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    static QUOTED_RE: OnceLock<regex::Regex> = OnceLock::new();
    let Some(toml_text) = read_cli_config_toml() else { return Vec::new() };
    let re = RE.get_or_init(|| regex::Regex::new(r"(?m)^\s*extra_skill_dirs\s*=\s*\[([^\]]*)\]").unwrap());
    let Some(m) = re.captures(&toml_text) else { return Vec::new() };
    let quoted = QUOTED_RE.get_or_init(|| regex::Regex::new(r#""([^"]*)""#).unwrap());
    quoted
        .captures_iter(&m[1])
        .map(|x| crate::expand_tilde(&x[1]))
        .collect()
}

#[derive(Serialize)]
pub struct ScopeListing {
    pub scope: String,
    pub dir: String,
    pub writable: bool,
    pub skills: Vec<SkillRecord>,
    pub warnings: Vec<String>,
}

pub fn list_all_skills(paths: &Paths) -> (Vec<ScopeListing>, String) {
    let project_root = find_project_root(&paths.root);

    let mut scopes: Vec<ScopeListing> = Vec::new();
    let (own, own_warnings) = list_skills(paths);
    scopes.push(ScopeListing {
        scope: "workbench".into(),
        dir: paths.skills_dir().to_string_lossy().to_string(),
        writable: true,
        skills: own
            .into_iter()
            .map(|mut s| {
                s.scope = Some("workbench".into());
                s.writable = Some(true);
                s
            })
            .collect(),
        warnings: own_warnings,
    });

    let user_dirs = vec![
        crate::kimi_code_home().join("skills"),
        crate::home_dir().join(".agents").join("skills"),
    ];
    let mut user_skills = Vec::new();
    for d in &user_dirs {
        user_skills.extend(list_dir_skills(d, "user", false));
    }
    scopes.push(ScopeListing {
        scope: "user".into(),
        dir: user_dirs.iter().map(|d| d.to_string_lossy().to_string()).collect::<Vec<_>>().join(" · "),
        writable: false,
        skills: user_skills,
        warnings: Vec::new(),
    });

    let mut project_skills = Vec::new();
    let mut project_dir_label = "(no .git ancestor)".to_string();
    if let Some(root) = &project_root {
        let dirs = vec![root.join(".kimi-code").join("skills"), root.join(".agents").join("skills")];
        project_dir_label = dirs.iter().map(|d| d.to_string_lossy().to_string()).collect::<Vec<_>>().join(" · ");
        for d in &dirs {
            project_skills.extend(list_dir_skills(d, "project", false));
        }
    }
    scopes.push(ScopeListing {
        scope: "project".into(),
        dir: project_dir_label,
        writable: false,
        skills: project_skills,
        warnings: Vec::new(),
    });

    let extras = extra_skill_dirs();
    let mut extra_skills = Vec::new();
    for d in &extras {
        extra_skills.extend(list_dir_skills(d, "extra", false));
    }
    scopes.push(ScopeListing {
        scope: "extra".into(),
        dir: if extras.is_empty() {
            "(none configured)".into()
        } else {
            extras.iter().map(|d| d.to_string_lossy().to_string()).collect::<Vec<_>>().join(" · ")
        },
        writable: false,
        skills: extra_skills,
        warnings: Vec::new(),
    });

    // mark shadowing: Project > User > Extra — same name at higher priority wins
    let priority = |scope: &str| match scope {
        "project" => 0,
        "user" => 1,
        "extra" => 2,
        _ => 3,
    };
    let mut seen: std::collections::HashMap<String, (usize, usize)> = std::collections::HashMap::new();
    for (si, sc) in scopes.iter().enumerate() {
        for (ki, s) in sc.skills.iter().enumerate() {
            let key = s.name.to_lowercase();
            match seen.get(&key) {
                None => {
                    seen.insert(key, (si, ki));
                }
                Some(&(psi, _)) if priority(sc.scope.as_str()) < priority(scopes[psi].scope.as_str()) => {
                    seen.insert(key, (si, ki));
                }
                _ => {}
            }
        }
    }
    for (si, sc) in scopes.iter_mut().enumerate() {
        for (ki, s) in sc.skills.iter_mut().enumerate() {
            let winner = seen.get(&s.name.to_lowercase()).copied();
            s.shadowed = Some(winner != Some((si, ki)));
        }
    }

    (scopes, "Built-in skills ship with the CLI itself (no files to browse).".to_string())
}

/// Copy an external skill into the workbench store.
pub fn import_skill(paths: &Paths, id: &str) -> Result<SkillRecord, String> {
    let (scopes, _) = list_all_skills(paths);
    let mut hit: Option<SkillRecord> = None;
    for sc in &scopes {
        for s in &sc.skills {
            if s.id == id && s.scope.as_deref() != Some("workbench") && hit.is_none() {
                hit = Some(s.clone());
            }
        }
    }
    let hit = hit.ok_or_else(|| format!("no external skill {}", id))?;
    create_skill(
        paths,
        &serde_json::json!({
            "name": id,
            "description": if hit.description.is_empty() { format!("imported from {} scope", hit.scope.unwrap_or_default()) } else { hit.description },
            "content": hit.content,
        }),
    )
}

/* ------------------------------------------------- the skill generator */

const GENERATOR_BODY: &str = r#"You are a skill author. Given a short hint, you write ONE skill in the Agent Skills format (agentskills.io/specification).

OUTPUT CONTRACT — respond with ONLY a JSON object, no prose, no fences:
{"name": "…", "description": "…", "content": "…"}

RULES for name:
- 1–64 chars, lowercase letters/numbers/hyphens, no leading/trailing/consecutive hyphens.
- Short, memorable, verb- or domain-led (e.g. grounding-protocol, review-pr, jargon-audit).
- MUST NOT collide with an existing skill (the caller lists taken names).

RULES for description (the trigger line — most important field):
- 1–1024 chars. State WHAT the skill does AND WHEN to use it.
- Pack concrete keywords an agent would match on ("use when …", mentions, file types, tasks).

RULES for content (Markdown body):
- Step-by-step procedure first, then examples (input → output), then edge cases.
- Prefer checklists and numbered steps over essays.
- Under ~120 lines. If reference material is needed, say so and name a references/ file.
- No YAML frontmatter in content — the caller writes the frontmatter.

QUALITY BAR: the generated skill must be usable verbatim by an agent that has never seen the hint. No placeholders like "TODO" or "fill this in"."#;

pub fn ensure_skill_generator(paths: &Paths) {
    if read_skill(paths, GENERATOR_SKILL_ID).is_err() {
        let _ = create_skill(
            paths,
            &serde_json::json!({
                "name": GENERATOR_SKILL_ID,
                "description": "Generate a new Agent Skills skill from a short hint. Use when the user wants a new skill drafted — outputs spec-valid name, trigger description, and Markdown instructions.",
                "content": GENERATOR_BODY,
            }),
        );
    }
}
