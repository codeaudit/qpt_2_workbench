//! Port of qpt-core.js — the shared domain core.
//!
//! Single source of truth for the specification's mechanics: gate math,
//! movement semantics (A13, A9, §14, A7), trace, Γ EMA, promotion, card
//! shapes, patch clamping, action execution. Cards are heterogeneous JSON
//! objects, exactly as the JS core treats them; board structure comes from
//! `data::boards()`.
//!
//! All functions are free functions (the JS `createCore(DATA)` closure is
//! replaced by the statically embedded boards data).

use serde_json::{json, Map, Value};

use crate::data::{boards, Board};
use crate::store::StoreData;

pub const SIGMA_A: f64 = 10.0; // sigmoid sharpness `a` (§8.1)

/// State shape expected throughout (JS: { boardId, cards: {id: card}, customSeq }).
/// The store's data IS the state: actions apply in place, exactly as the JS
/// core mutates `store.data` — no per-action clone of the cards map.
pub type State = StoreData;

/* ------------------------------------------------------------ helpers */

pub fn strf<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

pub fn numf(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

pub fn is_null(v: &Value, key: &str) -> bool {
    v.get(key).map(Value::is_null).unwrap_or(true)
}

/// JS `String(val)` for the value kinds that can reach sanitizePatch.
fn js_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => js_num_str(n.as_f64().unwrap_or(0.0)),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// JS number→string (shortest round-trip, integers without ".0").
pub fn js_num_str(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

/* ------------------------------------------------------------ basics */

pub fn board_of(board_id: &str) -> Option<&'static Board> {
    boards().iter().find(|b| b.id == board_id)
}

fn board_of_state(state: &State) -> Option<&'static Board> {
    board_of(&state.board_id)
}

pub fn col_index(state: &State, id: &str) -> i64 {
    board_of_state(state)
        .and_then(|b| b.columns.iter().position(|c| c.id == id))
        .map(|i| i as i64)
        .unwrap_or(-1)
}

pub fn col_name(board_id: &str, col_id: &str) -> String {
    board_of(board_id)
        .and_then(|b| b.columns.iter().find(|c| c.id == col_id))
        .map(|c| c.name.clone())
        .unwrap_or_else(|| col_id.to_string())
}

pub fn cards_on<'a>(state: &'a State, board_id: &str) -> Vec<&'a Value> {
    state
        .cards
        .values()
        .filter(|c| strf(c, "board") == Some(board_id))
        .collect()
}

/* ------------------------------------------------------ gate (§7–8) */

pub fn score_of(c: &Value) -> Option<f64> {
    let rho = numf(c, "rho")?;
    Some(rho - numf(c, "delta")? - numf(c, "gamma")? * numf(c, "k")?.ln())
}

pub fn zone_of(c: &Value) -> Option<i64> {
    let s = score_of(c)?;
    let theta = numf(c, "theta")?;
    let d = s - theta;
    if d.abs() <= 0.15 {
        Some(2)
    } else if d < 0.0 {
        Some(1)
    } else {
        Some(3)
    }
}

pub fn sigmoid_p(c: &Value) -> Option<f64> {
    let s = score_of(c)?;
    let theta = numf(c, "theta")?;
    Some(1.0 / (1.0 + (-SIGMA_A * (s - theta)).exp()))
}

#[derive(Clone, Debug)]
pub struct Eval {
    pub r: bool,
    pub g: bool,
    pub quant: bool,
    pub s: f64,
    pub key: &'static str,
    pub label: &'static str,
    pub sub: &'static str,
    pub living: bool,
}

impl Eval {
    pub fn to_json(&self) -> Value {
        json!({
            "R": self.r,
            "G": self.g,
            "quant": self.quant,
            "S": self.s,
            "key": self.key,
            "label": self.label,
            "sub": self.sub,
            "living": self.living,
        })
    }
}

pub fn evaluate(c: &Value) -> Option<Eval> {
    let source = strf(c, "source")?;
    let s = score_of(c)?;
    let theta = numf(c, "theta")?;
    let rc = source == "recruited";
    let gc = strf(c, "target") == Some("grounded");
    let quant = s >= theta;
    let (key, label, sub) = if rc && gc && quant {
        ("living", "Living", "generative act — dual passage")
    } else if rc && gc {
        ("threshold", "At threshold", "Zone 2 — unstable; stay with the encounter")
    } else if rc {
        ("delusion", "Beautiful delusion", "R without G — inspired, ungrounded")
    } else if gc {
        ("competent", "Competent-dead", "G without R — grounded, no vitality")
    } else {
        ("dead", "Fully dead", "administrative process — {△} → {△}")
    };
    Some(Eval { r: rc, g: gc, quant, s, key, label, sub, living: key == "living" })
}

pub fn naming_class(c: &Value) -> Value {
    let r = strf(c, "source") == Some("recruited");
    let g = strf(c, "target") == Some("grounded");
    let (key, label) = if r && g {
        ("living", "Living name")
    } else if r {
        ("delusion", "Poetic capture")
    } else if g {
        ("competent", "Technical term")
    } else {
        ("dead", "Jargon")
    };
    json!({ "key": key, "label": label })
}

pub fn weights(state: &State) -> Map<String, Value> {
    const EPS0: f64 = 0.05;
    let with_rel: Vec<&Value> = cards_on(state, &state.board_id)
        .into_iter()
        .filter(|c| !is_null(c, "reliability"))
        .collect();
    let sum: f64 = with_rel.iter().map(|c| numf(c, "reliability").unwrap_or(0.0) + EPS0).sum();
    let mut map = Map::new();
    for c in with_rel {
        if let Some(id) = strf(c, "id") {
            let w = (numf(c, "reliability").unwrap_or(0.0) + EPS0) / sum;
            map.insert(id.to_string(), json!(w));
        }
    }
    map
}

/* ------------------------------------------------------- trace (§9.2) */

pub fn push_trace(c: &mut Value, mut entry: Map<String, Value>) {
    if c.get("trace").and_then(Value::as_array).is_none() {
        c["trace"] = json!([]);
    }
    let trace = c["trace"].as_array_mut().unwrap();
    entry.insert("n".into(), json!(trace.len() + 1));
    trace.push(Value::Object(entry));
    if trace.len() > 50 {
        let start = trace.len() - 50;
        trace.drain(0..start);
    }
}

fn trace_entry(action: &str, from: &str, to: &str, note: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("action".into(), json!(action));
    m.insert("from".into(), json!(from));
    m.insert("to".into(), json!(to));
    m.insert("note".into(), json!(note));
    m
}

pub fn on_transition(state: &State, c: &mut Value, from: &str, to: &str) {
    let mut note = String::new();
    let board = board_of_state(state);
    let cols: &[_] = board.map(|b| b.columns.as_slice()).unwrap_or(&[]);
    let fi = cols.iter().position(|x| x.id == from).map(|i| i as i64).unwrap_or(-1);
    let ti = cols.iter().position(|x| x.id == to).map(|i| i as i64).unwrap_or(-1);
    let ev = evaluate(c);
    if to == "gate" {
        if let Some(ev) = &ev {
            let theta = numf(c, "theta").unwrap_or(0.0);
            note = format!(
                "evaluated at the gate: {} — S {:.2}{} θ {:.2}",
                ev.label,
                ev.s,
                if ev.quant { " ≥" } else { " <" },
                theta
            );
        }
    }
    if from == "gate" && to == "articulation" {
        if let Some(ev) = &ev {
            let theta = numf(c, "theta").unwrap_or(0.0);
            note = format!(
                "dual passage (A9): S {:.2} ≥ θ {:.2} — proceeds to naming downstream of passage",
                ev.s, theta
            );
        }
    }
    if strf(c, "board") == Some("dialectic") && ti == fi + 1 {
        let step = |i: i64| {
            cols.get(i as usize)
                .and_then(|c| c.step.clone())
                .unwrap_or_default()
        };
        let mut parts = vec![format!("χ {} → {} (schedule →|π)", step(fi), step(ti))];
        if !is_null(c, "reliability") {
            let old = numf(c, "reliability").unwrap_or(0.0);
            let new = ((0.7 * old + 0.3) * 100.0).round() / 100.0;
            c["reliability"] = json!(new);
            parts.push(format!("Γ {:.2} → {:.2} (EMA λ = 0.7, §19)", old, new));
        }
        note = parts.join("; ");
    }
    let card_board = strf(c, "board").unwrap_or("").to_string();
    let from_name = col_name(&card_board, from);
    let to_name = col_name(&card_board, to);
    push_trace(c, trace_entry("passage", &from_name, &to_name, &note));
}

pub fn apply_move(state: &State, c: &mut Value, to_id: &str) {
    let from = strf(c, "column").unwrap_or("").to_string();
    c["column"] = json!(to_id);
    on_transition(state, c, &from, to_id);
}

/* ------------------------------------------------ movement semantics */

pub struct MoveCheck {
    pub ok: bool,
    pub msg: Option<String>,
    pub focus: Option<String>,
}

impl MoveCheck {
    fn ok() -> MoveCheck {
        MoveCheck { ok: true, msg: None, focus: None }
    }
    fn refuse(msg: String, focus: Option<&str>) -> MoveCheck {
        MoveCheck { ok: false, msg: Some(msg), focus: focus.map(str::to_string) }
    }
}

pub fn can_move(state: &State, c: &Value, to_id: &str) -> MoveCheck {
    let from_col = strf(c, "column").unwrap_or("");
    let from = col_index(state, from_col);
    let to = col_index(state, to_id);
    if to < 0 || to == from {
        return MoveCheck::ok();
    }
    if to > from + 1 {
        return MoveCheck::refuse(
            "A13 · Commitment horizon = 1 — the option space past the next gate passage does not yet exist. Advance one column at a time.".into(),
            None,
        );
    }
    if state.board_id == "protocol" {
        let ev = evaluate(c);
        if from_col == "gate" && to_id == "articulation" {
            if let Some(ev) = &ev {
                if !ev.living {
                    return MoveCheck::refuse(
                        format!(
                            "A9 · The gate refused: R ⊓ G does not hold here ({}). {{△}}-articulation before dual passage is dead naming.",
                            ev.label
                        ),
                        Some("gate"),
                    );
                }
            }
        }
        if from_col == "gate" && to < from {
            if let Some(ev) = &ev {
                if !ev.living && is_null(c, "pathology") {
                    return MoveCheck::refuse(
                        format!(
                            "§14 · Gate failed ({}). Diagnose via the five-layer model before returning to step 1.",
                            ev.label
                        ),
                        Some("pathology"),
                    );
                }
            }
        }
    }
    if state.board_id == "dialectic" && to_id == "synthesize" && strf(c, "kind") != Some("note") {
        let genesis_ok = strf(c, "genesis").map(|g| !g.trim().is_empty()).unwrap_or(false);
        if !genesis_ok {
            return MoveCheck::refuse(
                "A7 · Genesis requires an emergent property present in neither parent — declare it before synthesis.".into(),
                Some("genesis"),
            );
        }
    }
    MoveCheck::ok()
}

/* ------------------------------------------------------------ agents */

const SUB: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];

fn agent_num(a: &str) -> Option<i64> {
    if let Some(rest) = a.strip_prefix('u') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return rest.parse::<i64>().ok();
        }
        if !rest.is_empty() && rest.chars().all(|c| SUB.contains(&c)) {
            let digits: String = rest
                .chars()
                .map(|c| (SUB.iter().position(|s| *s == c).unwrap() as u8 + b'0') as char)
                .collect();
            return digits.parse::<i64>().ok();
        }
    }
    None
}

pub fn next_agent(state: &State, board_id: &str) -> String {
    let used: Vec<i64> = cards_on(state, board_id)
        .iter()
        .filter_map(|c| strf(c, "agent"))
        .filter_map(agent_num)
        .collect();
    let n = used.iter().max().copied().unwrap_or(0) + 1;
    let sub: String = n
        .to_string()
        .chars()
        .map(|d| SUB[d.to_digit(10).unwrap() as usize])
        .collect();
    format!("u{}", sub)
}

/* ------------------------------------------------------------ cards */

pub fn build_card(state: &State, board_id: &str, id: &str, fields: &Value) -> Option<Value> {
    let b = board_of(board_id)?;
    if strf(fields, "title").is_none() && fields.get("title").is_none() {
        return None;
    }
    let col = b.columns.first()?.id.clone();
    let title: String = js_string(fields.get("title").unwrap()).chars().take(200).collect();
    let note = strf(fields, "note");

    if board_id == "protocol" {
        let source = if strf(fields, "source") == Some("initiated") { "initiated" } else { "recruited" };
        let target = if strf(fields, "target") == Some("terminated") { "terminated" } else { "grounded" };
        let (sign, sign_name) = if source == "recruited" {
            ("⟨⦿○ ≈ ⊣○⟩", "Qualisign–Icon–Rheme")
        } else {
            ("⟨⦿△ ≡ ⊣△⟩", "Legisign–Symbol–Argument")
        };
        let default_note = "Practitioner-entered transformation. Metrics are initial estimates — revise them after [□]-encounter.";
        return Some(json!({
            "id": id,
            "board": board_id,
            "column": col,
            "title": title,
            "sign": sign,
            "signName": sign_name,
            "source": source,
            "target": target,
            "rho": 0.80,
            "delta": 0.10,
            "gamma": 0.25,
            "k": 3,
            "theta": 0.40,
            "scale": "meso",
            "cycle": 0,
            "pathology": Value::Null,
            "death": Value::Null,
            "axioms": ["A9"],
            "note": note.unwrap_or(default_note),
        }));
    }

    if board_id == "dialectic" {
        if strf(fields, "kind") == Some("note") {
            return Some(json!({
                "id": id,
                "board": board_id,
                "column": col,
                "kind": "note",
                "title": title,
                "sign": "⟹ᵐⁿᵃᵛ¹ ⊗ |ᵍ ⊗ ⟹ᵐᵉᵐ",
                "signName": "moderator note",
                "reliability": Value::Null,
                "cycle": 0,
                "axioms": ["A16"],
                "note": note.unwrap_or("Practitioner-entered note."),
            }));
        }
        let rel = match fields.get("reliability").and_then(Value::as_f64) {
            Some(r) => r.clamp(0.0, 1.0),
            None => 0.70,
        };
        let agent = next_agent(state, board_id);
        let default_note = format!(
            "Practitioner-entered position. Γ starts at {:.2}; the moderator updates it by EMA as scores arrive (§19), and w is recomputed across the board.",
            rel
        );
        return Some(json!({
            "id": id,
            "board": board_id,
            "column": col,
            "kind": "position",
            "agent": agent,
            "title": title,
            "sign": "⟨⦿△ ≡ ⊣α⟩",
            "signName": "Legisign–Symbol–Abduction",
            "reliability": rel,
            "cycle": 0,
            "genesis": Value::Null,
            "axioms": ["A19"],
            "note": note.unwrap_or(&default_note),
        }));
    }

    if board_id == "resolution" {
        let tags: Vec<Value> = match fields.get("tags").and_then(Value::as_array) {
            Some(arr) => arr
                .iter()
                .filter(|t| t.is_string())
                .take(8)
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        return Some(json!({
            "id": id,
            "board": board_id,
            "column": col,
            "title": title,
            "sign": "⟨⦿□ ⇢ ⊣□⟩",
            "signName": "Sinsign–Index–Dicisign",
            "cycle": 0,
            "tags": tags,
            "axioms": ["ML"],
            "note": note.unwrap_or("Practitioner-entered intervention."),
        }));
    }

    None
}

enum PatchRule {
    Str,
    StrNull,
    Num01,
    K,
    Enum(Vec<Option<&'static str>>),
}

fn editable_rule(key: &str) -> Option<PatchRule> {
    use PatchRule::*;
    Some(match key {
        "title" | "note" | "sign" | "signName" | "agent" => Str,
        "genesis" => StrNull,
        "rho" | "delta" | "gamma" | "theta" | "reliability" => Num01,
        "k" => K,
        "source" => Enum(vec![Some("recruited"), Some("initiated")]),
        "target" => Enum(vec![Some("grounded"), Some("terminated")]),
        "scale" => Enum(vec![Some("micro"), Some("meso"), Some("macro")]),
        "pathology" => Enum(vec![
            Some("structural"),
            Some("attentional"),
            Some("content"),
            Some("scalar"),
            Some("temporal"),
            None,
        ]),
        "death" => Enum(vec![Some("fossil"), Some("residue"), Some("imposition"), None]),
        _ => return None,
    })
}

pub fn sanitize_patch(patch: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let obj = match patch.as_object() {
        Some(o) => o,
        None => return out,
    };
    for (key, val) in obj {
        if key == "tags" {
            if let Some(arr) = val.as_array() {
                let tags: Vec<Value> =
                    arr.iter().filter(|t| t.is_string()).take(8).cloned().collect();
                out.insert("tags".into(), Value::Array(tags));
            }
            continue;
        }
        let rule = match editable_rule(key) {
            Some(r) => r,
            None => continue,
        };
        match rule {
            PatchRule::Str => {
                let limit = if key == "note" { 2000 } else { 200 };
                let s: String = js_string(val).chars().take(limit).collect();
                out.insert(key.clone(), json!(s));
            }
            PatchRule::StrNull => {
                if val.is_null() || val.as_str() == Some("") {
                    out.insert(key.clone(), Value::Null);
                } else {
                    let s: String = js_string(val).chars().take(200).collect();
                    out.insert(key.clone(), json!(s));
                }
            }
            PatchRule::Num01 => {
                let n = match val {
                    Value::Number(_) => val.as_f64(),
                    Value::String(s) => crate::js_parse_float(s),
                    _ => None,
                };
                if let Some(n) = n {
                    out.insert(key.clone(), json!(n.clamp(0.0, 1.0)));
                }
            }
            PatchRule::K => {
                let n = match val {
                    Value::Number(_) => val.as_f64().map(|f| f as i64),
                    Value::String(s) => crate::js_parse_int(s),
                    _ => None,
                };
                if let Some(n) = n {
                    out.insert(key.clone(), json!(n.clamp(1, 16)));
                }
            }
            PatchRule::Enum(options) => {
                let matched = options.iter().any(|opt| match (opt, val) {
                    (Some(o), Value::String(s)) => o == s,
                    (None, Value::Null) => true,
                    _ => false,
                });
                if matched {
                    out.insert(key.clone(), val.clone());
                }
            }
        }
    }
    out
}

/* --------------------------------------------------------- promotion */

fn strip_gate_fields(c: &mut Value) {
    if let Some(obj) = c.as_object_mut() {
        for k in ["source", "target", "rho", "delta", "gamma", "k", "theta", "scale"] {
            obj.remove(k);
        }
        obj.insert("pathology".into(), Value::Null);
        obj.insert("death".into(), Value::Null);
    }
}

fn strip_dialectic_fields(c: &mut Value) {
    if let Some(obj) = c.as_object_mut() {
        for k in ["kind", "agent", "reliability", "genesis"] {
            obj.remove(k);
        }
    }
}

fn add_axioms(c: &mut Value, ids: &[&str]) {
    let mut axioms: Vec<Value> = c
        .get("axioms")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for id in ids {
        let v = json!(id);
        if !axioms.contains(&v) {
            axioms.push(v);
        }
    }
    c["axioms"] = Value::Array(axioms);
}

pub struct PromotionTarget {
    pub board: &'static str,
    pub column: &'static str,
    pub label: &'static str,
    pub why: &'static str,
}

pub fn promotion_targets(c: &Value) -> Vec<PromotionTarget> {
    let mut t = Vec::new();
    let board = strf(c, "board").unwrap_or("");
    let column = strf(c, "column").unwrap_or("");
    if board == "protocol" && column == "closure" {
        t.push(PromotionTarget {
            board: "dialectic",
            column: "explore",
            label: "⇧ Promote to the Scheduled Dialectic",
            why: "§15 · System-2 = coordinated(System-1): a resolved structure whose question is now contested enters coordination as a position in Explore.",
        });
    }
    if board == "dialectic" && column == "synthesize" {
        t.push(PromotionTarget {
            board: "protocol",
            column: "initiation",
            label: "⇧ Ground the constructor in the Generative Protocol",
            why: "§7.4 · The gate applies to everything: an emergent constructor is not automatically living — it must pass 𝒢 like any transformation.",
        });
    }
    if board == "protocol" && !is_null(c, "death") {
        t.push(PromotionTarget {
            board: "resolution",
            column: "phase0",
            label: "⇧ Escalate to the Resolution Procedure",
            why: "§11 / §29 · Dead structure is failure-field material: it is not forced through the gate but mapped, excavated, and redesigned.",
        });
    }
    if board == "resolution" && column == "phase5" {
        t.push(PromotionTarget {
            board: "protocol",
            column: "initiation",
            label: "⇧ Return to living process",
            why: "§29 · Evolutionary iteration closes the loop: the redesigned structure re-enters the Generative Protocol at Initiation.",
        });
    }
    t
}

/// Pure promotion: mutates the card; returns the result object ({ok, label, why}
/// or {ok:false, message}), mirroring the JS return value.
pub fn promote(state: &State, c: &mut Value, target_board: &str) -> Value {
    let targets = promotion_targets(c);
    let t = match targets.into_iter().find(|x| x.board == target_board) {
        Some(t) => t,
        None => {
            return json!({
                "ok": false,
                "message": format!(
                    "no promotion path {}/{} → {}",
                    strf(c, "board").unwrap_or(""),
                    strf(c, "column").unwrap_or(""),
                    target_board
                )
            })
        }
    };
    let from_board = strf(c, "board").unwrap_or("").to_string();
    let from_column = strf(c, "column").unwrap_or("").to_string();

    if t.board == "dialectic" {
        strip_gate_fields(c);
        if let Some(obj) = c.as_object_mut() {
            obj.remove("tags");
        }
        c["kind"] = json!("position");
        c["agent"] = json!(next_agent(state, "dialectic"));
        c["reliability"] = json!(0.70);
        c["genesis"] = Value::Null;
        c["sign"] = json!("⟨⦿△ ≡ ⊣α⟩");
        c["signName"] = json!("Legisign–Symbol–Abduction");
        add_axioms(c, &["A19", "A20"]);
    } else if t.board == "protocol" {
        strip_dialectic_fields(c);
        if let Some(obj) = c.as_object_mut() {
            obj.remove("tags");
        }
        c["source"] = json!("recruited");
        c["target"] = json!("grounded");
        c["rho"] = json!(0.80);
        c["delta"] = json!(0.10);
        c["gamma"] = json!(0.25);
        c["k"] = json!(3);
        c["theta"] = json!(0.40);
        c["scale"] = json!("meso");
        c["pathology"] = Value::Null;
        c["death"] = Value::Null;
        c["sign"] = json!("⟨⦿□ ⇢ ⊣□⟩");
        c["signName"] = json!("Sinsign–Index–Dicisign");
        add_axioms(c, &["A9"]);
    } else {
        let was_death = c.get("death").cloned();
        let was_path = c.get("pathology").cloned();
        strip_gate_fields(c);
        strip_dialectic_fields(c);
        let mut tags: Vec<Value> = Vec::new();
        if let Some(d) = was_death.as_ref().and_then(Value::as_str) {
            tags.push(json!(format!("† {}", d)));
        }
        if let Some(p) = was_path.as_ref().and_then(Value::as_str) {
            tags.push(json!(format!("{} layer", p)));
        }
        if tags.is_empty() {
            tags.push(json!("from the protocol"));
        }
        c["tags"] = Value::Array(tags);
        c["pathology"] = Value::Null;
        c["death"] = Value::Null;
        c["sign"] = json!("⟨⦿□ ⇢ ⊣□⟩");
        c["signName"] = json!("Sinsign–Index–Dicisign");
        add_axioms(c, &["ML"]);
    }
    c["origin"] = json!({ "board": from_board.clone(), "column": from_column.clone() });
    let from_label = format!(
        "{} · {}",
        board_of(&from_board).map(|b| b.title.as_str()).unwrap_or(""),
        col_name(&from_board, &from_column)
    );
    let to_label = format!(
        "{} · {}",
        board_of(t.board).map(|b| b.title.as_str()).unwrap_or(""),
        col_name(t.board, t.column)
    );
    push_trace(c, trace_entry("⇧ promote", &from_label, &to_label, t.why));
    c["board"] = json!(t.board);
    c["column"] = json!(t.column);
    json!({ "ok": true, "label": t.label, "why": t.why, "column": t.column })
}

/* -------------------------------------------------- action execution */

/// The canonical action vocabulary — executed identically by browser and server.
pub fn apply_action(state: &mut State, a: &Value) -> Value {
    let action = strf(a, "action").unwrap_or("");
    match action {
        "set_board" => {
            let board = strf(a, "board").unwrap_or("");
            if board_of(board).is_none() {
                return json!({ "ok": false, "message": format!("unknown board {}", board) });
            }
            state.board_id = board.to_string();
            json!({ "ok": true, "message": format!("board → {}", board) })
        }
        "create_card" => {
            let id = format!("custom-{}", state.custom_seq);
            state.custom_seq += 1;
            let board = strf(a, "board").unwrap_or("");
            let card = match build_card(state, board, &id, a) {
                Some(c) => c,
                None => {
                    return json!({ "ok": false, "message": format!("cannot create card on board {}", board) })
                }
            };
            state.cards.insert(id.clone(), card);
            ensure_handles(&mut state.cards);
            let col = strf(&state.cards[&id], "column").unwrap_or("").to_string();
            let via = strf(a, "via").unwrap_or("the workbench");
            let to_name = col_name(board, &col);
            if let Some(c) = state.cards.get_mut(&id) {
                push_trace(
                    c,
                    trace_entry(
                        "enters the workflow",
                        "—",
                        &to_name,
                        &format!("created via {}", via),
                    ),
                );
            }
            state.board_id = board.to_string();
            let title = strf(&state.cards[&id], "title").unwrap_or("").to_string();
            json!({ "ok": true, "message": format!("created “{}” ({})", title, id), "id": id })
        }
        "move_card" => {
            let id = strf(a, "id").unwrap_or("");
            if !state.cards.contains_key(id) {
                return json!({ "ok": false, "message": format!("no card {}", id) });
            }
            let mut c = state.cards[id].clone();
            let card_board = strf(&c, "board").unwrap_or("").to_string();
            if card_board != state.board_id {
                state.board_id = card_board;
            }
            let to = strf(a, "column").unwrap_or("");
            let chk = can_move(state, &c, to);
            if !chk.ok {
                let mut out = Map::new();
                out.insert("ok".into(), json!(false));
                out.insert("message".into(), json!(chk.msg.unwrap_or_default()));
                if let Some(f) = chk.focus {
                    out.insert("focus".into(), json!(f));
                }
                return Value::Object(out);
            }
            apply_move(state, &mut c, to);
            state.cards.insert(id.to_string(), c);
            json!({ "ok": true, "message": format!("{} → {}", id, to) })
        }
        "edit_card" => {
            let id = strf(a, "id").unwrap_or("");
            if !state.cards.contains_key(id) {
                return json!({ "ok": false, "message": format!("no card {}", id) });
            }
            let patch = sanitize_patch(a.get("patch").unwrap_or(&Value::Null));
            if patch.is_empty() {
                return json!({ "ok": false, "message": "no editable fields in patch" });
            }
            let keys: Vec<String> = patch.keys().cloned().collect();
            let c = state.cards.get_mut(id).unwrap();
            if let Some(obj) = c.as_object_mut() {
                for (k, v) in patch {
                    obj.insert(k, v);
                }
            }
            let card_board = strf(c, "board").unwrap_or("").to_string();
            if card_board != state.board_id {
                state.board_id = card_board;
            }
            json!({ "ok": true, "message": format!("edited {} ({})", id, keys.join(", ")) })
        }
        "promote_card" => {
            let id = strf(a, "id").unwrap_or("");
            if !state.cards.contains_key(id) {
                return json!({ "ok": false, "message": format!("no card {}", id) });
            }
            let mut c = state.cards[id].clone();
            let card_board = strf(&c, "board").unwrap_or("").to_string();
            if card_board != state.board_id {
                state.board_id = card_board;
            }
            let to_board = strf(a, "board").unwrap_or("");
            let r = promote(state, &mut c, to_board);
            if r.get("ok") != Some(&json!(true)) {
                return r;
            }
            let new_board = strf(&c, "board").unwrap_or("").to_string();
            state.cards.insert(id.to_string(), c);
            state.board_id = new_board;
            json!({
                "ok": true,
                "message": r.get("label").cloned().unwrap_or(Value::Null),
                "why": r.get("why").cloned().unwrap_or(Value::Null),
            })
        }
        "evaluate_card" => {
            let id = strf(a, "id").unwrap_or("");
            let c = match state.cards.get(id) {
                Some(c) => c,
                None => return json!({ "ok": false, "message": format!("no card {}", id) }),
            };
            match evaluate(c) {
                Some(ev) => json!({
                    "ok": true,
                    "message": format!("{} (S {:.3})", ev.label, ev.s),
                    "result": ev.to_json(),
                }),
                None => json!({
                    "ok": true,
                    "message": "no gate fields on this card",
                    "result": Value::Null,
                }),
            }
        }
        _ => json!({ "ok": false, "message": format!("unknown action {}", action) }),
    }
}

/* ------------------------------------------------------------- misc */

// Stable CamelCase handles for referencing (@OnboardingDropoff). Assigned
// once at creation/seed/migration, deduped, and never re-derived from
// renames — references must not break when a title changes.
const HANDLE_STOP: [&str; 4] = ["the", "a", "an", "here"];

pub fn to_camel_case(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c.is_whitespace() { c } else { ' ' })
        .collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    while words.len() > 1 && HANDLE_STOP.contains(&words[0].to_lowercase().as_str()) {
        words.remove(0);
    }
    words.truncate(5);
    let cc: String = words
        .iter()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if cc.is_empty() {
        "Card".into()
    } else {
        cc
    }
}

/// Ensure every card in the map has a unique handle; mutates in place.
/// Returns true when anything changed.
pub fn ensure_handles(cards: &mut Map<String, Value>) -> bool {
    // Fast path (creation onto an already-handled map): skip rebuilding the
    // taken-set when no card needs a handle.
    if cards.values().all(|c| strf(c, "handle").is_some()) {
        return false;
    }
    let mut taken: std::collections::HashSet<String> = cards
        .values()
        .filter_map(|c| strf(c, "handle"))
        .map(|h| h.to_lowercase())
        .collect();
    let mut changed = false;
    let ids: Vec<String> = cards.keys().cloned().collect();
    for id in ids {
        let c = cards.get_mut(&id).unwrap();
        if strf(c, "handle").is_some() {
            continue;
        }
        let title = strf(c, "title").unwrap_or(&id).to_string();
        let base = to_camel_case(&title);
        let mut h = base.clone();
        let mut n = 2;
        while taken.contains(&h.to_lowercase()) {
            h = format!("{}{}", base, n);
            n += 1;
        }
        taken.insert(h.to_lowercase());
        c["handle"] = json!(h);
        changed = true;
    }
    changed
}

/// Resolve a reference: id, @handle, handle, exact title, or title substring.
/// Case-insensitive compares are allocation-free (ASCII-insensitive).
pub fn find_card<'a>(cards: &'a Map<String, Value>, reference: &str) -> Option<&'a Value> {
    let key = reference.strip_prefix('@').unwrap_or(reference);
    if let Some(c) = cards.get(key) {
        return Some(c);
    }
    let ci_contains = |hay: &str| {
        let (h, n) = (hay.as_bytes(), key.as_bytes());
        n.is_empty() || (n.len() <= h.len() && h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n)))
    };
    cards
        .values()
        .find(|c| strf(c, "handle").map(|h| h.eq_ignore_ascii_case(key)).unwrap_or(false))
        .or_else(|| {
            cards
                .values()
                .find(|c| strf(c, "title").map(|t| t.eq_ignore_ascii_case(key)).unwrap_or(false))
        })
        .or_else(|| {
            cards
                .values()
                .find(|c| strf(c, "title").map(&ci_contains).unwrap_or(false))
        })
}

pub fn compact_state(state: &State) -> Value {
    let boards_json: Vec<Value> = boards()
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "columns": b.columns.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let cards_json: Vec<Value> = state
        .cards
        .values()
        .map(|c| {
            let mut out = Map::new();
            for k in ["id", "handle", "board", "column", "title", "sign"] {
                if let Some(v) = c.get(k) {
                    out.insert(k.into(), v.clone());
                }
            }
            out.insert("cycle".into(), c.get("cycle").cloned().unwrap_or(json!(0)));
            if !is_null(c, "source") {
                let ev = evaluate(c);
                for k in ["source", "target", "rho", "delta", "gamma", "k", "theta"] {
                    if let Some(v) = c.get(k) {
                        out.insert(k.into(), v.clone());
                    }
                }
                if let Some(s) = score_of(c) {
                    let rounded: f64 = format!("{:.3}", s).parse().unwrap_or(s);
                    out.insert("S".into(), json!(rounded));
                }
                out.insert(
                    "verdict".into(),
                    ev.map(|e| json!(e.key)).unwrap_or(Value::Null),
                );
            }
            if !is_null(c, "reliability") {
                if let Some(v) = c.get("reliability") {
                    out.insert("reliability".into(), v.clone());
                }
            }
            for k in ["kind", "agent"] {
                if let Some(v) = c.get(k) {
                    if !v.is_null() {
                        out.insert(k.into(), v.clone());
                    }
                }
            }
            for k in ["genesis", "pathology", "death"] {
                if let Some(s) = strf(c, k) {
                    if !s.is_empty() {
                        out.insert(k.into(), json!(s));
                    }
                }
            }
            if let Some(tags) = c.get("tags").and_then(Value::as_array) {
                if !tags.is_empty() {
                    out.insert("tags".into(), Value::Array(tags.clone()));
                }
            }
            Value::Object(out)
        })
        .collect();
    json!({
        "boardId": state.board_id,
        "boards": boards_json,
        "cards": cards_json,
    })
}

pub fn seed_cards() -> Map<String, Value> {
    let mut cards = Map::new();
    for b in boards() {
        for c in &b.cards {
            if let Some(mut obj) = c.as_object().cloned() {
                obj.insert("board".into(), json!(b.id));
                let id = strf(c, "id").unwrap_or("").to_string();
                cards.insert(id, Value::Object(obj));
            }
        }
    }
    ensure_handles(&mut cards);
    cards
}
