# qpt_2_workbench

An interactive kanban interface for the **Quaternion Process Theory — Consolidated 2.x
Specification, Edition 2.7** (`../QPT_2x_Consolidated_Specification_r2.7.md`).

The board doesn't just display the specification — it *runs* it. Cards are evaluated by
the Quality Gate (`𝒢 ≡ R ⊓ G`, quantitatively `S = ρ − δ − γ·ln k ≥ θ`), movement is
constrained by the axioms, and every refusal cites the section it enforces.

## Quick start

```bash
open index.html
```

No build step, no dependencies, works from `file://` — vanilla HTML/CSS/JS.

## The boards — three workflows from the spec

| Board | Source | Workflow |
|---|---|---|
| **The Generative Protocol** | §14 · Part IV | `(○)` Initiation → `[□]` Encounter → `𝒢` Gate Evaluation → `{△}` Articulation → `↻` Recursive Closure |
| **The Scheduled Dialectic** | §19 · Part V | Explore → Integrate → Consolidate → Synthesize, under the χ schedule `0.9 →\|π 0.7 →\|π 0.5 →\|π 0.3` |
| **The Resolution Procedure** | §29 · Part VII | Phase 0 Pre-diagnostic → … → Phase 5 Evolutionary iteration |

Cards carry sign classes, trajectories (`source → target`), anchoring metrics
(`ρ δ γ k θ`), scale, pathology layers, death modes, reliability `Γ` — everything the
gate and the dialectic need. Verdicts (Living / Beautiful delusion / Competent-dead /
Fully dead), zones (Z1–Z3), and weights `w` recompute live.

## Rules of play (enforced semantics)

Constraints — checked before every move, with axiom-citing toasts that link into the registry:

- **A13 · horizon = 1** — forward moves are limited to the adjacent column
- **A9 · dual gate** — only living trajectories advance Gate → Articulation
- **§14 · diagnose first** — a failed card must be diagnosed (five-layer model) before returning
- **A7 · genesis** — entering Synthesize requires a declared emergent property

Actions — fired by successful moves:

- **Trace (§9.2)** — every transition is logged on the card ("a trace, not a plan")
- **Gate recording (§8)** — entering the gate logs the verdict with S vs θ
- **Γ update (§19)** — surviving a dialectic phase raises reliability by EMA (`Γ ← 0.7Γ + 0.3`), recomputing all board weights
- **↻ cycle (A12)** — Closure → Initiation increments the cycle counter

Promotion — the meta-workflow between boards, from a card's detail view:

- Closure → Explore (as a dialectic position, §15)
- Synthesize → Initiation (grounding the constructor, §7.4)
- death mode → Phase 0 (escalation to the failure field, §11/§29)
- Phase 5 → Initiation (return to living process, §29)

## Interacting

- **Drag & drop** between columns (or the `‹ ›` buttons); refusals explain themselves
- **Click a card** for the full view: gate evaluation, editable trajectory matrix,
  domain-clamped metric sliders, pathology/death pickers, naming classification (§12),
  the trace, registry links, promotion actions
- **Create cards** at each board's entry column (transformations, positions, moderator
  notes, interventions — each with the right shape)
- **Theme toggle** (dark/light), persisted

## Learning features

- **Guided tour** — 10 steps, auto-plays on first visit (`? Tour` to replay)
- **Practice** (`▶ Practice`) — 5 exercises graded by the spec's own constraints
- **Guided simulation** (`◉ Sim`) — a 16-step narrated scenario that drives the real
  board through every stage: a refused gate, diagnosis, passage, naming, closure,
  promotion, the dialectic, genesis, and home. Non-destructive; restores your board
- **Reference drawer** (`⌘ Reference`) — Semantics (the rules of play), formulas,
  the full axiom/law registry (A1–A24, L1–L7, ML), signs & categories, agency,
  failure & diagnostics — all searchable — plus a 102-card flashcard **Drill**

## Deep links

```
?board=protocol|dialectic|resolution
?card=<id>            open a card's detail view
?drawer=semantics|gate|axioms|signs|agency|failure|drill
?practice=1           open the practice panel
?sim=1&step=N         start / jump into the simulation
?theme=light|dark     force a theme
?notour=1             suppress the first-visit tour
```

## Persistence

Board state, learning progress, and theme are stored in `localStorage`
(`qpt-workbench-v2`, `qpt-learn-v1`, `qpt-theme`). **Reset board** restores the
seeded specification state.

## Files

```
index.html      shell
styles.css      theme (dark + light, CSS variables)
qpt-data.js     all spec content: boards, cards, registry, tour, exercises, simulation
app.js          kanban, gate logic, enforcement, editor, promotion, trace, learning
```

Every section reference (§) in the UI points into the specification document.
