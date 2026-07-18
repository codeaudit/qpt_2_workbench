//! Port of cli-exec.js — the command language over the store.
//!
//! One executor shared by the HTTP endpoint (POST /api/cli), the standalone
//! bin (qpt-cli "..."), and available to the agent. Mutations run through
//! `core` — the same enforcement as every other surface.

use serde_json::{json, Map, Value};

use crate::{core, skills, store::Store, Paths};

pub struct CliResult {
    pub ok: bool,
    pub output: String,
    pub changed: bool,
}

fn ok(output: String, changed: bool) -> CliResult {
    CliResult { ok: true, output, changed }
}

fn bad(output: String) -> CliResult {
    CliResult { ok: false, output, changed: false }
}

pub fn tokenize(s: &str) -> Vec<String> {
    // key="quoted value" | key='quoted value' | key=value | "quoted" | 'quoted' | bare
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r#"(\S+?=)(?:"([^"]*)"|'([^']*)'|(\S+))|"([^"]*)"|'([^']*)'|(\S+)"#).unwrap());
    let mut out = Vec::new();
    for m in re.captures_iter(s) {
        if let Some(key) = m.get(1) {
            let val = m.get(2).or(m.get(3)).or(m.get(4)).map(|v| v.as_str()).unwrap_or("");
            out.push(format!("{}{}", key.as_str(), val));
        } else {
            let val = m.get(5).or(m.get(6)).or(m.get(7)).map(|v| v.as_str()).unwrap_or("");
            out.push(val.to_string());
        }
    }
    out
}

const HELP: &str = r#"qpt workbench CLI
  state                              compact store snapshot (JSON)
  boards                             boards and their column flows
  board <id>                         switch the active board (protocol|dialectic|resolution)
  cards [--board <id>]               list cards on a board
  card <id>                          full card JSON
  create --board <b> --title "…"     new card at the board's entry column
       [--source recruited|initiated] [--target grounded|terminated]
       [--kind position|note] [--reliability 0..1] [--tags a,b] [--note "…"]
  move <id> <column>                 move one column (A13: no forward skips)
  edit <id> key=value …              domain-clamped edit (rho delta gamma k theta
                                     source target scale pathology death reliability genesis title note sign agent tags)
  evaluate <id>                      gate verdict with S vs θ
  promote <id> --to <board>          cross-board promotion (paths per §15/§7.4/§11/§29)
  reset                              reseed the store from the specification
  skills                             list skills (data/skills/<name>/SKILL.md)
  skill show <id>                    print a skill
  skill create <id> --description "…" [--content "…"]
  skill edit <id> [name=…] [description=…] [content=…]
  skill delete <id>"#;

/// Flags: `--key value` or bare `--key` → true, as in the JS parser.
type Flags = Map<String, Value>;

fn flag_str<'a>(flags: &'a Flags, k: &str) -> Option<&'a str> {
    flags.get(k).and_then(Value::as_str)
}

/// JS `flags.board || data.boardId` — a bare `--board` is `true` in JS.
fn flag_or(flags: &Flags, k: &str, default: &str) -> String {
    match flags.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(true)) => "true".into(),
        _ => default.to_string(),
    }
}

fn js_json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

pub fn exec_command(line: &str, store: &mut Store, paths: &Paths) -> CliResult {
    let t = tokenize(line);
    if t.is_empty() {
        return bad("empty command — try `help`".into());
    }

    let cmd = t[0].clone();
    let rest = &t[1..];
    let mut pos: Vec<String> = Vec::new();
    let mut flags: Flags = Map::new();
    let mut i = 0;
    while i < rest.len() {
        if let Some(k) = rest[i].strip_prefix("--") {
            let next = rest.get(i + 1);
            match next {
                Some(n) if !n.starts_with("--") => {
                    flags.insert(k.to_string(), Value::String(n.clone()));
                    i += 2;
                }
                _ => {
                    flags.insert(k.to_string(), Value::Bool(true));
                    i += 1;
                }
            }
        } else {
            pos.push(rest[i].clone());
            i += 1;
        }
    }

    /// Run an action through the core; save on success.
    macro_rules! finish {
        ($action:expr) => {{
            let r = core::apply_action(&mut store.data, &$action);
            if r.get("ok") == Some(&json!(true)) {
                store.save();
                ok(crate::core::strf(&r, "message").unwrap_or("").to_string(), true)
            } else {
                bad(crate::core::strf(&r, "message").unwrap_or("").to_string())
            }
        }};
    }

    match cmd.as_str() {
        "help" => ok(HELP.to_string(), false),

        "state" => {
            ok(crate::json_indent1(&core::compact_state(&store.data)), false)
        }

        "boards" => ok(
            crate::data::boards()
                .iter()
                .map(|b| {
                    format!(
                        "{}:  {}",
                        b.id,
                        b.columns.iter().map(|c| c.id.clone()).collect::<Vec<_>>().join(" → ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            false,
        ),

        "board" => {
            let id = pos.first().cloned().unwrap_or_default();
            if crate::data::boards().iter().all(|b| b.id != id) {
                return bad(format!(
                    "unknown board {}",
                    if id.is_empty() { "(missing)".to_string() } else { id }
                ));
            }
            store.data.board_id = id.clone();
            store.save();
            ok(format!("board → {}", id), true)
        }

        "cards" => {
            let board_id = flag_or(&flags, "board", &store.data.board_id);
            let list = core::cards_on(&store.data, &board_id);
            if list.is_empty() {
                return ok(format!("(no cards on {})", board_id), false);
            }
            ok(list
                .iter()
                .map(|c| {
                    let ev = core::evaluate(c);
                    format!(
                        "{}{} @ {} — {}{}",
                        core::strf(c, "id").unwrap_or(""),
                        core::strf(c, "handle")
                            .map(|h| format!(" @{}", h))
                            .unwrap_or_default(),
                        core::strf(c, "column").unwrap_or(""),
                        core::strf(c, "title").unwrap_or(""),
                        ev.map(|e| format!("  [{}]", e.key)).unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"), false)
        }

        "card" => {
            let reference = pos.first().cloned().unwrap_or_default();
            match core::find_card(&store.data.cards, &reference) {
                None => bad(format!(
                    "no card {}",
                    if reference.is_empty() { "(missing id)".to_string() } else { reference }
                )),
                Some(c) => ok(crate::json_indent1(c), false),
            }
        }

        "create" => {
            let mut action = Map::new();
            action.insert("action".into(), json!("create_card"));
            action.insert("board".into(), json!(flag_or(&flags, "board", &store.data.board_id)));
            for k in ["title", "source", "target", "kind", "note"] {
                if let Some(v) = flag_str(&flags, k) {
                    action.insert(k.into(), json!(v));
                }
            }
            if let Some(v) = flags.get("reliability") {
                let n = match v {
                    Value::String(s) => crate::js_parse_float(s),
                    Value::Bool(_) => None,
                    _ => v.as_f64(),
                };
                // JS parseFloat failure → NaN → serialized as null downstream
                action.insert("reliability".into(), n.map(|v| json!(v)).unwrap_or(Value::Null));
            }
            if let Some(tags) = flag_str(&flags, "tags") {
                let arr: Vec<Value> = tags
                    .split(',')
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .map(|x| json!(x))
                    .collect();
                action.insert("tags".into(), Value::Array(arr));
            }
            action.insert("via".into(), json!("the CLI"));
            finish!(Value::Object(action))
        }

        "move" => {
            if pos.len() < 2 {
                return bad("usage: move <id|@handle> <column>".into());
            }
            let Some(c) = core::find_card(&store.data.cards, &pos[0]) else {
                return bad(format!("no card {}", pos[0]));
            };
            let id = core::strf(c, "id").unwrap_or("").to_string();
            finish!(json!({ "action": "move_card", "id": id, "column": pos[1] }))
        }

        "edit" => {
            if pos.is_empty() {
                return bad("usage: edit <id|@handle> key=value …".into());
            }
            let Some(c) = core::find_card(&store.data.cards, &pos[0]) else {
                return bad(format!("no card {}", pos[0]));
            };
            let id = core::strf(c, "id").unwrap_or("").to_string();
            let mut patch = Map::new();
            static NUM_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let num_re = NUM_RE.get_or_init(|| regex::Regex::new(r"^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$").unwrap());
            for kv in &pos[1..] {
                let Some(eq) = kv.find('=') else {
                    return bad(format!("edit: expected key=value, got {}", js_json_str(kv)));
                };
                let v = &kv[eq + 1..];
                let value = if v == "null" {
                    Value::Null
                } else if v == "true" {
                    json!(true)
                } else if v == "false" {
                    json!(false)
                } else if !v.is_empty() && num_re.is_match(v) {
                    json!(v.parse::<f64>().unwrap_or(0.0))
                } else if v.contains(',') {
                    Value::Array(
                        v.split(',')
                            .map(|x| x.trim())
                            .filter(|x| !x.is_empty())
                            .map(|x| json!(x))
                            .collect(),
                    )
                } else {
                    json!(v)
                };
                patch.insert(kv[..eq].to_string(), value);
            }
            finish!(json!({ "action": "edit_card", "id": id, "patch": Value::Object(patch) }))
        }

        "evaluate" => {
            let reference = pos.first().cloned().unwrap_or_default();
            let Some(c) = core::find_card(&store.data.cards, &reference) else {
                return bad(format!(
                    "no card {}",
                    if reference.is_empty() { "(missing id)".to_string() } else { reference }
                ));
            };
            let id = core::strf(c, "id").unwrap_or("").to_string();
            let r = core::apply_action(&mut store.data, &json!({ "action": "evaluate_card", "id": id }));
            let msg = core::strf(&r, "message").unwrap_or("").to_string();
            if r.get("ok") == Some(&json!(true)) {
                ok(msg, false)
            } else {
                bad(msg)
            }
        }

        "promote" => {
            if pos.is_empty() || flags.get("to").is_none() {
                return bad("usage: promote <id|@handle> --to <board>".into());
            }
            let Some(c) = core::find_card(&store.data.cards, &pos[0]) else {
                return bad(format!("no card {}", pos[0]));
            };
            let id = core::strf(c, "id").unwrap_or("").to_string();
            let to = flag_or(&flags, "to", "");
            finish!(json!({ "action": "promote_card", "id": id, "board": to }))
        }

        "reset" => {
            store.reset();
            ok("store reseeded from the specification".into(), true)
        }

        "skills" => {
            let (skills_list, warnings) = skills::list_skills(paths);
            let mut lines: Vec<String> = skills_list
                .iter()
                .map(|s| {
                    let d = &s.description;
                    if d.chars().count() > 80 {
                        format!("{} — {}…", s.id, d.chars().take(80).collect::<String>())
                    } else {
                        format!("{} — {}", s.id, d)
                    }
                })
                .collect();
            for w in warnings {
                lines.push(format!("⚠ {}", w));
            }
            if lines.is_empty() {
                ok("(no skills — data/skills/ is empty)".into(), false)
            } else {
                ok(lines.join("\n"), false)
            }
        }

        "skill" => {
            let sub = pos.first().map(String::as_str).unwrap_or("");
            let run = || -> Result<CliResult, String> {
                match sub {
                    "show" => {
                        let (skills_list, _) = skills::list_skills(paths);
                        let hit = pos.get(1).and_then(|q| {
                            skills_list.iter().find(|s| &s.id == q || &s.name == q)
                        });
                        let Some(hit) = hit else {
                            return Ok(bad(format!(
                                "no skill {}",
                                pos.get(1).cloned().unwrap_or_else(|| "(missing id)".into())
                            )));
                        };
                        let s = skills::read_skill(paths, &hit.id)?;
                        Ok(ok(format!("# {} ({})\n{}\n\n{}", s.name, s.id, s.description, s.content), false))
                    }
                    "create" => {
                        if pos.get(1).is_none() {
                            return Ok(bad(
                                "usage: skill create <id> --description \"…\" [--content \"…\"]".into(),
                            ));
                        }
                        let s = skills::create_skill(
                            paths,
                            &json!({
                                "name": pos[1],
                                "description": flag_str(&flags, "description").unwrap_or(""),
                                "content": flag_str(&flags, "content").unwrap_or(""),
                            }),
                        )?;
                        Ok(ok(format!("skill created: data/skills/{}/SKILL.md", s.id), true))
                    }
                    "edit" => {
                        let (skills_list, _) = skills::list_skills(paths);
                        let hit = pos.get(1).and_then(|q| {
                            skills_list.iter().find(|s| &s.id == q || &s.name == q)
                        });
                        let Some(hit) = hit else {
                            return Ok(bad(format!(
                                "no skill {}",
                                pos.get(1).cloned().unwrap_or_else(|| "(missing id)".into())
                            )));
                        };
                        let mut patch = Map::new();
                        let allowed = ["name", "description", "content"];
                        for kv in &pos[2..] {
                            let Some(eq) = kv.find('=') else {
                                return Ok(bad(format!(
                                    "skill edit: expected key=value, got {}",
                                    js_json_str(kv)
                                )));
                            };
                            let k = &kv[..eq];
                            if !allowed.contains(&k) {
                                return Ok(bad(format!(
                                    "skill edit: key must be {}",
                                    allowed.join("|")
                                )));
                            }
                            patch.insert(k.to_string(), json!(kv[eq + 1..]));
                        }
                        if patch.is_empty() {
                            return Ok(bad("skill edit: nothing to change".into()));
                        }
                        let s = skills::update_skill(paths, &hit.id, &Value::Object(patch))?;
                        Ok(ok(format!("skill updated: {}", s.id), true))
                    }
                    "delete" => {
                        let (skills_list, _) = skills::list_skills(paths);
                        let hit = pos.get(1).and_then(|q| {
                            skills_list.iter().find(|s| &s.id == q || &s.name == q)
                        });
                        let Some(hit) = hit else {
                            return Ok(bad(format!(
                                "no skill {}",
                                pos.get(1).cloned().unwrap_or_else(|| "(missing id)".into())
                            )));
                        };
                        skills::delete_skill(paths, &hit.id)?;
                        Ok(ok(format!("skill deleted: {}", hit.id), true))
                    }
                    _ => Ok(bad("skill: expected show|create|edit|delete".into())),
                }
            };
            match run() {
                Ok(r) => r,
                Err(e) => bad(e),
            }
        }

        _ => bad(format!("unknown command {} — try `help`", js_json_str(&cmd))),
    }
}
