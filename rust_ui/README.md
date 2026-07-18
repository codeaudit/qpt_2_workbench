# qpt_rust_ui

The **QPT Workbench** with its backend re-implemented in **Rust** — a port of
[`../qpt-ui`](../qpt-ui) (Node.js). The browser frontend is unchanged: the same
`index.html`, `app.js`, `styles.css`, `qpt-data.js` and `qpt-core.js` are served
verbatim from `public/`, so the app looks and behaves identically. Everything
outside the browser — the domain core, HTTP API, store, jobs queue, skills,
settings/MCP, functions engine, CLI, and the Kimi agent bridge — is Rust.

An interactive kanban interface for the **Quaternion Process Theory —
Consolidated 2.x Specification, Edition 2.7**
(`../QPT_2x_Consolidated_Specification_r2.7.md`). The board doesn't just display
the specification — it *runs* it: cards are evaluated by the Quality Gate
(`𝒢 ≡ R ⊓ G`, quantitatively `S = ρ − δ − γ·ln k ≥ θ`), movement is constrained
by the axioms, and every refusal cites the section it enforces. See
`../qpt-ui/README.md` for the full product documentation (boards, rules of
play, learning features) — all of it applies here unchanged.

## Running it

Prerequisites:

- **Rust** (developed on 1.86; `cargo --version`)
- **Node.js 18+** — only if you use *Functions* (`data/functions/*.js` are user
  JS executed by `node`, same contract as the Node backend)
- **Kimi Code CLI** installed and authenticated — only for the `✦ Agent` panel
  (`kimi --version`; `kimi login` if missing)

```bash
cd qpt_rust_ui
cargo run --release          # → QPT Workbench → http://localhost:8787
```

Open **http://localhost:8787**. Everything works — boards, tour, practice,
simulation, reference, themes, CLI console, jobs panel, settings — including
the `✦ Agent` panel, driven by the local `kimi` CLI.

The UI also works standalone from `public/index.html` (`file://`) exactly as
before, minus the agent panel.

### CLI

Same command language as the Node version, as a standalone binary:

```bash
cargo run --bin qpt-cli -- "cards --board protocol"
cargo run --bin qpt-cli -- "create --board protocol --title 'Ground the signup flow' --source recruited --target grounded"
cargo run --bin qpt-cli -- "move p1 gate"        # A13 refuses forward skips
cargo run --bin qpt-cli -- "edit p2 pathology=content theta=0.35"
cargo run --bin qpt-cli -- "evaluate p1"         # → Living (S 0.660)
cargo run --bin qpt-cli -- "promote p9 --to dialectic"
cargo run --bin qpt-cli -- "help"
```

(The binary must run from the crate root so it finds `data/store.json` —
or copy `target/release/qpt-cli` and run it from any QPT data dir.)

### Offline / CI mode (no model calls)

```bash
QPT_AGENT_MOCK=1 cargo run
```

The agent endpoint returns a canned plan (one `create_card` action) — enough to
test the full browser → server → board pipeline without a CLI or quota.

### Options

| Variable | Default | Purpose |
|---|---|---|
| `PORT` | `8787` | server port |
| `KIMI_EXECUTABLE` | `kimi` | path/name of the CLI binary |
| `KIMI_MODEL` | CLI default | model alias for agent turns |
| `QPT_AGENT_MOCK` | — | `1` = canned responses, no CLI needed |
| `QPT_MOCK_DELAY_MS` | `500` | mock latency (`0` for fast headless tests) |
| `LOG_LEVEL` | `info` | `debug` · `info` · `warn` · `error` |

Logging matches the Node version: console + `data/server.log` (rotated to `.1`
at ~1 MB), with `server.start`, `http` access lines, `job.*`, `agent.*`,
`action.refused`, `cli`, `store.save`, and all settings/skill/model/MCP
mutations. No prompt bodies or secrets — sizes and ids only.

## Tests and parity

```bash
cargo test                   # 20 core semantics tests + full HTTP API suite
node tests/parity.mjs        # differential test vs the Node backend
```

`tests/parity.mjs` boots the original Node server (`../qpt-ui`) and this Rust
server against identical scratch data dirs and deep-compares ~75 responses:
every `/api` route, the full CLI grammar (over HTTP and as standalone bins),
mock agent turns (one-shot and NDJSON stream), jobs, skills across all scopes,
settings/keys/MCP/functions, and static bytes.

## Layout

```
Cargo.toml        one crate "qpt": lib + bins qpt-server (default) · qpt-cli
src/
  core.rs         ← qpt-core.js: gate math, canMove, trace, Γ EMA, promotion, actions
  data.rs         ← qpt-data.js (boards + seed cards, embedded from assets/boards.json)
  store.rs        ← server-store.js (data/store.json, atomic writes, versioned)
  cli_exec.rs     ← cli-exec.js (shared by HTTP and the qpt-cli bin)
  jobs.rs         ← server-jobs.js (FIFO queue, progress, cancel, history)
  skills.rs       ← server-skills.js (Agent Skills format + scope browsing)
  settings.rs     ← server-settings.js (masked keys, prompt override, MCP sync)
  functions.rs    ← server-functions.js (user JS via node, 20 s timeout)
  agent.rs        ← the /api/agent section of server.js (kimi bridge, plan validation)
  log.rs          ← server-log.js
  http.rs         ← the routing/static section of server.js (axum)
  main.rs         qpt-server startup
  bin/qpt-cli.rs  ← cli.js
assets/boards.json  QPT_DATA.boards, generated from qpt-data.js (see below)
public/           the frontend, copied verbatim from qpt-ui
data/             store.json, skills/, functions/, config.json (starting state,
                  copied from qpt-ui; server.log and config.json are gitignored)
tests/            core_tests.rs · api_tests.rs · parity.mjs
```

## Differences from the Node backend (intentional)

- **No `sdk` wire backend.** The npm SDK can't be used from Rust, and current
  CLIs don't ship `--wire` anyway — the bridge always uses the `compat`
  backend (`kimi -p --output-format stream-json`), which is the path current
  CLIs actually take in the Node version too. `/api/health` reports
  `backend: "compat"` (or `"mock"`). The startup probe still runs for logging.
- **Static serving is confined to `public/`.** The Node server exposed its
  whole app dir (including `data/store.json`); this one serves only the
  frontend files. The API surface is unchanged.
- **`GET /api/models` has no SDK fallback.** Models are parsed from the CLI's
  `config.toml` directly (the preferred source in the Node version too; the
  SDK fallback only existed for malformed configs).
- User **functions still require Node** at runtime (`data/functions/*.js`),
  unchanged by design — the contract (`{args, state}` on stdin, one JSON on
  stdout, 20 s timeout) is identical.
- Method mismatches on API routes return 405 (axum) instead of falling
  through to the static 404; the UI never relies on that.

If the frontend in `../qpt-ui` changes, re-copy it:
`cp ../qpt-ui/{index.html,app.js,styles.css,qpt-data.js,qpt-core.js} public/`.
If the seed boards in `qpt-data.js` change, regenerate `assets/boards.json`:

```bash
cd ../qpt-ui && node -e "const fs=require('fs');const QPT_DATA=new Function(fs.readFileSync('qpt-data.js','utf8')+';return QPT_DATA;')();fs.writeFileSync('../qpt_rust_ui/assets/boards.json',JSON.stringify(QPT_DATA.boards));"
```
