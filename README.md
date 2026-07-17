# qpt_2_workbench

An interactive kanban interface for the **Quaternion Process Theory — Consolidated 2.x
Specification, Edition 2.7** (`../QPT_2x_Consolidated_Specification_r2.7.md`).

The board doesn't just display the specification — it *runs* it. Cards are evaluated by
the Quality Gate (`𝒢 ≡ R ⊓ G`, quantitatively `S = ρ − δ − γ·ln k ≥ θ`), movement is
constrained by the axioms, and every refusal cites the section it enforces.

## Running it

Two ways, depending on whether you want the Kimi agent bridge.

### A. UI only (zero setup)

```bash
open index.html            # macOS
# or: double-click index.html, or serve the folder any way you like
```

Everything works from `file://` — boards, editing, tour, practice, simulation,
reference, themes — **except** the `✦ Agent` panel, which needs the server (below).
No build step, no dependencies.

### B. Full experience (UI + Kimi agent)

Prerequisites:

- **Node.js 18+** (developed on v22; check with `node --version`)
- **Kimi Code CLI** installed and authenticated — check with `kimi --version`;
  if missing or logged out, run `kimi login`

Then:

```bash
cd qpt-ui
npm install              # one time: @moonshot-ai/kimi-agent-sdk + zod
npm start                # → QPT Workbench → http://localhost:8787
```

Open **http://localhost:8787** and click **`✦ Agent`** in the topbar. Try:
*"create a living transformation card on the protocol board"* or *"escalate the
fossil auth middleware to the resolution procedure"*.

Verify the bridge independently:

```bash
curl http://localhost:8787/api/health
# {"ok":true,"mock":false,"backend":"compat","executable":"kimi"}
```

`backend` is `sdk` when the CLI supports `--wire` (wire-protocol sessions with
conversation continuity) or `compat` for current CLIs (stateless stream-json turns) —
see § Agent bridge.

### Offline / CI mode (no model calls)

```bash
QPT_AGENT_MOCK=1 npm start
```

The agent endpoint returns a canned plan (one `create_card` action) — enough to test
the full browser → server → board pipeline without a CLI or quota.

### Options

| Variable | Default | Purpose |
|---|---|---|
| `PORT` | `8787` | server port (`PORT=9000 npm start`) |
| `KIMI_EXECUTABLE` | `kimi` | path/name of the CLI binary |
| `KIMI_MODEL` | CLI default | model alias for agent turns |
| `QPT_AGENT_MOCK` | — | `1` = canned responses, no CLI needed |

### Troubleshooting

- **“Cannot reach the agent server…”** in the panel — the server isn't running, or is
  on a non-default port. From `file://` the panel always targets `http://localhost:8787`;
  served from the server itself it uses the same origin automatically.
- **`503 agent unavailable … unknown option '--work-dir'`** — expected on CLI 0.26.x;
  the server falls back to the `compat` backend automatically. Upgrade the CLI
  (`kimi upgrade`) once a `--wire`-capable release exists to get the `sdk` backend.
- **Agent turns fail with auth errors** — run `kimi login`, then retry.
- **Board looks different between the two run modes** — `file://` and
  `http://localhost:8787` are separate origins, each with its own `localStorage` board.
  **Reset board** (topbar) restores the seeded state on either.

No build step, no framework — vanilla HTML/CSS/JS plus a small Node server for the agent.

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

## Agent bridge (`✦ Agent`)

Natural-language requests for the board, processed by a Kimi agent
([@moonshot-ai/kimi-agent-sdk](https://www.npmjs.com/package/@moonshot-ai/kimi-agent-sdk))
driving the local Kimi Code CLI.

```
browser chat panel ──POST /api/agent {message, state}──▶ server.js
     ◀── {reply, actions} ── validates plan ── Kimi agent (kimi CLI)
actions execute in the browser through the same code paths as manual moves
```

**The agent cannot bypass the spec.** It returns a JSON action plan; the server validates
it (allowlist, argument checks, ≤ 12 actions), and the UI executes each action through the
real `canMove` / gate / promotion code — so an illegal move is refused with the same
A13/A9/§14/A7 toasts a human would get.

Action vocabulary: `set_board`, `create_card`, `move_card`, `edit_card` (domain-clamped
patch), `promote_card`, `evaluate_card`.

Backends (auto-detected at startup, see `GET /api/health`):

- **`sdk`** — the SDK's wire-protocol session, used when the installed CLI supports
  `--wire` / `--work-dir`. Conversation continuity across requests.
- **`compat`** — fallback for CLIs without `--wire` (e.g. 0.26.x): each request runs as a
  stateless `kimi -p --output-format stream-json` turn with full instructions + state.
  When a `--wire`-capable CLI lands, the SDK path activates automatically.

Environment: `PORT` (default 8787) · `KIMI_EXECUTABLE` (default `kimi`) · `KIMI_MODEL` ·
`QPT_AGENT_MOCK=1` (offline pipeline test with canned responses, no model calls).

The panel also works from `file://` — it targets `http://localhost:8787` by default.

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
app.js          kanban, gate logic, enforcement, editor, promotion, trace, learning, agent panel
server.js       static hosting + /api/agent bridge to the Kimi CLI
package.json    deps: @moonshot-ai/kimi-agent-sdk, zod (peer)
```

Every section reference (§) in the UI points into the specification document.
