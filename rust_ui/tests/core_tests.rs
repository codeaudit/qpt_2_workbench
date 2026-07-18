//! Unit tests for the domain core (port of qpt-core.js semantics).

use qpt::core::*;
use serde_json::{json, Map, Value};

fn seeded() -> State {
    State { version: 0, board_id: "protocol".into(), cards: seed_cards(), custom_seq: 0 }
}

fn card(cards: &Map<String, Value>, id: &str) -> Value {
    cards.get(id).unwrap().clone()
}

/* ------------------------------------------------------------- gate math */

#[test]
fn score_zone_sigmoid() {
    let s = seeded();
    let p1 = card(&s.cards, "p1");
    // 0.95 − 0.07 − 0.2·ln 3 = 0.66027…
    let score = score_of(&p1).unwrap();
    assert!((score - 0.66028).abs() < 1e-4);
    assert_eq!(zone_of(&p1), Some(3)); // S − θ = 0.16 > 0.15

    let mut edge = p1.clone();
    edge["theta"] = json!(score); // |S − θ| = 0 ≤ 0.15
    assert_eq!(zone_of(&edge), Some(2));
    edge["theta"] = json!(score - 0.151); // just past the boundary
    assert_eq!(zone_of(&edge), Some(3));
    edge["theta"] = json!(score + 0.5);
    assert_eq!(zone_of(&edge), Some(1));

    let p = sigmoid_p(&p1).unwrap();
    assert!(p > 0.5 && p < 1.0);
    // a = 10: 1/(1+e^(−10·0.16028)) ≈ 0.833
    assert!((p - 0.8330).abs() < 1e-3);

    // no gate fields → nulls
    let d1 = card(&s.cards, "d1");
    assert_eq!(score_of(&d1), None);
    assert_eq!(zone_of(&d1), None);
    assert_eq!(sigmoid_p(&d1), None);
}

#[test]
fn evaluate_verdicts() {
    let mk = |source: &str, target: &str, s_over: f64| {
        // construct a card whose S − θ = s_over
        json!({
            "source": source, "target": target,
            "rho": 0.5 + s_over, "delta": 0.0, "gamma": 0.0, "k": 1, "theta": 0.5,
        })
    };
    let ev = evaluate(&mk("recruited", "grounded", 0.3)).unwrap();
    assert_eq!(ev.key, "living");
    assert!(ev.living && ev.quant && ev.r && ev.g);
    assert_eq!(ev.label, "Living");

    let ev = evaluate(&mk("recruited", "grounded", -0.3)).unwrap();
    assert_eq!(ev.key, "threshold");
    assert!(!ev.living);

    let ev = evaluate(&mk("recruited", "terminated", 0.3)).unwrap();
    assert_eq!(ev.key, "delusion");

    let ev = evaluate(&mk("initiated", "grounded", 0.3)).unwrap();
    assert_eq!(ev.key, "competent");

    let ev = evaluate(&mk("initiated", "terminated", 0.3)).unwrap();
    assert_eq!(ev.key, "dead");

    assert!(evaluate(&json!({"title": "no gate fields"})).is_none());
}

#[test]
fn naming_classes() {
    let nc = |s: &str, t: &str| naming_class(&json!({"source": s, "target": t}));
    assert_eq!(nc("recruited", "grounded")["key"], json!("living"));
    assert_eq!(nc("recruited", "terminated")["label"], json!("Poetic capture"));
    assert_eq!(nc("initiated", "grounded")["label"], json!("Technical term"));
    assert_eq!(nc("initiated", "terminated")["label"], json!("Jargon"));
}

/* -------------------------------------------------------- movement rules */

#[test]
fn can_move_a13_forward_skip_refused() {
    let s = seeded();
    let p1 = card(&s.cards, "p1"); // encounter
    let chk = can_move(&s, &p1, "articulation");
    assert!(!chk.ok);
    assert_eq!(
        chk.msg.unwrap(),
        "A13 · Commitment horizon = 1 — the option space past the next gate passage does not yet exist. Advance one column at a time."
    );
    // one column forward is fine
    assert!(can_move(&s, &p1, "gate").ok);
    // same column / unknown column → ok (no-op)
    assert!(can_move(&s, &p1, "encounter").ok);
    assert!(can_move(&s, &p1, "nowhere").ok);
    // backward jumps are free
    let mut moved = p1.clone();
    moved["column"] = json!("closure");
    assert!(can_move(&s, &moved, "initiation").ok);
}

#[test]
fn can_move_a9_gate_requires_living() {
    let s = seeded();
    let p2 = card(&s.cards, "p2"); // gate, dead verdict
    let chk = can_move(&s, &p2, "articulation");
    assert!(!chk.ok);
    assert_eq!(chk.focus.as_deref(), Some("gate"));
    assert_eq!(
        chk.msg.unwrap(),
        "A9 · The gate refused: R ⊓ G does not hold here (Fully dead). {△}-articulation before dual passage is dead naming."
    );
    // a living card passes
    let mut living = p2.clone();
    living["source"] = json!("recruited");
    living["target"] = json!("grounded");
    living["rho"] = json!(0.95);
    living["delta"] = json!(0.05);
    living["gamma"] = json!(0.1);
    living["theta"] = json!(0.1); // S = 0.72 ≥ θ
    assert!(can_move(&s, &living, "articulation").ok);
}

#[test]
fn can_move_section14_diagnose_before_return() {
    let s = seeded();
    let mut p2 = card(&s.cards, "p2"); // gate, dead — seed already carries a pathology
    p2["pathology"] = Value::Null;
    let chk = can_move(&s, &p2, "encounter");
    assert!(!chk.ok);
    assert_eq!(chk.focus.as_deref(), Some("pathology"));
    assert_eq!(
        chk.msg.unwrap(),
        "§14 · Gate failed (Fully dead). Diagnose via the five-layer model before returning to step 1."
    );
    // with a pathology set, the return is allowed (the seeded p2 is exactly that)
    let seeded_p2 = card(&s.cards, "p2");
    assert!(can_move(&s, &seeded_p2, "encounter").ok);
}

#[test]
fn can_move_a7_genesis_required_for_synthesize() {
    let mut s = seeded();
    s.board_id = "dialectic".into();
    let d1 = card(&s.cards, "d1"); // explore, kind position
    let mut at_consolidate = d1.clone();
    at_consolidate["column"] = json!("consolidate");
    let chk = can_move(&s, &at_consolidate, "synthesize");
    assert!(!chk.ok);
    assert_eq!(chk.focus.as_deref(), Some("genesis"));
    assert_eq!(
        chk.msg.unwrap(),
        "A7 · Genesis requires an emergent property present in neither parent — declare it before synthesis."
    );
    let mut with_genesis = at_consolidate.clone();
    with_genesis["genesis"] = json!("emergent property");
    assert!(can_move(&s, &with_genesis, "synthesize").ok);
    // moderator notes are exempt
    let mut note = at_consolidate.clone();
    note["kind"] = json!("note");
    assert!(can_move(&s, &note, "synthesize").ok);
}

/* ------------------------------------------------------ trace and EMA */

#[test]
fn dialectic_forward_move_updates_reliability_by_ema() {
    let mut s = seeded();
    s.board_id = "dialectic".into();
    let mut d1 = card(&s.cards, "d1"); // explore, reliability 0.81
    apply_move(&s, &mut d1, "integrate");
    // Γ ← 0.7·0.81 + 0.3 = 0.867 → round → 0.87
    assert_eq!(d1["reliability"], json!(0.87));
    let trace = d1["trace"].as_array().unwrap();
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0]["action"], json!("passage"));
    assert_eq!(trace[0]["n"], json!(1));
    assert!(trace[0]["note"]
        .as_str()
        .unwrap()
        .contains("Γ 0.81 → 0.87 (EMA λ = 0.7, §19)"));
}

#[test]
fn gate_entry_records_evaluation_note() {
    let s = seeded();
    let mut p1 = card(&s.cards, "p1"); // encounter → gate
    apply_move(&s, &mut p1, "gate");
    let trace = p1["trace"].as_array().unwrap();
    assert!(trace[0]["note"]
        .as_str()
        .unwrap()
        .contains("evaluated at the gate: Living — S 0.66 ≥ θ 0.50"));
}

#[test]
fn trace_is_capped_at_50() {
    let s = seeded();
    let mut p1 = card(&s.cards, "p1");
    for _ in 0..60 {
        push_trace(&mut p1, {
            let mut m = Map::new();
            m.insert("action".into(), json!("x"));
            m.insert("from".into(), json!("a"));
            m.insert("to".into(), json!("b"));
            m.insert("note".into(), json!(""));
            m
        });
    }
    let trace = p1["trace"].as_array().unwrap();
    assert_eq!(trace.len(), 50);
    // n is assigned as length+1 before the cap trim, so at the cap it stays 51
    assert_eq!(trace.last().unwrap()["n"], json!(51));
}

/* ------------------------------------------------------------- cards */

#[test]
fn build_card_shapes() {
    let s = seeded();
    let c = build_card(&s, "protocol", "x1", &json!({"title": "T", "source": "initiated", "target": "terminated"})).unwrap();
    assert_eq!(c["column"], json!("initiation"));
    assert_eq!(c["signName"], json!("Legisign–Symbol–Argument"));
    assert_eq!(c["rho"], json!(0.80));
    assert_eq!(c["k"], json!(3));
    assert_eq!(c["pathology"], Value::Null);

    let p = build_card(&s, "dialectic", "x2", &json!({"title": "P", "reliability": 1.7})).unwrap();
    assert_eq!(p["kind"], json!("position"));
    assert_eq!(p["reliability"], json!(1.0)); // clamped 0..1
    assert_eq!(p["agent"], json!("u₅")); // u₁..u₄ seeded → next is u₅

    let n = build_card(&s, "dialectic", "x3", &json!({"title": "N", "kind": "note"})).unwrap();
    assert_eq!(n["kind"], json!("note"));
    assert_eq!(n["reliability"], Value::Null);

    let r = build_card(&s, "resolution", "x4", &json!({"title": "R", "tags": ["a", "b", 3]})).unwrap();
    assert_eq!(r["tags"], json!(["a", "b"]));

    assert!(build_card(&s, "protocol", "x5", &json!({})).is_none());
    assert!(build_card(&s, "unknown", "x6", &json!({"title": "T"})).is_none());
}

#[test]
fn next_agent_uses_subscript_numerals() {
    let s = seeded();
    assert_eq!(next_agent(&s, "dialectic"), "u₅"); // u₁..u₄ exist
    let mut s2 = seeded();
    s2.cards.retain(|_, c| strf(c, "board") != Some("dialectic"));
    assert_eq!(next_agent(&s2, "dialectic"), "u₁");
}

#[test]
fn sanitize_patch_clamps_and_filters() {
    let out = sanitize_patch(&json!({
        "rho": 1.7, "delta": -2, "k": 99, "theta": "0.35",
        "source": "initiated", "target": "bogus", "scale": "macro",
        "pathology": "content", "death": "fossil",
        "genesis": "", "title": "x", "note": "y", "hack": true,
        "tags": ["a", 1, "b"],
    }));
    assert_eq!(out["rho"], json!(1.0));
    assert_eq!(out["delta"], json!(0.0));
    assert_eq!(out["k"], json!(16));
    assert_eq!(out["theta"], json!(0.35));
    assert_eq!(out["source"], json!("initiated"));
    assert!(out.get("target").is_none());
    assert_eq!(out["scale"], json!("macro"));
    assert_eq!(out["pathology"], json!("content"));
    assert_eq!(out["death"], json!("fossil"));
    assert_eq!(out["genesis"], Value::Null); // "" → null
    assert!(out.get("hack").is_none());
    assert_eq!(out["tags"], json!(["a", "b"]));

    // k clamped to ≥ 1; null assignments allowed for pathology/death
    let out = sanitize_patch(&json!({"k": 0, "pathology": Value::Null, "death": Value::Null}));
    assert_eq!(out["k"], json!(1));
    assert_eq!(out["pathology"], Value::Null);
    assert_eq!(out["death"], Value::Null);

    // title truncated to 200 chars
    let long = "x".repeat(300);
    let out = sanitize_patch(&json!({"title": long}));
    assert_eq!(out["title"].as_str().unwrap().len(), 200);
}

/* ---------------------------------------------------------- promotion */

#[test]
fn promotion_paths_and_transforms() {
    let mut s = seeded();

    // protocol closure → dialectic explore
    let mut c = card(&s.cards, "p1");
    c["column"] = json!("closure");
    let targets = promotion_targets(&c);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].board, "dialectic");
    let r = promote(&s, &mut c, "dialectic");
    assert_eq!(r["ok"], json!(true));
    assert_eq!(c["board"], json!("dialectic"));
    assert_eq!(c["column"], json!("explore"));
    assert_eq!(c["kind"], json!("position"));
    assert_eq!(c["reliability"], json!(0.70));
    assert!(c.get("rho").is_none()); // gate fields stripped
    assert_eq!(c["origin"], json!({"board": "protocol", "column": "closure"}));

    // dialectic synthesize → protocol initiation
    let mut d = card(&s.cards, "d6"); // seeded in synthesize? locate whatever is there
    d["column"] = json!("synthesize");
    let r = promote(&s, &mut d, "protocol");
    assert_eq!(r["ok"], json!(true));
    assert_eq!(d["board"], json!("protocol"));
    assert_eq!(d["source"], json!("recruited"));
    assert!(d.get("reliability").is_none());

    // protocol card with death mode → resolution phase0 (death/pathology → tags)
    let mut dead = card(&s.cards, "p2");
    dead["death"] = json!("fossil");
    dead["pathology"] = json!("structural");
    let targets = promotion_targets(&dead);
    assert!(targets.iter().any(|t| t.board == "resolution"));
    let r = promote(&s, &mut dead, "resolution");
    assert_eq!(r["ok"], json!(true));
    assert_eq!(dead["column"], json!("phase0"));
    assert_eq!(dead["tags"], json!(["† fossil", "structural layer"]));
    assert_eq!(dead["death"], Value::Null);

    // resolution phase5 → protocol initiation
    let mut r6 = card(&s.cards, "r6");
    r6["column"] = json!("phase5");
    let r = promote(&s, &mut r6, "protocol");
    assert_eq!(r["ok"], json!(true));
    assert_eq!(r6["board"], json!("protocol"));

    // no path → refusal message
    let p1 = card(&s.cards, "p1");
    let r = promote(&s, &mut p1.clone(), "dialectic");
    assert_eq!(r["ok"], json!(false));
    assert_eq!(
        r["message"],
        json!("no promotion path protocol/encounter → dialectic")
    );
    s.cards.insert("p1".into(), p1);
}

/* ------------------------------------------------------------- handles */

#[test]
fn camel_case_handles() {
    assert_eq!(to_camel_case("the onboarding drop-off"), "OnboardingDropOff");
    assert_eq!(to_camel_case("a b c d e f g"), "BCDEF"); // stop word dropped, 5 words max
    assert_eq!(to_camel_case("!!!"), "Card");
    assert_eq!(to_camel_case("here: the locus"), "Locus");
}

#[test]
fn ensure_handles_dedupes_case_insensitively() {
    let mut cards = Map::new();
    cards.insert("a".into(), json!({"id": "a", "title": "Same Title"}));
    cards.insert("b".into(), json!({"id": "b", "title": "same title"}));
    cards.insert("c".into(), json!({"id": "c", "title": "Same Title", "handle": "Custom"}));
    assert!(ensure_handles(&mut cards));
    assert_eq!(cards["a"]["handle"], json!("SameTitle"));
    assert_eq!(cards["b"]["handle"], json!("SameTitle2"));
    assert_eq!(cards["c"]["handle"], json!("Custom")); // never re-derived
    assert!(!ensure_handles(&mut cards)); // idempotent
}

#[test]
fn find_card_resolution_order() {
    let s = seeded();
    assert_eq!(find_card(&s.cards, "p1").unwrap()["id"], json!("p1"));
    assert_eq!(find_card(&s.cards, "@OnboardingDropOff").unwrap()["id"], json!("p1"));
    assert_eq!(find_card(&s.cards, "onboardingdropoff").unwrap()["id"], json!("p1"));
    assert!(find_card(&s.cards, "no such card exists anywhere").is_none());
}

/* ------------------------------------------------------------ actions */

#[test]
fn apply_action_create_move_edit_evaluate() {
    let mut s = seeded();

    // create
    let r = apply_action(&mut s, &json!({"action": "create_card", "board": "protocol", "title": "Ground the signup flow"}));
    assert_eq!(r["ok"], json!(true));
    assert_eq!(r["id"], json!("custom-0"));
    assert_eq!(s.board_id, "protocol");
    assert_eq!(s.custom_seq, 1);
    let c = card(&s.cards, "custom-0");
    assert_eq!(c["handle"], json!("GroundTheSignupFlow"));
    assert_eq!(c["trace"][0]["action"], json!("enters the workflow"));

    // move with A13 refusal (initiation → gate)
    let r = apply_action(&mut s, &json!({"action": "move_card", "id": "custom-0", "column": "gate"}));
    assert_eq!(r["ok"], json!(false));
    assert!(r["message"].as_str().unwrap().starts_with("A13"));

    // legal move
    let r = apply_action(&mut s, &json!({"action": "move_card", "id": "custom-0", "column": "encounter"}));
    assert_eq!(r["ok"], json!(true));
    assert_eq!(r["message"], json!("custom-0 → encounter"));

    // edit clamps through the action
    let r = apply_action(&mut s, &json!({"action": "edit_card", "id": "custom-0", "patch": {"theta": 0.35, "pathology": "content", "bogus": 1}}));
    assert_eq!(r["ok"], json!(true));
    assert_eq!(r["message"], json!("edited custom-0 (theta, pathology)"));
    assert_eq!(card(&s.cards, "custom-0")["theta"], json!(0.35));

    // edit with nothing editable
    let r = apply_action(&mut s, &json!({"action": "edit_card", "id": "custom-0", "patch": {"bogus": 1}}));
    assert_eq!(r["message"], json!("no editable fields in patch"));

    // evaluate (matches the README: Living (S 0.660))
    let r = apply_action(&mut s, &json!({"action": "evaluate_card", "id": "p1"}));
    assert_eq!(r["message"], json!("Living (S 0.660)"));
    assert_eq!(r["result"]["key"], json!("living"));

    // unknown action / unknown card / unknown board
    assert_eq!(
        apply_action(&mut s, &json!({"action": "dance"}))["message"],
        json!("unknown action dance")
    );
    assert_eq!(
        apply_action(&mut s, &json!({"action": "move_card", "id": "zzz", "column": "gate"}))["message"],
        json!("no card zzz")
    );
    assert_eq!(
        apply_action(&mut s, &json!({"action": "set_board", "board": "zzz"}))["message"],
        json!("unknown board zzz")
    );

    // set_board switches
    let r = apply_action(&mut s, &json!({"action": "set_board", "board": "dialectic"}));
    assert_eq!(r["message"], json!("board → dialectic"));
    assert_eq!(s.board_id, "dialectic");
}

/* ------------------------------------------------------------ seeding */

#[test]
fn seed_cards_match_the_spec_fixture() {
    let cards = seed_cards();
    assert_eq!(cards.len(), 21); // p1–p9, d1–d6, r1–r6
    for id in ["p1", "p9", "d1", "d6", "r1", "r6"] {
        let c = cards.get(id).unwrap();
        assert!(c.get("handle").and_then(Value::as_str).is_some());
        assert!(c.get("board").and_then(Value::as_str).is_some());
    }
    assert_eq!(cards["p1"]["board"], json!("protocol"));
    assert_eq!(cards["d1"]["board"], json!("dialectic"));
    assert_eq!(cards["r1"]["board"], json!("resolution"));
}

#[test]
fn compact_state_shape() {
    let s = seeded();
    let cs = compact_state(&s);
    assert_eq!(cs["boardId"], json!("protocol"));
    assert_eq!(cs["boards"].as_array().unwrap().len(), 3);
    let cards = cs["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 21);
    let p1 = cards.iter().find(|c| c["id"] == json!("p1")).unwrap();
    assert_eq!(p1["S"], json!(0.66));
    assert_eq!(p1["verdict"], json!("living"));
    assert_eq!(p1["handle"], json!("OnboardingDropOff"));
    let d1 = cards.iter().find(|c| c["id"] == json!("d1")).unwrap();
    assert_eq!(d1["reliability"], json!(0.81));
    assert!(d1.get("S").is_none());
    assert_eq!(d1["agent"], json!("u₁"));
}
