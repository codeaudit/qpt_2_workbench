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
browser chat panel ──POST /api/agent {message}──▶ server.js
     ◀── {reply, results} ── executes plan on the store ── Kimi agent (kimi CLI)
                    ▲                    │
        canonical store (data/store.json) ◀──┘   versioned, synced to the browser
```

**The agent cannot bypass the spec.** The agent returns a JSON action plan; the server
validates it (allowlist, argument checks, ≤ 12 actions) and executes each action on the
canonical store through `qpt-core.js` — the same domain code the browser runs — so an
illegal move comes back refused with the same A13/A9/§14/A7 semantics a human gets.
The browser adopts the store state after each turn.

**Live progress.** The bridge streams NDJSON (`POST /api/agent?stream=1`): progress lines
(`reasoning… Ns` heartbeat, `drafting the plan…`, `calling tool: X` when available) followed
by one `result` line. The chat panel renders them as a live `⏳ Ns · activity` bubble, and
Send is disabled while a turn is in flight. Plain `POST /api/agent` (no `stream`) still
returns one-shot JSON.

Action vocabulary: `set_board`, `create_card`, `move_card`, `edit_card` (domain-clamped
patch), `promote_card`, `evaluate_card`.

Backends (auto-detected at startup, see `GET /api/health`):

- **`sdk`** — the SDK's wire-protocol session, used when the installed CLI supports
  `--wire` / `--work-dir`. Conversation continuity across requests.
- **`compat`** — fallback for CLIs without `--wire` (e.g. 0.26.x): each request runs as a
  stateless `kimi -p --output-format stream-json` turn with full instructions + state.
  When a `--wire`-capable CLI lands, the SDK path activates automatically.

Environment: `PORT` (default 8787) · `KIMI_EXECUTABLE` (default `kimi`) · `KIMI_MODEL` ·
`QPT_AGENT_MOCK=1` (offline pipeline test with canned responses, no model calls) ·
`QPT_MOCK_DELAY_MS` (default 500 — mock latency; `0` for fast headless tests).

**Model selection.** The panel header has a model dropdown, populated from the CLI
configuration (`~/.kimi-code/config.toml`, or `$KIMI_CODE_HOME/config.toml`), read
directly — the SDK's `parseConfig()` is kept only as a fallback because it drops
models from real-world configs. `GET /api/models` returns the list with the CLI's
default marked. Switching applies to every subsequent agent turn (`POST /api/model` —
runtime only; it rebuilds the SDK session and never touches your CLI config).
`KIMI_MODEL` pins the startup value; "CLI default" unsets it. Models you add to the
CLI (`kimi provider`) appear automatically.

**Agentic jobs — the common way to invoke and monitor.** Every long-running call
(chat turns, skill generations, anything future) runs as a **job** on a single FIFO
worker (`server-jobs.js`). Submit via `POST /api/jobs {kind: "agent"|"generate", …}`
and get a `jobId` immediately; watch with `GET /api/jobs` / `GET /api/jobs/<id>`
(status, live progress lines, results, timings) and stop with
`POST /api/jobs/<id>/cancel` (kills the CLI child process / interrupts the SDK turn;
queued jobs never start). In the UI, the **◷ Jobs** topbar button carries a live
badge of active jobs and opens the jobs panel: per-job status
(queued/running/done/error/cancelled), latest progress, durations, and cancel buttons —
it polls even while closed, so the badge stays honest. The chat stream and the skill
`generate` button both flow through the same queue, so everything you start is
visible in one place.

The panel also works from `file://` — it targets `http://localhost:8787` by default.

## Store, CLI, and skills

**Canonical store.** With the server running, state lives in `data/store.json`
(versioned, atomic writes, seeded from the spec on first run). The browser hydrates
from it, pushes its own edits (debounced), and polls for external changes every 3 s —
so UI, CLI, and agent always share one board. From `file://` or offline, the browser
falls back to `localStorage` as before.

**One domain core.** `qpt-core.js` holds all mechanics — gate math, `canMove`, traces,
Γ EMA, promotion, card shapes, patch clamping, action execution — shared by the
browser, the server store, the CLI, and the agent. A rule changes in exactly one place.

**CLI** — the same command language in three places: the `›` console in the agent panel,
`POST /api/cli`, and a standalone bin that works without the server:

```bash
node cli.js "cards --board protocol"
node cli.js "create --board protocol --title 'Ground the signup flow' --source recruited --target grounded"
node cli.js "move p1 gate"          # A13 refuses forward skips, §14 refuses undiagnosed returns
node cli.js "edit p2 pathology=content theta=0.35"
node cli.js "evaluate p1"           # → Living (S 0.660)
node cli.js "promote p9 --to dialectic"
node cli.js "help"                  # full grammar
```

**Skills** — user-authored instruction sets the agent reads on every request, stored on
disk in the [Agent Skills open format](https://agentskills.io/specification):
`data/skills/<name>/SKILL.md` (YAML frontmatter with `name` + `description`, Markdown
body; optional `license` / `compatibility` / `metadata` / `allowed-tools` preserved).
Names follow the spec — 1–64 lowercase alphanumerics/hyphens, no leading/trailing or
consecutive hyphens, and the `name` field always matches the directory. Create and edit
them in the Reference drawer → **Skills** tab (the `＋ New skill` button), or via CLI / REST:

```bash
node cli.js "skill create grounding-protocol \
  --description 'House rules for gate edits — when to touch rho vs theta.' \
  --content 'Prefer raising rho over lowering theta.'"
node cli.js "skills"                # list · skill show/edit/delete <id>
```

When a request matches a skill's description, the agent follows its content — skills
are how you teach the agent your house rules. Since they're plain folders, they're
version-controllable and loadable by any skills-compatible tool (the layout matches
`kimi --skills-dir` conventions too). The JSON store holds no skills — only cards.

**Browsing all skills on the system.** The Skills tab also aggregates every scope the
CLI discovers (`GET /api/skills/all`): **workbench** (`data/skills/`, editable), **user**
(`$KIMI_CODE_HOME/skills` + `~/.agents/skills`), **project** (`<nearest .git ancestor>/.kimi-code/skills`
+ `.agents/skills`), and **extra** (`extra_skill_dirs` from the CLI config). External
skills render read-only with an **import** button (`POST /api/skills/import`) that copies
one into the workbench store so the agent starts using it; duplicates shadowed by a
higher-priority scope (Project > User > Extra) are marked. Built-in CLI skills ship
inside the CLI itself — no files to browse.

**Generating skills.** The New skill form has a `generate` button: type a hint
("audit card titles for jargon") and the **`skill-generator` skill** — itself an
ordinary, editable `data/skills/skill-generator/SKILL.md` — is invoked as the agent's
instructions (`POST /api/skills/generate`). It returns a spec-valid draft
`{name, description, content}` with collision checks against every known skill name;
the draft lands in the form for your review, and **create** saves it. Nothing is
auto-saved. Edit `skill-generator` like any other skill to change how drafts are written.

**Installed catalog** (all editable, all matched by description):

- *Board operations* — `grounding-protocol` (fix a failing card, lever order, never fake G) ·
  `gate-triage` (route a whole board by verdict × zone) · `living-naming` (§12 title audits) ·
  `promotion-paths` (the four cross-board paths and their transforms) ·
  `dialectic-moderator` (run board 2: χ, Γ EMA, genesis, HALT)
- *Process* — `board-health-report` (fixed-format periodic audit) ·
  `minimum-dissolution` (§11 dead structure, living side first) ·
  `qpt-spec-lookup` (answers from the r2.7 file, §-cited, canonical numbers)
- *Coaching & meta* — `socratic-gate` (teach the five layers by questions) ·
  `sim-authoring` (write new tour/sim/exercise content) ·
  `store-backup` (snapshot/restore `store.json` + skills) ·
  `skill-generator` (draft new skills from a hint)

HTTP surface: `GET/POST /api/state` · `POST /api/cli` · `GET/POST/DELETE /api/skills` ·
`GET /api/skills/all` · `POST /api/skills/import` · `POST /api/skills/generate` ·
`GET/POST /api/jobs` · `GET /api/jobs/<id>` · `POST /api/jobs/<id>/cancel` ·
`POST /api/agent(?stream=1)` · `GET /api/health`.

## Settings page (`⚙ Settings`)

A full settings view with six sections:

- **Skills** — read-only browser over every scope (workbench · user · project · extra)
- **Models** — the same model selector as the panel, plus the full registered list
  (capabilities, context size, CLI default)
- **API Keys** — named secrets stored in `data/config.json` (gitignored), injected as
  environment variables into every agent turn. Write-only: the UI shows masks
  (`tvly…7890`), values are never returned in full
- **System Prompt** — override the agent's built-in instructions; applies to every
  subsequent turn, one click back to the default
- **MCP Servers** — manage entries in the [MCP format](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/mcp.html)
  (stdio `command`/`args`, http/sse `url`, `env`, `headers`, `enabled`), synced to the
  project-level `.kimi-code/mcp.json` that agent sessions load (previous file backed
  up as `mcp.json.bak`; entries you didn't create here are preserved). stdio servers
  execute their command when a session starts — only add servers you trust
- **Functions** — user-generated functions in `data/functions/<name>.js`. Contract:
  read `{args, state}` as JSON on stdin, print one JSON `{message, actions?}` on
  stdout (20 s timeout). Returned actions execute through the shared core — functions
  compute, the spec enforces. The agent calls them via the `run_function` action;
  `// @description` on line one is what it matches against

Settings routes: `GET/PUT /api/settings` · `GET/POST/DELETE /api/keys` ·
`GET/POST/DELETE /api/mcp` · `GET/POST/DELETE /api/functions` ·
`POST /api/functions/<name>/run`.

## Deep links

```
?board=protocol|dialectic|resolution
?card=<id>            open a card's detail view
?drawer=semantics|skills|gate|axioms|signs|agency|failure|drill
?practice=1           open the practice panel
?sim=1&step=N         start / jump into the simulation
?theme=light|dark     force a theme
?notour=1             suppress the first-visit tour
```

## Persistence

With the server running, the canonical state is `data/store.json` (cards, active board,
skills) and the browser mirrors it. Standalone (`file://`), state lives in
`localStorage` (`qpt-workbench-v2`, `qpt-learn-v1`, `qpt-theme`). **Reset board**
restores the seeded specification state on either.

## Files

```
index.html      shell
styles.css      theme (dark + light, CSS variables)
qpt-data.js     all spec content: boards, cards, registry, tour, exercises, simulation
qpt-core.js     shared domain core (browser + Node): gate, rules, promotion, actions
app.js          kanban UI: rendering, editor, tour, practice, sim, sync, agent panel
server.js       static hosting + /api/agent bridge + state/cli/skills routes
server-jobs.js  FIFO agentic job queue: submit, progress, cancel, history
server-settings.js  workbench config: masked keys, prompt override, MCP sync
server-functions.js user-function engine (data/functions/*.js)
server-store.js canonical card store (data/store.json), seeded from the spec
server-skills.js skills on disk — Agent Skills format (data/skills/*/SKILL.md)
cli-exec.js     command language executor (shared by HTTP and the bin)
cli.js          standalone CLI: node cli.js "<command>"
package.json    deps: @moonshot-ai/kimi-agent-sdk, zod (peer)
```

Every section reference (§) in the UI points into the specification document.
