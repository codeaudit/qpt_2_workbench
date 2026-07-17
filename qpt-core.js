/* QPT Workbench — shared domain core.
 *
 * Single source of truth for the specification's mechanics, usable in the browser
 * (window.QPT_CORE) and in Node (module.exports). All semantic enforcement —
 * A13, A9, §14, A7, gate math, trace, promotion — lives here and only here;
 * app.js (DOM) and server.js (store/CLI/agent) delegate to it.
 *
 * State shape expected throughout: { boardId, cards: {id: card}, customSeq }.
 */
"use strict";

(function (root, factory) {
  if (typeof module !== "undefined" && module.exports) module.exports = factory();
  else root.QPT_CORE = factory();
})(typeof self !== "undefined" ? self : this, function () {
  const SIGMA_A = 10; // sigmoid sharpness `a` (§8.1)

  function createCore(DATA) {
    const R = DATA.reference;
    const boards = DATA.boards;
    const clone = (o) => JSON.parse(JSON.stringify(o));

    /* ------------------------------------------------------------ basics */

    function boardOf(state) { return boards.find((b) => b.id === state.boardId); }
    function colIndex(state, id) { return boardOf(state).columns.findIndex((c) => c.id === id); }
    function colName(boardId, colId) {
      const b = boards.find((x) => x.id === boardId);
      const col = b && b.columns.find((x) => x.id === colId);
      return col ? col.name : colId;
    }
    function cardsOn(state, boardId) {
      return Object.values(state.cards).filter((c) => c.board === boardId);
    }

    /* ------------------------------------------------------ gate (§7–8) */

    function scoreOf(c) {
      if (c.rho == null) return null;
      return c.rho - c.delta - c.gamma * Math.log(c.k);
    }
    function zoneOf(c) {
      const S = scoreOf(c);
      if (S == null) return null;
      const d = S - c.theta;
      if (Math.abs(d) <= 0.15) return 2;
      return d < 0 ? 1 : 3;
    }
    function sigmoidP(c) {
      const S = scoreOf(c);
      if (S == null) return null;
      return 1 / (1 + Math.exp(-SIGMA_A * (S - c.theta)));
    }
    function evaluate(c) {
      if (c.source == null) return null;
      const S = scoreOf(c);
      const Rc = c.source === "recruited";
      const Gc = c.target === "grounded";
      const quant = S >= c.theta;
      let key, label, sub;
      if (Rc && Gc && quant)        { key = "living";    label = "Living";             sub = "generative act — dual passage"; }
      else if (Rc && Gc)            { key = "threshold"; label = "At threshold";       sub = "Zone 2 — unstable; stay with the encounter"; }
      else if (Rc && !Gc)           { key = "delusion";  label = "Beautiful delusion"; sub = "R without G — inspired, ungrounded"; }
      else if (!Rc && Gc)           { key = "competent"; label = "Competent-dead";     sub = "G without R — grounded, no vitality"; }
      else                          { key = "dead";      label = "Fully dead";         sub = "administrative process — {△} → {△}"; }
      return { R: Rc, G: Gc, quant, S, key, label, sub, living: key === "living" };
    }
    function namingClass(c) {
      const r = c.source === "recruited", g = c.target === "grounded";
      if (r && g)  return { key: "living",    label: "Living name" };
      if (r && !g) return { key: "delusion",  label: "Poetic capture" };
      if (!r && g) return { key: "competent", label: "Technical term" };
      return { key: "dead", label: "Jargon" };
    }
    function weights(state) {
      const EPS0 = 0.05;
      const withRel = cardsOn(state, state.boardId).filter((c) => c.reliability != null);
      const sum = withRel.reduce((acc, c) => acc + c.reliability + EPS0, 0);
      const map = {};
      withRel.forEach((c) => { map[c.id] = (c.reliability + EPS0) / sum; });
      return map;
    }

    /* ------------------------------------------------------- trace (§9.2) */

    function pushTrace(c, entry) {
      if (!c.trace) c.trace = [];
      entry.n = c.trace.length + 1;
      c.trace.push(entry);
      if (c.trace.length > 50) c.trace = c.trace.slice(-50);
    }

    function onTransition(state, c, from, to) {
      let note = "";
      const cols = boardOf(state).columns;
      const fi = cols.findIndex((x) => x.id === from);
      const ti = cols.findIndex((x) => x.id === to);
      const ev = evaluate(c);
      if (to === "gate" && ev) {
        note = "evaluated at the gate: " + ev.label + " — S " + ev.S.toFixed(2) +
               (ev.quant ? " ≥ θ " : " < θ ") + c.theta.toFixed(2);
      }
      if (from === "gate" && to === "articulation" && ev) {
        note = "dual passage (A9): S " + ev.S.toFixed(2) + " ≥ θ " + c.theta.toFixed(2) +
               " — proceeds to naming downstream of passage";
      }
      if (c.board === "dialectic" && ti === fi + 1) {
        const parts = ["χ " + cols[fi].step + " → " + cols[ti].step + " (schedule →|π)"];
        if (c.reliability != null) {
          const old = c.reliability;
          c.reliability = Math.round((0.7 * old + 0.3) * 100) / 100;
          parts.push("Γ " + old.toFixed(2) + " → " + c.reliability.toFixed(2) + " (EMA λ = 0.7, §19)");
        }
        note = parts.join("; ");
      }
      pushTrace(c, { action: "passage", from: colName(c.board, from), to: colName(c.board, to), note });
    }

    function applyMove(state, c, toId) {
      const from = c.column;
      c.column = toId;
      onTransition(state, c, from, toId);
    }

    /* ------------------------------------------------ movement semantics */

    function canMove(state, c, toId) {
      const from = colIndex(state, c.column);
      const to = colIndex(state, toId);
      if (to < 0 || to === from) return { ok: true };
      if (to > from + 1) {
        return { ok: false, msg: "A13 · Commitment horizon = 1 — the option space past the next gate passage does not yet exist. Advance one column at a time." };
      }
      if (state.boardId === "protocol") {
        const ev = evaluate(c);
        if (c.column === "gate" && toId === "articulation" && ev && !ev.living) {
          return { ok: false, msg: "A9 · The gate refused: R ⊓ G does not hold here (" + ev.label + "). {△}-articulation before dual passage is dead naming.", focus: "gate" };
        }
        if (c.column === "gate" && to < from && ev && !ev.living && !c.pathology) {
          return { ok: false, msg: "§14 · Gate failed (" + ev.label + "). Diagnose via the five-layer model before returning to step 1.", focus: "pathology" };
        }
      }
      if (state.boardId === "dialectic" && toId === "synthesize" && c.kind !== "note") {
        if (!(c.genesis && c.genesis.trim())) {
          return { ok: false, msg: "A7 · Genesis requires an emergent property present in neither parent — declare it before synthesis.", focus: "genesis" };
        }
      }
      return { ok: true };
    }

    /* ------------------------------------------------------------ agents */

    function nextAgent(state, boardId) {
      const SUB = "₀₁₂₃₄₅₆₇₈₉";
      const agentNum = (a) => {
        if (!a) return null;
        let m = /^u(\d+)$/.exec(a);
        if (m) return parseInt(m[1], 10);
        m = /^u([₀-₉]+)$/.exec(a);
        if (m) return parseInt(m[1].split("").map((ch) => SUB.indexOf(ch)).join(""), 10);
        return null;
      };
      const used = cardsOn(state, boardId).map((c) => agentNum(c.agent)).filter((n) => n != null);
      const n = (used.length ? Math.max.apply(null, used) : 0) + 1;
      return "u" + String(n).split("").map((d) => SUB[parseInt(d, 10)]).join("");
    }

    /* ------------------------------------------------------------ cards */

    function buildCard(state, boardId, id, fields) {
      const b = boards.find((x) => x.id === boardId);
      if (!b || !fields.title) return null;
      const col = b.columns[0].id;
      const title = String(fields.title).slice(0, 200);
      if (boardId === "protocol") {
        const source = fields.source === "initiated" ? "initiated" : "recruited";
        const target = fields.target === "terminated" ? "terminated" : "grounded";
        return {
          id, board: boardId, column: col, title,
          sign: source === "recruited" ? "⟨⦿○ ≈ ⊣○⟩" : "⟨⦿△ ≡ ⊣△⟩",
          signName: source === "recruited" ? "Qualisign–Icon–Rheme" : "Legisign–Symbol–Argument",
          source, target,
          rho: 0.80, delta: 0.10, gamma: 0.25, k: 3, theta: 0.40,
          scale: "meso", cycle: 0, pathology: null, death: null, axioms: ["A9"],
          note: fields.note || "Practitioner-entered transformation. Metrics are initial estimates — revise them after [□]-encounter.",
        };
      }
      if (boardId === "dialectic") {
        if (fields.kind === "note") {
          return {
            id, board: boardId, column: col, kind: "note", title,
            sign: "⟹ᵐⁿᵃᵛ¹ ⊗ |ᵍ ⊗ ⟹ᵐᵉᵐ", signName: "moderator note",
            reliability: null, cycle: 0, axioms: ["A16"],
            note: fields.note || "Practitioner-entered note.",
          };
        }
        const rel = Math.max(0, Math.min(1, typeof fields.reliability === "number" ? fields.reliability : 0.70));
        return {
          id, board: boardId, column: col, kind: "position", agent: nextAgent(state, boardId), title,
          sign: "⟨⦿△ ≡ ⊣α⟩", signName: "Legisign–Symbol–Abduction",
          reliability: rel, cycle: 0, genesis: null, axioms: ["A19"],
          note: fields.note || ("Practitioner-entered position. Γ starts at " + rel.toFixed(2) +
                "; the moderator updates it by EMA as scores arrive (§19), and w is recomputed across the board."),
        };
      }
      if (boardId === "resolution") {
        return {
          id, board: boardId, column: col, title,
          sign: "⟨⦿□ ⇢ ⊣□⟩", signName: "Sinsign–Index–Dicisign",
          cycle: 0,
          tags: Array.isArray(fields.tags) ? fields.tags.filter((t) => typeof t === "string").slice(0, 8) : [],
          axioms: ["ML"],
          note: fields.note || "Practitioner-entered intervention.",
        };
      }
      return null;
    }

    const EDITABLE_KEYS = {
      title: "str", note: "str", sign: "str", signName: "str", agent: "str",
      source: ["recruited", "initiated"], target: ["grounded", "terminated"],
      scale: ["micro", "meso", "macro"],
      pathology: ["structural", "attentional", "content", "scalar", "temporal", null],
      death: ["fossil", "residue", "imposition", null],
      rho: "num01", delta: "num01", gamma: "num01", theta: "num01", reliability: "num01",
      k: "k", genesis: "strnull",
    };

    function sanitizePatch(patch) {
      const out = {};
      Object.entries(patch || {}).forEach(([key, val]) => {
        const rule = EDITABLE_KEYS[key];
        if (key === "tags" && Array.isArray(val)) { out.tags = val.filter((t) => typeof t === "string").slice(0, 8); return; }
        if (!rule) return;
        if (rule === "str") out[key] = String(val).slice(0, key === "note" ? 2000 : 200);
        else if (rule === "strnull") out[key] = val == null || val === "" ? null : String(val).slice(0, 200);
        else if (rule === "num01") { const n = parseFloat(val); if (!isNaN(n)) out[key] = Math.max(0, Math.min(1, n)); }
        else if (rule === "k") { const n = parseInt(val, 10); if (!isNaN(n)) out[key] = Math.max(1, Math.min(16, n)); }
        else if (Array.isArray(rule) && rule.includes(val)) out[key] = val;
      });
      return out;
    }

    /* --------------------------------------------------------- promotion */

    function stripGateFields(c) {
      delete c.source; delete c.target;
      delete c.rho; delete c.delta; delete c.gamma; delete c.k; delete c.theta;
      delete c.scale;
      c.pathology = null; c.death = null;
    }
    function stripDialecticFields(c) {
      delete c.kind; delete c.agent; delete c.reliability; delete c.genesis;
    }
    function addAxioms(c, ids) {
      c.axioms = (c.axioms || []).concat(ids).filter((v, i, a) => a.indexOf(v) === i);
    }

    function promotionTargets(c) {
      const T = [];
      if (c.board === "protocol" && c.column === "closure") {
        T.push({ board: "dialectic", column: "explore",
          label: "⇧ Promote to the Scheduled Dialectic",
          why: "§15 · System-2 = coordinated(System-1): a resolved structure whose question is now contested enters coordination as a position in Explore." });
      }
      if (c.board === "dialectic" && c.column === "synthesize") {
        T.push({ board: "protocol", column: "initiation",
          label: "⇧ Ground the constructor in the Generative Protocol",
          why: "§7.4 · The gate applies to everything: an emergent constructor is not automatically living — it must pass 𝒢 like any transformation." });
      }
      if (c.board === "protocol" && c.death) {
        T.push({ board: "resolution", column: "phase0",
          label: "⇧ Escalate to the Resolution Procedure",
          why: "§11 / §29 · Dead structure is failure-field material: it is not forced through the gate but mapped, excavated, and redesigned." });
      }
      if (c.board === "resolution" && c.column === "phase5") {
        T.push({ board: "protocol", column: "initiation",
          label: "⇧ Return to living process",
          why: "§29 · Evolutionary iteration closes the loop: the redesigned structure re-enters the Generative Protocol at Initiation." });
      }
      return T;
    }

    // pure promotion: mutates the card; returns { ok, label, why } or { ok:false, message }
    function promote(state, c, targetBoard) {
      const t = promotionTargets(c).find((x) => x.board === targetBoard);
      if (!t) {
        return { ok: false, message: "no promotion path " + c.board + "/" + c.column + " → " + targetBoard };
      }
      const from = { board: c.board, column: c.column };
      if (t.board === "dialectic") {
        stripGateFields(c);
        delete c.tags;
        c.kind = "position";
        c.agent = nextAgent(state, "dialectic");
        c.reliability = 0.70;
        c.genesis = null;
        c.sign = "⟨⦿△ ≡ ⊣α⟩"; c.signName = "Legisign–Symbol–Abduction";
        addAxioms(c, ["A19", "A20"]);
      } else if (t.board === "protocol") {
        stripDialecticFields(c);
        delete c.tags;
        c.source = "recruited"; c.target = "grounded";
        c.rho = 0.80; c.delta = 0.10; c.gamma = 0.25; c.k = 3; c.theta = 0.40;
        c.scale = "meso"; c.pathology = null; c.death = null;
        c.sign = "⟨⦿□ ⇢ ⊣□⟩"; c.signName = "Sinsign–Index–Dicisign";
        addAxioms(c, ["A9"]);
      } else {
        const wasDeath = c.death, wasPath = c.pathology;
        stripGateFields(c);
        stripDialecticFields(c);
        const tags = [];
        if (wasDeath) tags.push("† " + wasDeath);
        if (wasPath) tags.push(wasPath + " layer");
        c.tags = tags.length ? tags : ["from the protocol"];
        c.pathology = null; c.death = null;
        c.sign = "⟨⦿□ ⇢ ⊣□⟩"; c.signName = "Sinsign–Index–Dicisign";
        addAxioms(c, ["ML"]);
      }
      c.origin = from;
      pushTrace(c, {
        action: "⇧ promote",
        from: (boards.find((b) => b.id === from.board) || {}).title + " · " + colName(from.board, from.column),
        to: (boards.find((b) => b.id === t.board) || {}).title + " · " + colName(t.board, t.column),
        note: t.why,
      });
      c.board = t.board;
      c.column = t.column;
      return { ok: true, label: t.label, why: t.why, column: t.column };
    }

    /* -------------------------------------------------- action execution */

    // the canonical action vocabulary — executed identically by browser and server
    function applyAction(state, a) {
      switch (a.action) {
        case "set_board": {
          if (!boards.some((b) => b.id === a.board)) return { ok: false, message: "unknown board " + a.board };
          state.boardId = a.board;
          return { ok: true, message: "board → " + a.board };
        }
        case "create_card": {
          state.customSeq = state.customSeq || 0;
          const id = "custom-" + state.customSeq++;
          const card = buildCard(state, a.board, id, a);
          if (!card) return { ok: false, message: "cannot create card on board " + a.board };
          state.cards[id] = card;
          ensureHandles(state.cards); // assigns a unique CamelCase handle (idempotent)
          pushTrace(card, { action: "enters the workflow", from: "—", to: colName(a.board, card.column), note: "created via " + (a.via || "the workbench") });
          state.boardId = a.board;
          return { ok: true, message: "created “" + card.title + "” (" + id + ")", id };
        }
        case "move_card": {
          const c = state.cards[a.id];
          if (!c) return { ok: false, message: "no card " + a.id };
          if (c.board !== state.boardId) state.boardId = c.board;
          const chk = canMove(state, c, a.column);
          if (!chk.ok) return { ok: false, message: chk.msg, focus: chk.focus };
          applyMove(state, c, a.column);
          return { ok: true, message: a.id + " → " + a.column };
        }
        case "edit_card": {
          const c = state.cards[a.id];
          if (!c) return { ok: false, message: "no card " + a.id };
          const patch = sanitizePatch(a.patch);
          if (!Object.keys(patch).length) return { ok: false, message: "no editable fields in patch" };
          Object.assign(c, patch);
          if (c.board !== state.boardId) state.boardId = c.board;
          return { ok: true, message: "edited " + a.id + " (" + Object.keys(patch).join(", ") + ")" };
        }
        case "promote_card": {
          const c = state.cards[a.id];
          if (!c) return { ok: false, message: "no card " + a.id };
          if (c.board !== state.boardId) state.boardId = c.board;
          const r = promote(state, c, a.board);
          if (!r.ok) return r;
          state.boardId = c.board;
          return { ok: true, message: r.label, why: r.why };
        }
        case "evaluate_card": {
          const c = state.cards[a.id];
          if (!c) return { ok: false, message: "no card " + a.id };
          const ev = evaluate(c);
          return { ok: true, message: ev ? ev.label + " (S " + ev.S.toFixed(3) + ")" : "no gate fields on this card", result: ev };
        }
        default:
          return { ok: false, message: "unknown action " + a.action };
      }
    }

    /* ------------------------------------------------------------- misc */

    // stable CamelCase handles for referencing (@OnboardingDropoff).
    // Assigned once at creation/seed/migration, deduped, and never re-derived
    // from renames — references must not break when a title changes.
    const HANDLE_STOP = new Set(["the", "a", "an", "here"]);
    function toCamelCase(title) {
      let words = String(title || "").replace(/[^a-zA-Z0-9\s]/g, " ").split(/\s+/).filter(Boolean);
      while (words.length > 1 && HANDLE_STOP.has(words[0].toLowerCase())) words = words.slice(1);
      words = words.slice(0, 5);
      const cc = words.map((w) => w[0].toUpperCase() + w.slice(1)).join("");
      return cc || "Card";
    }

    // ensure every card in the map has a unique handle; mutates in place
    function ensureHandles(cards) {
      const taken = new Set(
        Object.values(cards).map((c) => (c.handle || "").toLowerCase()).filter(Boolean)
      );
      let changed = false;
      Object.values(cards).forEach((c) => {
        if (c.handle) return;
        const base = toCamelCase(c.title || c.id);
        let h = base, n = 2;
        while (taken.has(h.toLowerCase())) h = base + n++;
        taken.add(h.toLowerCase());
        c.handle = h;
        changed = true;
      });
      return changed;
    }

    // resolve a reference: id, @handle, handle, exact title, or title substring
    function findCard(cards, ref) {
      const key = String(ref || "").replace(/^@/, "");
      if (cards[key]) return cards[key];
      const lower = key.toLowerCase();
      return Object.values(cards).find((c) => (c.handle || "").toLowerCase() === lower)
        || Object.values(cards).find((c) => (c.title || "").toLowerCase() === lower)
        || Object.values(cards).find((c) => (c.title || "").toLowerCase().includes(lower))
        || null;
    }

    function compactState(state) {
      return {
        boardId: state.boardId,
        boards: boards.map((b) => ({ id: b.id, columns: b.columns.map((c) => c.id) })),
        cards: Object.values(state.cards).map((c) => {
          const out = { id: c.id, handle: c.handle, board: c.board, column: c.column, title: c.title, sign: c.sign, cycle: c.cycle || 0 };
          if (c.source != null) {
            const ev = evaluate(c);
            out.source = c.source; out.target = c.target;
            out.rho = c.rho; out.delta = c.delta; out.gamma = c.gamma; out.k = c.k; out.theta = c.theta;
            out.S = +scoreOf(c).toFixed(3);
            out.verdict = ev && ev.key;
          }
          if (c.reliability != null) out.reliability = c.reliability;
          if (c.kind) out.kind = c.kind;
          if (c.agent) out.agent = c.agent;
          if (c.genesis) out.genesis = c.genesis;
          if (c.pathology) out.pathology = c.pathology;
          if (c.death) out.death = c.death;
          if (c.tags && c.tags.length) out.tags = c.tags;
          return out;
        }),
      };
    }

    function seedCards() {
      const cards = {};
      boards.forEach((b) => b.cards.forEach((c) => {
        cards[c.id] = clone(c);
        cards[c.id].board = b.id;
      }));
      ensureHandles(cards);
      return cards;
    }

    return {
      boards, colName, colIndex, cardsOn,
      scoreOf, zoneOf, sigmoidP, evaluate, namingClass, weights,
      pushTrace, onTransition, applyMove, canMove,
      nextAgent, buildCard, sanitizePatch, EDITABLE_KEYS,
      promotionTargets, promote, applyAction, compactState, seedCards,
      toCamelCase, ensureHandles, findCard,
    };
  }

  return { createCore };
});
