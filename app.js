/* QPT Workbench — kanban, Quality Gate evaluation, and spec-semantic editing.
 *
 * Enforcements implemented (all rejections cite the spec):
 *   A13  commitment horizon = 1   — forward moves limited to the adjacent column
 *   A9   dual gate                — only living trajectories advance Gate → Articulation
 *   §14  generative protocol      — a failed card must be diagnosed (five layers) before returning
 *   §12  living naming            — titles are classified live; naming upstream of the gate warns
 *   A7   genesis emergence        — entering Synthesize requires an emergent property ∉ parents
 *   §19  scheduled dialectic      — χ follows the column schedule; Γ editable, w recomputed live
 */
"use strict";

(function () {
  const R = QPT_DATA.reference;
  const STORE_KEY = "qpt-workbench-v2";
  const SIGMA_A = 10; // sigmoid sharpness `a` (§8.1)

  const SCALE_HINTS = {
    micro: "seconds — haptic contact (§10)",
    meso: "minutes–hours — kinesthetic encounter (§10)",
    macro: "days–years — biographical inhabitation (§10)",
  };

  const state = { boardId: "protocol", cards: {}, customSeq: 0 };
  let pendingMove = null; // { id, colId } awaiting a semantically required edit
  let sim = { active: false, idx: -1, timer: null, snapshot: null, quiet: false, playing: false };

  /* ------------------------------------------------------------- state */

  function clone(obj) { return JSON.parse(JSON.stringify(obj)); }

  function initState() {
    QPT_DATA.boards.forEach((b) => b.cards.forEach((c) => {
      state.cards[c.id] = clone(c);
      state.cards[c.id].board = b.id;
    }));
    const saved = loadStore();
    if (saved) {
      if (saved.boardId && QPT_DATA.boards.some((b) => b.id === saved.boardId)) {
        state.boardId = saved.boardId;
      }
      Object.entries(saved.cards || {}).forEach(([id, c]) => { state.cards[id] = c; });
    }
    state.customSeq = Object.keys(state.cards).reduce((n, id) => {
      const m = /^custom-(\d+)$/.exec(id);
      return m ? Math.max(n, parseInt(m[1], 10) + 1) : n;
    }, 0);
  }

  function loadStore() {
    try { return JSON.parse(localStorage.getItem(STORE_KEY)); } catch (e) { return null; }
  }

  function saveStore() {
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify({ boardId: state.boardId, cards: state.cards }));
    } catch (e) { /* storage unavailable — board still works in-memory */ }
  }

  function resetStore() {
    try { localStorage.removeItem(STORE_KEY); } catch (e) {}
    pendingMove = null;
    activeExercise = null;
    const gb = document.getElementById("goal-banner");
    if (gb) gb.classList.add("hidden");
    state.boardId = "protocol";
    state.cards = {};
    state.customSeq = 0;
    initState();
    closeModal();
    renderAll();
    notice("Board reset to the seeded specification state.");
  }

  function currentBoard() { return QPT_DATA.boards.find((b) => b.id === state.boardId); }
  function boardCards() {
    return Object.values(state.cards).filter((c) => c.board === state.boardId);
  }
  function colIndex(id) { return currentBoard().columns.findIndex((c) => c.id === id); }
  function colById(id) { return currentBoard().columns.find((c) => c.id === id); }

  /* ------------------------------------------------------ gate logic (§7–8) */

  function scoreOf(c) {
    if (c.rho == null) return null;
    return c.rho - c.delta - c.gamma * Math.log(c.k);
  }

  function zoneOf(c) {
    const S = scoreOf(c);
    if (S == null) return null;
    const d = S - c.theta;
    if (Math.abs(d) <= 0.15) return 2;      // phase transition
    return d < 0 ? 1 : 3;                    // drift / anchored control
  }

  function sigmoidP(c) {
    const S = scoreOf(c);
    if (S == null) return null;
    return 1 / (1 + Math.exp(-SIGMA_A * (S - c.theta)));
  }

  function evaluate(c) {
    if (c.source == null) return null;
    const S = scoreOf(c);
    const Rc = c.source === "recruited";                 // (○)-recruited
    const Gc = c.target === "grounded";                  // [□]-grounded
    const quant = S >= c.theta;                          // S ≥ θ
    let key, label, sub;
    if (Rc && Gc && quant)        { key = "living";    label = "Living";             sub = "generative act — dual passage"; }
    else if (Rc && Gc)            { key = "threshold"; label = "At threshold";       sub = "Zone 2 — unstable; stay with the encounter"; }
    else if (Rc && !Gc)           { key = "delusion";  label = "Beautiful delusion"; sub = "R without G — inspired, ungrounded"; }
    else if (!Rc && Gc)           { key = "competent"; label = "Competent-dead";     sub = "G without R — grounded, no vitality"; }
    else                          { key = "dead";      label = "Fully dead";         sub = "administrative process — {△} → {△}"; }
    return { R: Rc, G: Gc, quant, S, key, label, sub, living: key === "living" };
  }

  // naming trajectory (§12) — derived from the same (source, target) vector
  function namingClass(c) {
    const r = c.source === "recruited", g = c.target === "grounded";
    if (r && g)  return { key: "living",    label: "Living name" };
    if (r && !g) return { key: "delusion",  label: "Poetic capture" };
    if (!r && g) return { key: "competent", label: "Technical term" };
    return { key: "dead", label: "Jargon" };
  }

  // normalized reliability weights across a board (§19): wᵤ = (Γᵤ+ε₀)/Σᵥ(Γᵥ+ε₀)
  function weights() {
    const EPS0 = 0.05;
    const withRel = boardCards().filter((c) => c.reliability != null);
    const sum = withRel.reduce((acc, c) => acc + c.reliability + EPS0, 0);
    const map = {};
    withRel.forEach((c) => { map[c.id] = (c.reliability + EPS0) / sum; });
    return map;
  }

  /* ------------------------------------------------------------- notices */

  function notice(msg) {
    if (sim.quiet) return; // replaying simulation steps silently
    const box = document.getElementById("toast");
    const t = el("div", "toast", msg);
    // toasts citing a registry entry link straight into the reference drawer
    const m = /^(A\d+|L[1-7]|ML)\b/.exec(msg);
    if (m && R.axioms.some((a) => a.id === m[1])) {
      t.classList.add("link");
      t.title = "Open " + m[1] + " in the registry";
      t.addEventListener("click", () => openDrawer("axioms", m[1]));
    }
    box.appendChild(t);
    setTimeout(() => t.classList.add("gone"), 4200);
    setTimeout(() => t.remove(), 4800);
  }

  /* ------------------------------------------------- movement semantics */

  function canMove(c, toId) {
    const from = colIndex(c.column);
    const to = colIndex(toId);
    if (to < 0 || to === from) return { ok: true };

    // A13 — commitment horizon = 1: no forward skips
    if (to > from + 1) {
      return { ok: false, msg: "A13 · Commitment horizon = 1 — the option space past the next gate passage does not yet exist. Advance one column at a time." };
    }

    if (state.boardId === "protocol") {
      const ev = evaluate(c);
      // A9 — dual gate: only living trajectories proceed to articulation
      if (c.column === "gate" && toId === "articulation" && ev && !ev.living) {
        return { ok: false, msg: "A9 · The gate refused: R ⊓ G does not hold here (" + ev.label + "). {△}-articulation before dual passage is dead naming.", focus: "gate" };
      }
      // §14 step 3 — failure must be diagnosed before returning to step 1
      if (c.column === "gate" && to < from && ev && !ev.living && !c.pathology) {
        return { ok: false, msg: "§14 · Gate failed (" + ev.label + "). Diagnose via the five-layer model before returning to step 1.", focus: "pathology" };
      }
    }

    // A7 — genesis emergence: synthesis requires a property present in no parent
    if (state.boardId === "dialectic" && toId === "synthesize" && c.kind !== "note") {
      if (!(c.genesis && c.genesis.trim())) {
        return { ok: false, msg: "A7 · Genesis requires an emergent property present in neither parent — declare it before synthesis.", focus: "genesis" };
      }
    }
    return { ok: true };
  }

  function colName(boardId, colId) {
    const b = QPT_DATA.boards.find((x) => x.id === boardId);
    const col = b && b.columns.find((x) => x.id === colId);
    return col ? col.name : colId;
  }

  // §9.2 — the trace: a card's own post-hoc articulation of its navigation
  function pushTrace(c, entry) {
    if (!c.trace) c.trace = [];
    entry.n = c.trace.length + 1;
    c.trace.push(entry);
    if (c.trace.length > 50) c.trace = c.trace.slice(-50);
  }

  // actions fired by a successful intra-board move
  function onTransition(c, from, to) {
    let note = "";
    const cols = currentBoard().columns;
    const fi = cols.findIndex((x) => x.id === from);
    const ti = cols.findIndex((x) => x.id === to);

    // gate evaluations are recorded with their quantitative state (§7–8)
    const ev = evaluate(c);
    if (to === "gate" && ev) {
      note = "evaluated at the gate: " + ev.label + " — S " + ev.S.toFixed(2) +
             (ev.quant ? " ≥ θ " : " < θ ") + c.theta.toFixed(2);
    }
    if (from === "gate" && to === "articulation" && ev) {
      note = "dual passage (A9): S " + ev.S.toFixed(2) + " ≥ θ " + c.theta.toFixed(2) +
             " — proceeds to naming downstream of passage";
    }

    // §19 — advancing a dialectic phase: χ follows the schedule; Γ updates by EMA
    if (c.board === "dialectic" && ti === fi + 1) {
      const parts = ["χ " + cols[fi].step + " → " + cols[ti].step + " (schedule →|π)"];
      if (c.reliability != null) {
        const old = c.reliability;
        c.reliability = Math.round((0.7 * old + 0.3) * 100) / 100; // λ = 0.7, score = 1 (survived the phase)
        parts.push("Γ " + old.toFixed(2) + " → " + c.reliability.toFixed(2) + " (EMA λ = 0.7, §19)");
      }
      note = parts.join("; ");
    }

    pushTrace(c, { action: "passage", from: colName(c.board, from), to: colName(c.board, to), note });
  }

  function moveCard(id, colId) {
    const c = state.cards[id];
    if (!c || c.column === colId) return;
    const chk = canMove(c, colId);
    if (!chk.ok) {
      notice(chk.msg);
      if (chk.focus) {
        pendingMove = { id, colId };
        openModal(id, chk.focus);
      }
      return;
    }
    const from = c.column;
    c.column = colId;
    onTransition(c, from, colId);
    saveStore();
    renderAll();
    if (colId === "gate") {
      const node = $board.querySelector('[data-id="' + id + '"]');
      if (node) node.classList.add("gate-flash");
    }
  }

  // an edit may have satisfied a pending requirement — retry the blocked move
  function retryPending(c) {
    if (!pendingMove || pendingMove.id !== c.id) return;
    const chk = canMove(c, pendingMove.colId);
    if (chk.ok) {
      const to = pendingMove.colId;
      pendingMove = null;
      notice("Requirement satisfied — passage recorded.");
      closeModal();
      moveCard(c.id, to);
    }
  }

  function applyEdit(c) {
    saveStore();
    renderBoard();
    refreshView(c);
    retryPending(c);
  }

  /* ------------------------------------------- promotion between boards */

  function cardsOn(boardId) {
    return Object.values(state.cards).filter((c) => c.board === boardId);
  }

  function nextAgent(boardId) {
    const SUB = "₀₁₂₃₄₅₆₇₈₉";
    const agentNum = (a) => {
      if (!a) return null;
      let m = /^u(\d+)$/.exec(a);
      if (m) return parseInt(m[1], 10);
      m = /^u([₀-₉]+)$/.exec(a);
      if (m) return parseInt(m[1].split("").map((ch) => SUB.indexOf(ch)).join(""), 10);
      return null;
    };
    const used = cardsOn(boardId).map((c) => agentNum(c.agent)).filter((n) => n != null);
    const n = (used.length ? Math.max.apply(null, used) : 0) + 1;
    return "u" + String(n).split("").map((d) => SUB[parseInt(d, 10)]).join("");
  }

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

  // the meta-workflow: how work moves between the three boards
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

  function promote(c, t) {
    const from = { board: c.board, column: c.column };
    if (t.board === "dialectic") {
      stripGateFields(c);
      delete c.tags;
      c.kind = "position";
      c.agent = nextAgent("dialectic");
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
    } else { // resolution
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
      from: (QPT_DATA.boards.find((b) => b.id === from.board) || {}).title + " · " + colName(from.board, from.column),
      to: (QPT_DATA.boards.find((b) => b.id === t.board) || {}).title + " · " + colName(t.board, t.column),
      note: t.why,
    });
    c.board = t.board;
    c.column = t.column;
    pendingMove = null;
    saveStore();
    closeModal();
    state.boardId = t.board;
    renderAll();
    notice(t.why);
  }

  /* ----------------------------------------------------------- rendering */

  const $board = document.getElementById("board");
  const $ctx = document.getElementById("board-context");
  const $tabs = document.getElementById("board-tabs");

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function renderTabs() {
    $tabs.innerHTML = "";
    QPT_DATA.boards.forEach((b) => {
      const t = el("button", "tab" + (b.id === state.boardId ? " active" : ""), b.title);
      t.addEventListener("click", () => { state.boardId = b.id; saveStore(); renderAll(); });
      $tabs.appendChild(t);
    });
  }

  function renderContext() {
    const b = currentBoard();
    $ctx.innerHTML = "";
    $ctx.appendChild(el("span", "ctx-ref", b.ref));
    $ctx.appendChild(el("span", "ctx-note", b.note));
    const legend = el("span", "ctx-legend");
    [["living", "Living"], ["delusion", "Beautiful delusion"], ["competent", "Competent-dead"], ["dead", "Fully dead"]]
      .forEach(([k, label]) => legend.appendChild(el("span", "chip verdict " + k, label)));
    $ctx.appendChild(legend);
  }

  function renderBoard() {
    const b = currentBoard();
    const w = weights();
    $board.innerHTML = "";
    b.columns.forEach((col) => {
      const colEl = el("section", "column cat-" + col.cat);
      colEl.dataset.column = col.id;

      const head = el("header", "col-head");
      head.appendChild(el("span", "col-glyph", col.glyph));
      const titleWrap = el("div", "col-title");
      titleWrap.appendChild(el("span", "col-name", col.name));
      titleWrap.appendChild(el("span", "col-step", col.step));
      head.appendChild(titleWrap);
      const count = el("span", "col-count");
      head.appendChild(count);
      colEl.appendChild(head);
      colEl.appendChild(el("p", "col-desc", col.desc));
      if (col.axioms && col.axioms.length) {
        const ax = el("div", "col-axioms");
        col.axioms.forEach((a) => ax.appendChild(el("span", "chip axiom", a)));
        colEl.appendChild(ax);
      }

      const list = el("div", "card-list");
      const cards = boardCards().filter((c) => c.column === col.id);
      count.textContent = cards.length;
      cards.forEach((c) => list.appendChild(renderCard(c, col, w)));
      if (!cards.length) {
        list.appendChild(el("div", "empty-hint",
          "No cards — the graph has not yet produced this adjacency (A12)."));
      }
      colEl.appendChild(list);

      if (col === b.columns[0]) {
        colEl.appendChild(renderNewCardForm(b));
      }

      colEl.addEventListener("dragover", (ev) => {
        ev.preventDefault();
        colEl.classList.add("drop-target");
      });
      colEl.addEventListener("dragleave", () => colEl.classList.remove("drop-target"));
      colEl.addEventListener("drop", (ev) => {
        ev.preventDefault();
        colEl.classList.remove("drop-target");
        moveCard(ev.dataTransfer.getData("text/plain"), col.id);
      });

      $board.appendChild(colEl);
    });
    checkExercise();
  }

  function renderCard(c, col, w) {
    const card = el("article", "card");
    card.draggable = true;
    card.dataset.id = c.id;

    const top = el("div", "card-top");
    top.appendChild(el("span", "sign chip", c.sign));
    if (c.scale) top.appendChild(el("span", "chip scale", c.scale));
    if (c.cycle) top.appendChild(el("span", "chip cycle", "↻ " + c.cycle));
    card.appendChild(top);

    card.appendChild(el("h3", "card-title", c.title));

    if (c.source != null) {
      const traj = el("div", "card-meta");
      const srcGlyph = c.source === "recruited" ? "(○)" : "{△}";
      const tgtGlyph = c.target === "grounded" ? "[□]" : "{△}";
      traj.appendChild(el("span", "trajectory", srcGlyph + " → " + tgtGlyph));
      const z = zoneOf(c);
      if (z) traj.appendChild(el("span", "chip zone z" + z, "Z" + z));
      card.appendChild(traj);
      card.appendChild(renderScoreBar(c));
    }
    if (c.reliability != null) {
      const meta = el("div", "card-meta");
      meta.appendChild(el("span", "chip axiom", "Γ " + c.reliability.toFixed(2)));
      if (w[c.id] != null) meta.appendChild(el("span", "chip axiom", "w " + w[c.id].toFixed(2)));
      if (c.agent) meta.appendChild(el("span", "chip scale", c.agent));
      const sched = colById(c.column);
      if (sched && state.boardId === "dialectic") meta.appendChild(el("span", "chip tag", sched.step));
      card.appendChild(meta);
    }
    if (c.genesis) {
      const meta = el("div", "card-meta");
      meta.appendChild(el("span", "chip gen", "⋗ " + c.genesis));
      card.appendChild(meta);
    }
    if (c.tags && c.tags.length) {
      const tg = el("div", "card-meta");
      c.tags.forEach((t) => tg.appendChild(el("span", "chip tag", t)));
      card.appendChild(tg);
    }

    const ev = evaluate(c);
    if (ev) {
      card.appendChild(el("div", "chip verdict " + ev.key + (col.id === "gate" ? " pulse" : ""), ev.label));
    }
    if (c.pathology) {
      const layer = R.pathologyLayers.find((p) => p.id === c.pathology);
      card.appendChild(el("div", "chip pathology", "✕ " + (layer ? layer.name : c.pathology) + " layer"));
    }
    if (c.death) {
      const dm = R.deathModes.find((d) => d.id === c.death);
      card.appendChild(el("div", "chip death", "† " + (dm ? dm.name : c.death)));
    }

    const actions = el("div", "card-actions");
    const cols = currentBoard().columns;
    const idx = cols.findIndex((x) => x.id === c.column);
    if (idx > 0) actions.appendChild(moveBtn("‹", c.id, cols[idx - 1].id));
    if (idx < cols.length - 1) actions.appendChild(moveBtn("›", c.id, cols[idx + 1].id));
    if (c.column === "closure" && state.boardId === "protocol") {
      const cyc = el("button", "move-btn cycle-btn", "↻ return");
      cyc.title = "Recursive closure: return to Initiation (cycle +1)";
      cyc.addEventListener("click", (ev2) => {
        ev2.stopPropagation();
        c.cycle = (c.cycle || 0) + 1;
        const from = c.column;
        c.column = "initiation";
        pushTrace(c, { action: "↻ recursive closure", from: colName(c.board, from), to: colName(c.board, "initiation"),
          note: "cycle " + c.cycle + " — the enlarged whole is re-sensed; state, R-topology and boundaries have shifted (A12)" });
        saveStore(); renderAll();
      });
      actions.appendChild(cyc);
    }
    card.appendChild(actions);

    card.addEventListener("dragstart", (ev2) => {
      ev2.dataTransfer.setData("text/plain", c.id);
      ev2.dataTransfer.effectAllowed = "move";
      card.classList.add("dragging");
    });
    card.addEventListener("dragend", () => card.classList.remove("dragging"));
    card.addEventListener("click", () => openModal(c.id));

    return card;
  }

  function renderScoreBar(c) {
    const S = scoreOf(c);
    const wrap = el("div", "score-bar");
    const clamp = (v) => Math.max(0, Math.min(1, v));
    const fill = el("div", "score-fill z" + zoneOf(c));
    fill.style.width = (clamp(S) * 100).toFixed(1) + "%";
    const theta = el("div", "score-theta");
    theta.style.left = (clamp(c.theta) * 100).toFixed(1) + "%";
    theta.title = "θ = " + c.theta.toFixed(2);
    wrap.appendChild(fill);
    wrap.appendChild(theta);
    const lbl = el("div", "score-label", "S " + S.toFixed(2) + "  vs  θ " + c.theta.toFixed(2));
    const box = el("div", "score-box");
    box.appendChild(wrap);
    box.appendChild(lbl);
    return box;
  }

  function moveBtn(txt, id, colId) {
    const b = el("button", "move-btn", txt);
    b.addEventListener("click", (ev) => { ev.stopPropagation(); moveCard(id, colId); });
    return b;
  }

  // entry-point creation: new cards enter at each board's first column only
  // (consistent with A13 — everything walks the workflow from its start)
  function renderNewCardForm(board) {
    const form = el("form", "new-card");
    const input = el("input", "new-title");
    input.required = true;
    const row = el("div", "new-row");
    const btn = el("button", "move-btn", "add");
    btn.type = "submit";
    const firstCol = board.columns[0].id;
    const extras = {};

    if (board.id === "protocol") {
      input.placeholder = "＋ New transformation…";
      extras.src = el("select");
      extras.src.innerHTML = '<option value="recruited">(○)-recruited</option><option value="initiated">{△}-initiated</option>';
      extras.tgt = el("select");
      extras.tgt.innerHTML = '<option value="grounded">[□]-grounded</option><option value="terminated">{△}-terminated</option>';
      row.appendChild(extras.src);
      row.appendChild(extras.tgt);
    } else if (board.id === "dialectic") {
      input.placeholder = "＋ New position / note…";
      extras.kind = el("select");
      extras.kind.innerHTML = '<option value="position">position</option><option value="note">moderator note</option>';
      extras.rel = el("input", "new-rel num");
      extras.rel.type = "number";
      extras.rel.min = "0"; extras.rel.max = "1"; extras.rel.step = "0.01"; extras.rel.value = "0.70";
      extras.rel.title = "Γ reliability (positions only)";
      row.appendChild(extras.kind);
      row.appendChild(extras.rel);
    } else {
      input.placeholder = "＋ New intervention…";
      extras.tags = el("input", "new-rel");
      extras.tags.type = "text";
      extras.tags.placeholder = "tags, comma-separated";
      row.appendChild(extras.tags);
    }
    row.appendChild(btn);
    form.appendChild(input);
    form.appendChild(row);

    form.addEventListener("submit", (ev) => {
      ev.preventDefault();
      const title = input.value.trim();
      if (!title) return;
      const id = "custom-" + state.customSeq++;

      if (board.id === "protocol") {
        const source = extras.src.value, target = extras.tgt.value;
        state.cards[id] = {
          id, board: board.id, column: firstCol, title,
          sign: source === "recruited" ? "⟨⦿○ ≈ ⊣○⟩" : "⟨⦿△ ≡ ⊣△⟩",
          signName: source === "recruited" ? "Qualisign–Icon–Rheme" : "Legisign–Symbol–Argument",
          source, target,
          rho: 0.80, delta: 0.10, gamma: 0.25, k: 3, theta: 0.40,
          scale: "meso", cycle: 0, pathology: null, death: null, axioms: ["A9"],
          note: "Practitioner-entered transformation. Metrics are initial estimates — revise them after [□]-encounter.",
        };
      } else if (board.id === "dialectic") {
        if (extras.kind.value === "position") {
          const agent = nextAgent(board.id);
          const rel = Math.max(0, Math.min(1, parseFloat(extras.rel.value) || 0.70));
          state.cards[id] = {
            id, board: board.id, column: firstCol, kind: "position", agent, title,
            sign: "⟨⦿△ ≡ ⊣α⟩", signName: "Legisign–Symbol–Abduction",
            reliability: rel, cycle: 0, genesis: null, axioms: ["A19"],
            note: "Practitioner-entered position. Γ starts at " + rel.toFixed(2) +
                  "; the moderator updates it by EMA as scores arrive (§19), and w is recomputed across the board.",
          };
        } else {
          state.cards[id] = {
            id, board: board.id, column: firstCol, kind: "note", title,
            sign: "⟹ᵐⁿᵃᵛ¹ ⊗ |ᵍ ⊗ ⟹ᵐᵉᵐ", signName: "moderator note",
            reliability: null, cycle: 0, axioms: ["A16"],
            note: "Practitioner-entered note.",
          };
        }
      } else {
        state.cards[id] = {
          id, board: board.id, column: firstCol, title,
          sign: "⟨⦿□ ⇢ ⊣□⟩", signName: "Sinsign–Index–Dicisign",
          cycle: 0,
          tags: extras.tags.value.split(",").map((t) => t.trim()).filter(Boolean),
          axioms: ["ML"],
          note: "Practitioner-entered intervention.",
        };
      }
      pushTrace(state.cards[id], { action: "enters the workflow", from: "—",
        to: colName(board.id, firstCol), note: "created at the entry point" });
      saveStore();
      renderAll();
    });
    return form;
  }

  /* -------------------------------------------------------------- modal */

  const $modal = document.getElementById("modal");
  const $modalBody = document.getElementById("modal-body");
  let viewEl = null; // dynamic region, rebuilt on every edit
  let openCardId = null;

  function openModal(id, focus) {
    const c = state.cards[id];
    if (!c) return;
    openCardId = id;
    $modalBody.innerHTML = "";

    viewEl = el("div", "m-view");
    $modalBody.appendChild(viewEl);
    refreshView(c);

    $modalBody.appendChild(renderEditor(c));

    $modal.classList.add("open");

    if (focus) {
      const target = $modalBody.querySelector('[data-section="' + focus + '"]');
      if (target) {
        target.classList.add("flash");
        target.scrollIntoView({ block: "center" });
        const inp = target.querySelector("input");
        if (inp) inp.focus();
      }
    }
  }

  function refreshView(c) {
    if (!viewEl || openCardId !== c.id) return;
    viewEl.innerHTML = "";

    const head = el("div", "m-head");
    head.appendChild(el("span", "sign chip big", c.sign));
    const hw = el("div");
    hw.appendChild(el("h2", "m-title", c.title));
    if (c.signName) hw.appendChild(el("div", "m-sub", c.signName));
    head.appendChild(hw);
    viewEl.appendChild(head);

    // lineage: where this card was promoted from
    if (c.origin) {
      const ob = QPT_DATA.boards.find((b) => b.id === c.origin.board);
      const oc = ob && ob.columns.find((x) => x.id === c.origin.column);
      viewEl.appendChild(el("div", "m-origin",
        "⇧ promoted from " + (ob ? ob.title : c.origin.board) + " · " + (oc ? oc.name : c.origin.column)));
    }

    // naming trajectory (§12) — live classification, warns upstream of the gate
    if (c.source != null) {
      const nc = namingClass(c);
      const nrow = el("div", "m-naming");
      nrow.appendChild(el("span", "chip verdict " + nc.key, nc.label));
      const gi = colIndex("gate");
      const upstream = state.boardId === "protocol" && colIndex(c.column) >= 0 && colIndex(c.column) < gi;
      if (upstream) {
        nrow.appendChild(el("span", "ed-warn inline",
          "§12 · naming upstream of gate passage tends to dead naming ({△}{△} — organizational, never generative)."));
      }
      viewEl.appendChild(nrow);
    }

    const ev = evaluate(c);
    if (ev) viewEl.appendChild(renderGateView(c, ev));
    if (c.reliability != null || c.kind === "note") viewEl.appendChild(renderDialecticView(c));
    viewEl.appendChild(renderPathologyPicker(c));
    viewEl.appendChild(renderDeathPicker(c));
    viewEl.appendChild(renderTrace(c));

    if (c.axioms && c.axioms.length) {
      const n = el("div", "m-section");
      n.appendChild(el("h4", null, "Registry"));
      const row = el("div", "m-axioms");
      c.axioms.forEach((aid) => {
        const ax = R.axioms.find((a) => a.id === aid);
        const chip = el("button", "chip axiom link", aid);
        chip.title = ax ? ax.name + " — " + ax.body : aid;
        chip.addEventListener("click", () => { closeModal(); openDrawer("axioms", aid); });
        row.appendChild(chip);
      });
      n.appendChild(row);
      viewEl.appendChild(n);
    }
  }

  function renderGateView(c, ev) {
    const sec = el("div", "m-section");
    sec.dataset.section = "gate";
    sec.appendChild(el("h4", null, "Quality Gate — 𝒢 ≡ R ⊓ G  (§7)"));

    const verdictRow = el("div", "m-verdict");
    verdictRow.appendChild(el("span", "chip verdict big " + ev.key, ev.label));
    verdictRow.appendChild(el("span", "m-verdict-sub", ev.sub));
    sec.appendChild(verdictRow);

    // trajectory matrix (§7.1) — click a cell to re-aim the trajectory
    const m = R.gateMatrix;
    const grid = el("div", "matrix editable");
    grid.appendChild(el("div", "mx corner", "source ↓ · target →"));
    m.cols.forEach((cn) => grid.appendChild(el("div", "mx axis", cn)));
    m.rows.forEach((rn, ri) => {
      grid.appendChild(el("div", "mx axis", rn));
      m.cells[ri].forEach((cell, ci) => {
        const isHere = (ri === 0) === (c.source === "recruited") && (ci === 0) === (c.target === "grounded");
        const d = el("div", "mx cell " + cell.verdict + (isHere ? " here" : ""), cell.name);
        d.title = "Click to set this trajectory";
        d.addEventListener("click", () => {
          c.source = ri === 0 ? "recruited" : "initiated";
          c.target = ci === 0 ? "grounded" : "terminated";
          applyEdit(c);
        });
        grid.appendChild(d);
      });
    });
    sec.appendChild(grid);

    const S = scoreOf(c);
    sec.appendChild(el("div", "equation",
      "S = ρ − δ − γ·ln k = " + c.rho.toFixed(2) + " − " + c.delta.toFixed(2) +
      " − " + c.gamma.toFixed(2) + "·ln(" + c.k + ") = " + S.toFixed(3) +
      (ev.quant ? "  ≥  θ " : "  <  θ ") + c.theta.toFixed(2)));
    sec.appendChild(renderScoreBar(c));

    const P = sigmoidP(c);
    sec.appendChild(el("div", "score-label",
      "P(anchored | S) = σ(a(S − θ)) = " + (P * 100).toFixed(1) + "%   ·   Zone " + zoneOf(c) +
      " — " + ["", "prior-driven drift", "phase transition", "anchored control"][zoneOf(c)]));

    const conj = el("div", "conjuncts");
    conj.appendChild(el("span", "chip " + (ev.R ? "ok" : "bad"), "R recruitment " + (ev.R ? "✓" : "✗")));
    conj.appendChild(el("span", "chip " + (ev.G ? "ok" : "bad"), "G grounding " + (ev.G ? "✓" : "✗")));
    conj.appendChild(el("span", "chip " + (ev.quant ? "ok" : "bad"), "S ≥ θ " + (ev.quant ? "✓" : "✗")));
    sec.appendChild(conj);

    return sec;
  }

  function renderDialecticView(c) {
    const sec = el("div", "m-section");
    sec.appendChild(el("h4", null, "Dialectics (§19)"));
    const row = el("div", "conjuncts");
    if (c.agent) row.appendChild(el("span", "chip axiom", "agent " + c.agent));
    if (c.reliability != null) {
      row.appendChild(el("span", "chip axiom", "Γ = " + c.reliability.toFixed(2)));
      const w = weights()[c.id];
      if (w != null) row.appendChild(el("span", "chip axiom", "w = " + w.toFixed(3)));
    }
    const sched = colById(c.column);
    if (sched && state.boardId === "dialectic") {
      row.appendChild(el("span", "chip tag", sched.step + " (scheduled)"));
    }
    if (c.genesis) row.appendChild(el("span", "chip gen", "⋗ " + c.genesis));
    sec.appendChild(row);
    return sec;
  }

  // §13 — click a layer to locate the failure; click again to clear
  function renderPathologyPicker(c) {
    const sec = el("div", "m-section");
    sec.dataset.section = "pathology";
    sec.appendChild(el("h4", null, "Five-layer diagnostic (§13) — click to locate the failure"));
    const ladder = el("ol", "ladder");
    R.pathologyLayers.forEach((p) => {
      const li = el("li", p.id === c.pathology ? "hit" : "");
      li.appendChild(el("span", "lad-name", p.name));
      li.appendChild(el("span", "lad-q", p.question));
      li.addEventListener("click", () => {
        c.pathology = c.pathology === p.id ? null : p.id;
        applyEdit(c);
      });
      ladder.appendChild(li);
    });
    sec.appendChild(ladder);
    if (c.pathology === "temporal") {
      const tp = el("div", "conjuncts");
      R.temporalPathologies.forEach((t) => tp.appendChild(el("span", "chip pathology", t.name)));
      sec.appendChild(tp);
    }
    return sec;
  }

  // §9.2 — the card's own post-hoc articulation of its navigation
  function renderTrace(c) {
    const sec = el("div", "m-section");
    sec.appendChild(el("h4", null, "Trace — post-hoc articulation (§9.2)"));
    if (!c.trace || !c.trace.length) {
      sec.appendChild(el("p", "m-note dim",
        "No transitions yet — the trace begins with the next gate passage. A generative sequence is a trace, not a plan."));
      return sec;
    }
    const ol = el("ol", "trace-list");
    c.trace.forEach((t) => {
      const li = el("li");
      li.appendChild(el("span", "tr-n", String(t.n)));
      const body = el("span", "tr-body");
      const head = el("span", "tr-head");
      head.appendChild(el("span", "tr-action", t.action));
      head.appendChild(el("span", "tr-path", t.from + " → " + t.to));
      body.appendChild(head);
      if (t.note) body.appendChild(el("span", "tr-note", t.note));
      li.appendChild(body);
      ol.appendChild(li);
    });
    sec.appendChild(ol);
    return sec;
  }

  // §11 — click a death mode to diagnose structural death
  function renderDeathPicker(c) {
    const sec = el("div", "m-section");
    sec.appendChild(el("h4", null, "Structural death (§11) — click to diagnose"));
    const row = el("div", "death-picker");
    R.deathModes.forEach((d) => {
      const btn = el("button", "death-opt" + (c.death === d.id ? " hit" : ""));
      btn.appendChild(el("span", "do-name", d.name));
      btn.appendChild(el("span", "do-q", "“" + d.question + "”"));
      btn.addEventListener("click", () => {
        c.death = c.death === d.id ? null : d.id;
        applyEdit(c);
      });
      row.appendChild(btn);
    });
    sec.appendChild(row);
    if (c.death) {
      const dm = R.deathModes.find((d) => d.id === c.death);
      sec.appendChild(el("p", "m-note",
        dm.body + " Boundary protocol: diagnose through contact; strengthen the living side first; minimum dissolution."));
    }
    return sec;
  }

  /* -------------------------------------------------------------- editor */

  function renderEditor(c) {
    const sec = el("div", "m-section editor");
    sec.appendChild(el("h4", null, "Edit — constrained by the specification"));

    // title → naming classification (§12) is rendered live in the view region
    const titleRow = edRow("Name {△}");
    const titleIn = el("input", "in");
    titleIn.type = "text";
    titleIn.value = c.title;
    titleRow.appendChild(titleIn);
    sec.appendChild(titleRow);
    titleIn.addEventListener("change", () => {
      c.title = titleIn.value.trim() || c.title;
      titleIn.value = c.title;
      applyEdit(c);
    });

    // sign class
    if (c.sign != null) {
      const row = edRow("Sign class");
      const sel = el("select", "in");
      const known = R.signClasses.map((s) => s.cls);
      const options = known.includes(c.sign) ? R.signClasses : [{ cls: c.sign, name: c.signName || c.sign }].concat(R.signClasses);
      options.forEach((s) => {
        const o = el("option", null, s.cls + "  " + (s.name || ""));
        o.value = s.cls;
        if (s.cls === c.sign) o.selected = true;
        sel.appendChild(o);
      });
      sel.addEventListener("change", () => {
        c.sign = sel.value;
        const hit = R.signClasses.find((s) => s.cls === sel.value);
        c.signName = hit ? hit.name : c.signName;
        applyEdit(c);
      });
      row.appendChild(sel);
      sec.appendChild(row);
    }

    // anchoring metrics (§8) — domain-clamped sliders
    if (c.rho != null) {
      [
        ["rho",   "ρ support",      0, 1,  0.01],
        ["delta", "δ mismatch",     0, 1,  0.01],
        ["gamma", "γ penalty",      0, 1,  0.01],
        ["k",     "k anchors",      1, 16, 1],
        ["theta", "θ threshold",    0, 1,  0.01],
      ].forEach(([key, label, min, max, step]) => {
        sec.appendChild(sliderRow(c, key, label, min, max, step));
      });
    }

    // scale (§10)
    if (c.scale != null) {
      const row = edRow("Scale");
      const sel = el("select", "in");
      ["micro", "meso", "macro"].forEach((s) => {
        const o = el("option", null, s);
        o.value = s;
        if (s === c.scale) o.selected = true;
        sel.appendChild(o);
      });
      const hint = el("span", "ed-hint", SCALE_HINTS[c.scale] || "");
      sel.addEventListener("change", () => {
        c.scale = sel.value;
        hint.textContent = SCALE_HINTS[c.scale] || "";
        applyEdit(c);
      });
      row.appendChild(sel);
      row.appendChild(hint);
      sec.appendChild(row);
    }

    // reliability Γ (§19)
    if (c.reliability != null) {
      sec.appendChild(sliderRow(c, "reliability", "Γ reliability", 0, 1, 0.01));
    }

    // genesis property (A7) — required to enter Synthesize
    if (state.boardId === "dialectic" && c.kind !== "note") {
      const row = edRow("Genesis ⋗");
      row.dataset.section = "genesis";
      const inp = el("input", "in");
      inp.type = "text";
      inp.placeholder = "emergent property present in no parent (A7)…";
      inp.value = c.genesis || "";
      inp.addEventListener("change", () => {
        c.genesis = inp.value.trim() || null;
        applyEdit(c);
      });
      row.appendChild(inp);
      sec.appendChild(row);
      const hint = el("div", "ed-hint block",
        "Required for passage into Synthesize: ∃ property([□]⎕ₑw) ∉ { [□]⎕₁, [□]⎕₂, {△}⎕_mediation }.");
      hint.dataset.section = "genesis";
      sec.appendChild(hint);
    }

    // tags
    if (c.tags != null) {
      const row = edRow("Tags");
      const inp = el("input", "in");
      inp.type = "text";
      inp.value = (c.tags || []).join(", ");
      inp.placeholder = "comma-separated…";
      inp.addEventListener("change", () => {
        c.tags = inp.value.split(",").map((t) => t.trim()).filter(Boolean);
        applyEdit(c);
      });
      row.appendChild(inp);
      sec.appendChild(row);
    }

    // field note
    const noteRow = edRow("Field note");
    const ta = el("textarea", "in");
    ta.rows = 3;
    ta.value = c.note || "";
    ta.addEventListener("change", () => {
      c.note = ta.value;
      applyEdit(c);
    });
    noteRow.appendChild(ta);
    sec.appendChild(noteRow);

    // promotion between boards (the meta-workflow)
    const targets = promotionTargets(c);
    if (targets.length) {
      const prow = el("div", "m-section promote-sec");
      prow.appendChild(el("h4", null, "Promote"));
      targets.forEach((t) => {
        const b = el("button", "promote-btn", t.label);
        b.dataset.promote = t.board + ":" + t.column;
        b.title = t.why;
        b.addEventListener("click", () => promote(c, t));
        prow.appendChild(b);
        prow.appendChild(el("div", "ed-hint block", t.why));
      });
      sec.appendChild(prow);
    }

    // dissolve (§11 minimum dissolution)
    const delRow = el("div", "ed-row end");
    const del = el("button", "danger-btn", "Dissolve card");
    let armed = false;
    del.addEventListener("click", () => {
      if (!armed) {
        armed = true;
        del.textContent = "Confirm dissolution";
        del.classList.add("armed");
        setTimeout(() => { armed = false; del.textContent = "Dissolve card"; del.classList.remove("armed"); }, 3000);
        return;
      }
      delete state.cards[c.id];
      pendingMove = null;
      saveStore();
      closeModal();
      renderAll();
      notice(c.death
        ? "§11 · Minimum dissolution — remove one constraint, wait, sense reorganization, repeat."
        : "Card dissolved.");
    });
    delRow.appendChild(del);
    sec.appendChild(delRow);

    return sec;
  }

  function edRow(label) {
    const row = el("div", "ed-row");
    row.appendChild(el("label", "ed-label", label));
    return row;
  }

  function sliderRow(c, key, label, min, max, step) {
    const row = edRow(label);
    const inp = el("input", "range");
    inp.type = "range";
    inp.min = min; inp.max = max; inp.step = step;
    inp.value = c[key];
    const val = el("span", "ed-val", Number(c[key]).toFixed(step >= 1 ? 0 : 2));
    inp.addEventListener("input", () => {
      c[key] = step >= 1 ? parseInt(inp.value, 10) : parseFloat(inp.value);
      val.textContent = Number(c[key]).toFixed(step >= 1 ? 0 : 2);
      applyEdit(c);
    });
    row.appendChild(inp);
    row.appendChild(val);
    return row;
  }

  function closeModal() {
    $modal.classList.remove("open");
    pendingMove = null;
    openCardId = null;
    viewEl = null;
  }

  /* -------------------------------------------------------------- drawer */

  const $drawer = document.getElementById("drawer");
  const $drawerBody = document.getElementById("drawer-body");
  const $drawerSearch = document.getElementById("drawer-search");
  let drawerTab = "gate";

  function openDrawer(tab, highlightId) {
    drawerTab = tab || drawerTab;
    renderDrawer(highlightId);
    $drawer.classList.add("open");
  }
  function closeDrawer() { $drawer.classList.remove("open"); }

  function renderDrawer(highlightId) {
    document.querySelectorAll(".d-tab").forEach((t) => {
      t.classList.toggle("active", t.dataset.tab === drawerTab);
    });
    const q = ($drawerSearch.value || "").trim().toLowerCase();
    $drawerBody.innerHTML = "";
    const match = (txt) => !q || (txt || "").toLowerCase().includes(q);

    const sections = {
      semantics: () => {
        addSection("The rules of play — what moves and promotions mean");
        R.semantics.forEach((g) => {
          addSection(g.group);
          g.rows.forEach((r) => {
            if (match(r.left + r.body)) $drawerBody.appendChild(refRow("axiom", r.left, r.body, null, false));
          });
        });
      },
      gate: () => {
        addSection("The operator on which everything turns (§7)");
        R.formulas.forEach((f) => {
          if (match(f.name + f.body)) $drawerBody.appendChild(refRow("formula", f.name, f.body, f.ref, f.id === highlightId));
        });
        addSection("Gate matrix — trajectory, not content (§7.1)");
        const m = R.gateMatrix;
        const grid = el("div", "matrix");
        grid.appendChild(el("div", "mx corner", "source ↓ · target →"));
        m.cols.forEach((cn) => grid.appendChild(el("div", "mx axis", cn)));
        m.rows.forEach((rn, ri) => {
          grid.appendChild(el("div", "mx axis", rn));
          m.cells[ri].forEach((cell) => grid.appendChild(el("div", "mx cell " + cell.verdict, cell.name)));
        });
        $drawerBody.appendChild(grid);
        $drawerBody.appendChild(el("p", "m-note", m.note));
        addSection("Three zones (§8.1)");
        [["Z1", "S ≪ θ — prior-driven drift, (○)-dominant"],
         ["Z2", "S ≈ θ — phase transition, ((○)[□])-tension"],
         ["Z3", "S ≫ θ — anchored control, {△}-dominant"]].forEach(([z, txt]) => {
          $drawerBody.appendChild(refRow("plain", z, txt, null, false));
        });
      },
      axioms: () => {
        addSection("Consolidated axiom & law registry (Part IX)");
        R.axioms.forEach((a) => {
          if (match(a.id + a.name + a.body)) {
            $drawerBody.appendChild(refRow("axiom", a.id + " — " + a.name, a.body, "Part " + a.part, a.id === highlightId));
          }
        });
      },
      signs: () => {
        addSection("The three categories (§1)");
        R.categories.forEach((c) => {
          if (match(c.glyph + c.name + c.body)) $drawerBody.appendChild(refRow("glyph", c.glyph, c.name + " — " + c.body, c.ref, false));
        });
        addSection("Ten trichotomies (§3)");
        R.trichotomies.forEach((t) => {
          if (match(t.aspect + t.first + t.second + t.third)) {
            $drawerBody.appendChild(refRow("glyph", t.glyph, t.n + ". " + t.aspect + " — " + t.first + " / " + t.second + " / " + t.third, null, false));
          }
        });
        addSection("Core sign classes (§4)");
        R.signClasses.forEach((s) => {
          if (match(s.cls + s.name)) $drawerBody.appendChild(refRow("glyph", s.cls, s.name + " — " + s.body, null, false));
        });
      },
      agency: () => {
        addSection("Eight canonical loops (§30)");
        R.loops.forEach((l) => {
          if (match(l.glyph + l.name + l.body)) $drawerBody.appendChild(refRow("glyph", l.glyph, l.name + " — " + l.body, null, false));
        });
        addSection("25 agency dimensions (§31)");
        R.dimensions.forEach((d) => {
          if (match(d.name + d.cls + d.family)) {
            $drawerBody.appendChild(refRow("dim", d.n + ". " + d.name, d.cls + " · " + d.family, null, false));
          }
        });
      },
      failure: () => {
        addSection("Forces of the failure field (§26)");
        R.failureForces.forEach((f) => {
          if (match(f.glyph + f.name + f.body)) $drawerBody.appendChild(refRow("glyph", f.glyph, f.name + " — " + f.body, null, false));
        });
        addSection("Five-layer pathology diagnostic (§13)");
        R.pathologyLayers.forEach((p) => {
          if (match(p.name + p.question)) $drawerBody.appendChild(refRow("plain", p.name, p.question, null, false));
        });
        addSection("Temporal pathologies (§13)");
        R.temporalPathologies.forEach((t) => {
          if (match(t.name + t.body)) $drawerBody.appendChild(refRow("plain", t.name, t.body, null, false));
        });
        addSection("Three modes of structural death (§11)");
        R.deathModes.forEach((d) => {
          if (match(d.name + d.body)) $drawerBody.appendChild(refRow("plain", d.name, "“" + d.question + "” " + d.body, null, false));
        });
      },
      drill: () => renderDrill(),
    };
    (sections[drawerTab] || sections.gate)();

    if (highlightId) {
      const node = $drawerBody.querySelector(".highlight");
      if (node) node.scrollIntoView({ block: "center" });
    }
  }

  function addSection(title) { $drawerBody.appendChild(el("h3", "d-section", title)); }

  function refRow(kind, left, body, ref, highlight) {
    const row = el("div", "ref-row " + kind + (highlight ? " highlight" : ""));
    row.appendChild(el("span", "ref-left", left));
    row.appendChild(el("span", "ref-body", body));
    if (ref) row.appendChild(el("span", "ref-ref", ref));
    return row;
  }

  /* ----------------------------------------------- learn: tour, practice, drill */

  const LEARN_KEY = "qpt-learn-v1";
  let learn = { tourSeen: false, done: {}, drill: { knew: 0, missed: 0 } };
  let activeExercise = null;
  let drillState = null;
  let tourIdx = -1;

  function learnLoad() {
    try {
      const s = JSON.parse(localStorage.getItem(LEARN_KEY));
      if (s) learn = Object.assign(learn, s);
    } catch (e) {}
  }
  function learnSave() {
    try { localStorage.setItem(LEARN_KEY, JSON.stringify(learn)); } catch (e) {}
  }

  /* guided tour — an overlay that walks the board, spec refs included */

  function startTour() {
    tourIdx = 0;
    document.getElementById("tour").classList.add("open");
    showTourStep();
  }

  function endTour() {
    document.getElementById("tour").classList.remove("open");
    tourIdx = -1;
    learn.tourSeen = true;
    learnSave();
  }

  function showTourStep() {
    const steps = QPT_DATA.learn.tour;
    const st = steps[tourIdx];
    if (st.board && st.board !== state.boardId) {
      state.boardId = st.board;
      saveStore();
      renderAll();
    }
    const tgt = document.querySelector(st.sel);
    const hl = document.getElementById("tour-highlight");
    const box = document.getElementById("tour-box");
    document.getElementById("tour-title").textContent = st.title;
    document.getElementById("tour-body").textContent = st.body;
    document.getElementById("tour-count").textContent = (tourIdx + 1) + " / " + steps.length;
    document.getElementById("tour-back").disabled = tourIdx === 0;
    document.getElementById("tour-next").textContent = tourIdx === steps.length - 1 ? "Finish" : "Next";

    if (tgt) {
      tgt.scrollIntoView({ block: "nearest", inline: "center" });
      const r = tgt.getBoundingClientRect();
      const pad = 6;
      hl.style.display = "block";
      hl.style.left = (r.left - pad) + "px";
      hl.style.top = (r.top - pad) + "px";
      hl.style.width = (r.width + pad * 2) + "px";
      hl.style.height = (r.height + pad * 2) + "px";
      const bw = Math.min(340, window.innerWidth - 24);
      box.style.width = bw + "px";
      const below = r.bottom + 14 + 170 < window.innerHeight;
      box.style.left = Math.max(12, Math.min(r.left, window.innerWidth - bw - 12)) + "px";
      box.style.top = below ? (r.bottom + 14) + "px" : Math.max(12, r.top - 14 - box.offsetHeight) + "px";
    } else {
      hl.style.display = "none";
      box.style.width = "340px";
      box.style.left = ((window.innerWidth - 340) / 2) + "px";
      box.style.top = (window.innerHeight * 0.3) + "px";
    }
  }

  /* practice — exercises graded by the specification's own constraints */

  function openPractice() {
    const list = document.getElementById("learn-list");
    list.innerHTML = "";
    const feat = el("div", "ex-row sim-feature");
    const fhead = el("div", "ex-head");
    fhead.appendChild(el("span", "ex-check", "▶"));
    fhead.appendChild(el("span", "ex-title", "Guided simulation — the full navigation"));
    ["§14", "§15", "§19", "A7"].forEach((r) => fhead.appendChild(el("span", "chip axiom", r)));
    const fbtn = el("button", "move-btn", "watch");
    fbtn.addEventListener("click", startSimulation);
    fhead.appendChild(fbtn);
    feat.appendChild(fhead);
    feat.appendChild(el("p", "ex-goal",
      "Watch one card traverse the whole meta-workflow: initiation, encounter, a refused gate, diagnosis, passage, naming, closure, promotion, the dialectic, genesis — and home. 16 narrated steps; your board is restored afterward."));
    list.appendChild(feat);
    QPT_DATA.learn.exercises.forEach((ex) => {
      const row = el("div", "ex-row" + (learn.done[ex.id] ? " done" : ""));
      const head = el("div", "ex-head");
      head.appendChild(el("span", "ex-check", learn.done[ex.id] ? "✓" : "○"));
      head.appendChild(el("span", "ex-title", ex.title));
      ex.refs.forEach((r) => head.appendChild(el("span", "chip axiom", r)));
      const btn = el("button", "move-btn", learn.done[ex.id] ? "replay" : "start");
      btn.dataset.ex = ex.id;
      btn.addEventListener("click", () => startExercise(ex));
      head.appendChild(btn);
      row.appendChild(head);
      row.appendChild(el("p", "ex-goal", ex.goal));
      list.appendChild(row);
    });
    document.getElementById("learn-modal").classList.add("open");
  }

  function closePractice() { document.getElementById("learn-modal").classList.remove("open"); }

  function startExercise(ex) {
    const c = clone(ex.setup);
    c.board = ex.board;
    c.trace = [];
    pushTrace(c, { action: "enters the workflow", from: "—",
      to: colName(ex.board, ex.setup.column), note: "practice card — " + ex.title });
    state.cards[c.id] = c;               // restart = fresh card, same id
    state.boardId = ex.board;
    activeExercise = ex;
    saveStore();
    renderAll();
    closePractice();
    const gb = document.getElementById("goal-banner");
    document.getElementById("gb-title").textContent = ex.title;
    document.getElementById("gb-goal").textContent = ex.goal;
    gb.classList.remove("hidden");
    notice("Exercise started — the board itself is the grader.");
  }

  function quitExercise() {
    activeExercise = null;
    document.getElementById("goal-banner").classList.add("hidden");
  }

  function checkExercise() {
    if (!activeExercise) return;
    const c = state.cards[activeExercise.setup.id];
    if (c && activeExercise.done(c)) {
      const ex = activeExercise;
      learn.done[ex.id] = true;
      learnSave();
      quitExercise();
      notice("✓ " + ex.title + " — " + ex.explain);
    }
  }

  /* drill — flashcards over the registry (drawer tab) */

  function drillPool() {
    const P = [];
    R.axioms.forEach((a) => P.push({ q: a.id + " — " + a.name, a: a.body, ref: "Part " + a.part }));
    R.categories.forEach((c) => P.push({ q: c.glyph, a: c.name + " — " + c.body, ref: c.ref }));
    R.trichotomies.forEach((t) => P.push({ q: t.glyph + "  " + t.aspect, a: t.first + "  /  " + t.second + "  /  " + t.third, ref: "§3" }));
    R.formulas.forEach((f) => P.push({ q: f.name, a: f.body, ref: f.ref }));
    R.dimensions.forEach((d) => P.push({ q: d.name, a: d.cls + "  ·  " + d.family, ref: "§31" }));
    R.failureForces.forEach((f) => P.push({ q: f.glyph, a: f.name + " — " + f.body, ref: "§26" }));
    R.loops.forEach((l) => P.push({ q: l.glyph, a: l.name + " — " + l.body, ref: "§30" }));
    return P;
  }

  function shuffle(arr) {
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [arr[i], arr[j]] = [arr[j], arr[i]];
    }
    return arr;
  }

  function renderDrill() {
    if (!drillState) {
      const pool = drillPool();
      drillState = { pool, order: shuffle(pool.map((_, i) => i)), idx: 0, revealed: false };
    }
    const st = drillState;
    const card = st.pool[st.order[st.idx]];

    const wrap = el("div", "drill");
    const meta = el("div", "drill-meta");
    meta.appendChild(el("span", null, "Card " + (st.idx + 1) + " / " + st.pool.length + (card.ref ? " · " + card.ref : "")));
    meta.appendChild(el("span", null, "✓ " + learn.drill.knew + "   ✗ " + learn.drill.missed));
    wrap.appendChild(meta);

    wrap.appendChild(el("div", "drill-q", card.q));
    const ans = el("div", "drill-a" + (st.revealed ? "" : " hidden"), card.a);
    wrap.appendChild(ans);

    const btns = el("div", "drill-btns");
    if (!st.revealed) {
      const rev = el("button", "btn", "Reveal");
      rev.addEventListener("click", () => { st.revealed = true; renderDrawer(); });
      btns.appendChild(rev);
    } else {
      const knew = el("button", "btn drill-knew", "Knew it");
      knew.addEventListener("click", () => drillNext(true));
      const missed = el("button", "btn drill-missed", "Missed it");
      missed.addEventListener("click", () => drillNext(false));
      btns.appendChild(knew);
      btns.appendChild(missed);
    }
    const sh = el("button", "move-btn", "reshuffle");
    sh.addEventListener("click", () => { drillState = null; renderDrawer(); });
    btns.appendChild(sh);
    wrap.appendChild(btns);
    $drawerBody.appendChild(wrap);
  }

  function drillNext(knew) {
    if (knew) learn.drill.knew++; else learn.drill.missed++;
    learnSave();
    drillState.idx = (drillState.idx + 1) % drillState.order.length;
    if (drillState.idx === 0) {
      drillState.order = shuffle(drillState.order);
      notice("Drill deck complete — reshuffled.");
    }
    drillState.revealed = false;
    renderDrawer();
  }

  /* simulation — a narrated, self-driving scenario over the real board code */

  const simApi = {
    move: (id, col) => moveCard(id, col),
    edit: (id, fn) => { const c = state.cards[id]; if (c) { fn(c); applyEdit(c); } },
    open: (id, focus) => openModal(id, focus),
    close: () => closeModal(),
    promoteTo: (id, boardId) => {
      const c = state.cards[id];
      if (!c) return;
      const t = promotionTargets(c).find((x) => x.board === boardId);
      if (t) promote(c, t);
    },
    createSimCard: () => {
      state.cards.sim1 = {
        id: "sim1", board: "protocol", column: "initiation",
        title: "Something is off in onboarding",
        sign: "⟨⦿○ ≈ ⊣○⟩", signName: "Qualisign–Icon–Rheme",
        source: "recruited", target: "terminated",
        rho: 0.85, delta: 0.30, gamma: 0.25, k: 4, theta: 0.50,
        scale: "meso", cycle: 0, pathology: null, death: null, axioms: ["A9", "A11"],
        note: "A hunch, recorded indexically. Nothing measured yet.",
        trace: [],
      };
      pushTrace(state.cards.sim1, { action: "enters the workflow", from: "—", to: "Initiation",
        note: "recorded indexically — attention captured, not scanned" });
      saveStore();
      renderAll();
    },
  };

  function clearSpot() {
    document.querySelectorAll(".sim-spot").forEach((n) => n.classList.remove("sim-spot"));
  }

  function startSimulation() {
    sim.snapshot = { cards: clone(state.cards), boardId: state.boardId };
    pendingMove = null;
    quitExercise();
    closeModal();
    closePractice();
    sim.active = true;
    sim.idx = -1;
    sim.playing = false;
    sim.quiet = false;
    document.getElementById("sim-auto").textContent = "Auto ▶";
    document.getElementById("sim-bar").classList.remove("hidden");
    simStep(1);
  }

  function simStep(dir) {
    if (!sim.active) return;
    const steps = QPT_DATA.learn.simulation;
    const ni = sim.idx + dir;
    if (ni < 0) return;
    if (ni >= steps.length) { endSimulation(); return; }
    sim.idx = ni;
    const st = steps[ni];
    if (st.board && st.board !== state.boardId) {
      state.boardId = st.board;
      saveStore();
      renderAll();
    }
    clearSpot();
    if (st.run) st.run(simApi);
    if (st.sel) {
      const node = document.querySelector(st.sel);
      if (node) node.classList.add("sim-spot");
    }
    document.getElementById("sim-count").textContent = (ni + 1) + " / " + steps.length;
    document.getElementById("sim-text").textContent = st.text;
    document.getElementById("sim-back").disabled = ni === 0;
    document.getElementById("sim-next").textContent = ni === steps.length - 1 ? "Finish" : "Next";
    if (sim.playing) {
      clearTimeout(sim.timer);
      sim.timer = setTimeout(() => simStep(1), 6500);
    }
  }

  // step back: restore the snapshot and replay quietly up to the target step
  function simBack() {
    if (sim.idx <= 0) return;
    const target = sim.idx - 1;
    sim.quiet = true;
    closeModal();
    pendingMove = null;
    state.cards = clone(sim.snapshot.cards);
    state.boardId = sim.snapshot.boardId;
    saveStore();
    renderAll();
    sim.idx = -1;
    for (let i = 0; i < target; i++) simStep(1);
    sim.quiet = false;
    simStep(1);
  }

  function endSimulation() {
    clearTimeout(sim.timer);
    sim.active = false;
    sim.playing = false;
    document.getElementById("sim-bar").classList.add("hidden");
    clearSpot();
    closeModal();
    pendingMove = null;
    if (sim.snapshot) {
      state.cards = clone(sim.snapshot.cards);
      state.boardId = sim.snapshot.boardId;
      saveStore();
      renderAll();
      notice("Simulation complete — your board has been restored.");
    }
    sim.snapshot = null;
    sim.idx = -1;
  }

  function toggleSimAuto() {
    sim.playing = !sim.playing;
    document.getElementById("sim-auto").textContent = sim.playing ? "Pause ⏸" : "Auto ▶";
    if (sim.playing) {
      clearTimeout(sim.timer);
      sim.timer = setTimeout(() => simStep(1), 6500);
    } else {
      clearTimeout(sim.timer);
    }
  }

  /* ---------------------------------------------------------------- wire */

  function renderAll() {
    renderTabs();
    renderContext();
    renderBoard();
  }

  document.getElementById("modal-close").addEventListener("click", closeModal);
  $modal.addEventListener("click", (e) => { if (e.target === $modal) closeModal(); });
  document.getElementById("drawer-close").addEventListener("click", closeDrawer);
  document.getElementById("ref-btn").addEventListener("click", () => openDrawer());
  document.getElementById("reset-btn").addEventListener("click", resetStore);
  document.getElementById("practice-btn").addEventListener("click", openPractice);
  document.getElementById("tour-btn").addEventListener("click", startTour);

  // theme toggle — persisted, applied to <html data-theme>
  const themeBtn = document.getElementById("theme-btn");
  function syncThemeBtn() {
    themeBtn.textContent = document.documentElement.dataset.theme === "light" ? "☾ Dark" : "☀ Light";
  }
  themeBtn.addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "light" ? "dark" : "light";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem("qpt-theme", next); } catch (e) {}
    syncThemeBtn();
  });
  syncThemeBtn();
  document.getElementById("learn-close").addEventListener("click", closePractice);
  document.getElementById("learn-modal").addEventListener("click", (e) => {
    if (e.target === document.getElementById("learn-modal")) closePractice();
  });
  document.getElementById("gb-hint").addEventListener("click", () => {
    if (activeExercise) notice("Hint — " + activeExercise.hint);
  });
  document.getElementById("gb-quit").addEventListener("click", quitExercise);
  document.getElementById("tour-next").addEventListener("click", () => {
    if (tourIdx < QPT_DATA.learn.tour.length - 1) { tourIdx++; showTourStep(); } else endTour();
  });
  document.getElementById("tour-back").addEventListener("click", () => {
    if (tourIdx > 0) { tourIdx--; showTourStep(); }
  });
  document.getElementById("tour-skip").addEventListener("click", endTour);
  document.getElementById("sim-btn").addEventListener("click", startSimulation);
  document.getElementById("sim-next").addEventListener("click", () => simStep(1));
  document.getElementById("sim-back").addEventListener("click", simBack);
  document.getElementById("sim-auto").addEventListener("click", toggleSimAuto);
  document.getElementById("sim-exit").addEventListener("click", endSimulation);
  window.addEventListener("resize", () => { if (tourIdx >= 0) showTourStep(); });
  document.querySelectorAll(".d-tab").forEach((t) => {
    t.addEventListener("click", () => { drawerTab = t.dataset.tab; renderDrawer(); });
  });
  $drawerSearch.addEventListener("input", () => renderDrawer());
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { closeModal(); closeDrawer(); closePractice(); }
  });

  learnLoad();
  initState();

  // deep links: ?board=dialectic · ?card=p3 · ?drawer=axioms
  const params = new URLSearchParams(location.search);
  const pb = params.get("board");
  if (pb && QPT_DATA.boards.some((b) => b.id === pb)) state.boardId = pb;

  renderAll();

  const pc = params.get("card");
  if (pc) openModal(pc);
  const pd = params.get("drawer");
  if (pd) openDrawer(pd);
  if (params.get("practice")) openPractice();
  if (params.get("sim")) {
    startSimulation();
    const sn = parseInt(params.get("step") || "", 10);
    if (sn > 1 && sn <= QPT_DATA.learn.simulation.length) {
      sim.quiet = true;
      while (sim.idx < sn - 1) simStep(1);
      sim.quiet = false;
    }
  }

  // first visit: offer the guided tour (suppress with ?notour=1)
  if (!learn.tourSeen && !params.get("notour") && !pc && !pd && !params.get("practice") && !params.get("sim")) {
    setTimeout(startTour, 600);
  }
})();
