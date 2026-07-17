/* QPT Workbench — agent bridge server.
 *
 * Serves the static UI and exposes POST /api/agent, which forwards natural-language
 * requests to a Kimi Code CLI session via @moonshot-ai/kimi-agent-sdk. The agent
 * replies with a strict JSON action plan; the UI executes it through the same
 * spec-enforcing code paths as manual interaction, so the agent cannot bypass
 * the Quality Gate, horizon, diagnosis, or genesis rules.
 *
 *   npm install
 *   npm start                      # http://localhost:8787
 *   QPT_AGENT_MOCK=1 npm start     # offline pipeline test (no CLI/model calls)
 *
 * Env: PORT (default 8787), KIMI_EXECUTABLE (default "kimi"), KIMI_MODEL (optional),
 *      QPT_AGENT_MOCK=1 (canned responses).
 */

import http from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, ".."); // project root (the spec lives here)
const PORT = parseInt(process.env.PORT || "8787", 10);
const MOCK = process.env.QPT_AGENT_MOCK === "1";
const TURN_TIMEOUT_MS = 150000;

const BOARDS = ["protocol", "dialectic", "resolution"];

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".md": "text/markdown; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ico": "image/x-icon",
};

/* ------------------------------------------------------- agent protocol */

const INSTRUCTIONS = `You are the operator agent for QPT Workbench, an interactive kanban that RUNS the Quaternion Process Theory 2.7 specification. The user sends you natural-language requests plus the current board state as JSON. You decide which board actions to take.

BOARDS AND COLUMNS
- protocol: initiation → encounter → gate → articulation → closure (the Generative Protocol, §14)
- dialectic: explore → integrate → consolidate → synthesize (the Scheduled Dialectic, §19)
- resolution: phase0 → phase1 → phase2 → phase3 → phase4 → phase5 (the Resolution Procedure, §29)

THE SEMANTICS THE UI WILL ENFORCE (your actions are refused if they violate these)
- A13 horizon = 1: cards move forward ONE column at a time. Backward jumps are free.
- A9 dual gate: a protocol card may advance gate → articulation only when its verdict is Living ((○)-recruited source, [□]-grounded target, and S = rho − delta − gamma·ln(k) ≥ theta). Otherwise edit the card's metrics/trajectory first.
- §14: a card failing the gate must have its pathology set (structural|attentional|content|scalar|temporal) before it may leave the gate column backward.
- A7: entering synthesize requires card field genesis (an emergent property present in neither parent).
- Promotion paths: protocol closure → dialectic explore; dialectic synthesize → protocol initiation; a protocol card with a death mode (fossil|residue|imposition) → resolution phase0; resolution phase5 → protocol initiation.

ACTION VOCABULARY (reply with ONLY a JSON object, no prose outside it)
{"reply": "short explanation of what you did and why, with spec refs",
 "actions": [
  {"action": "set_board", "board": "protocol|dialectic|resolution"},
  {"action": "create_card", "board": "...", "title": "...", "source": "recruited|initiated", "target": "grounded|terminated", "kind": "position|note", "reliability": 0.7, "tags": ["..."], "note": "..."},
  {"action": "move_card", "id": "...", "column": "..."},
  {"action": "edit_card", "id": "...", "patch": {"title"|"note"|"rho"|"delta"|"gamma"|"k"|"theta"|"source"|"target"|"scale"|"pathology"|"death"|"reliability"|"genesis"|"tags": "..."}},
  {"action": "promote_card", "id": "...", "board": "dialectic|protocol|resolution"},
  {"action": "evaluate_card", "id": "..."}
 ]}
Use only the fields each action needs. create_card: source/target only matter on protocol; kind/reliability only on dialectic; tags only on resolution. Card ids in the state are current — use them verbatim. At most 12 actions; order them so each is legal when it runs (remember horizon = 1: multi-column journeys need one move per column).

You may read files in your working directory for exact semantics: QPT_2x_Consolidated_Specification_r2.7.md (the full spec) and qpt-ui/qpt-data.js (seeded content).`;

/* --------------------------------------------------------- SDK session */

const EXE = process.env.KIMI_EXECUTABLE || "kimi";
let sessionPromise = null;

// The SDK's wire transport needs a CLI with --wire/--work-dir. Older CLIs (e.g.
// 0.26.x) are driven through `kimi -p --output-format stream-json` instead.
// Probed once at startup.
let backendPromise = null;
function detectBackend() {
  if (!backendPromise) {
    backendPromise = (async () => {
      try {
        const { execFile } = await import("node:child_process");
        const help = await new Promise((res, rej) => {
          execFile(EXE, ["--help"], { timeout: 8000 }, (e, stdout) => (e ? rej(e) : res(stdout)));
        });
        return help.includes("--wire") ? "sdk" : "compat";
      } catch {
        return "compat";
      }
    })();
  }
  return backendPromise;
}

async function getSession() {
  if (!sessionPromise) {
    sessionPromise = (async () => {
      const { createSession } = await import("@moonshot-ai/kimi-agent-sdk");
      const opts = {
        workDir: ROOT,
        executable: EXE,
        yoloMode: true, // never stall the HTTP request on a tool approval
        thinking: false,
      };
      if (process.env.KIMI_MODEL) opts.model = process.env.KIMI_MODEL;
      const session = createSession(opts);
      return { session, greeted: false };
    })();
    sessionPromise.catch(() => { sessionPromise = null; });
  }
  return sessionPromise;
}

function buildPrompt(message, state, includeInstructions) {
  return (
    (includeInstructions ? INSTRUCTIONS + "\n\n" : "") +
    "CURRENT STATE (JSON):\n" + JSON.stringify(state) +
    "\n\nUSER REQUEST:\n" + message +
    "\n\nRespond with ONLY the JSON object described."
  );
}

async function runAgentSdk(message, state) {
  const ctx = await getSession();
  const turn = ctx.session.prompt(buildPrompt(message, state, ctx.greeted));
  ctx.greeted = true;
  let text = "";
  const timeout = new Promise((_, rej) => setTimeout(() => rej(new Error("agent turn timed out")), TURN_TIMEOUT_MS));
  const collect = (async () => {
    for await (const ev of turn) {
      if (ev.type === "ContentPart" && ev.payload && ev.payload.type === "text") {
        text += ev.payload.text;
      }
    }
    return turn.result;
  })();
  await Promise.race([collect, timeout]);
  return text;
}

// compat: one stateless prompt per request via the installed CLI's stream-json mode
async function runAgentCompat(message, state) {
  const { execFile } = await import("node:child_process");
  const args = ["-p", buildPrompt(message, state, true), "--output-format", "stream-json"];
  if (process.env.KIMI_MODEL) args.push("--model", process.env.KIMI_MODEL);
  const stdout = await new Promise((resolve, reject) => {
    execFile(EXE, args, { cwd: ROOT, timeout: TURN_TIMEOUT_MS, maxBuffer: 16 * 1024 * 1024 },
      (err, out, stderr) => {
        if (err) return reject(new Error("CLI exited with code " + err.code + ": " + String(stderr || err.message).slice(0, 300)));
        resolve(out);
      });
  });
  const parts = [];
  for (const line of stdout.split("\n")) {
    if (!line.trim()) continue;
    try {
      const ev = JSON.parse(line);
      if (ev.role === "assistant") {
        if (typeof ev.content === "string") parts.push(ev.content);
        else if (Array.isArray(ev.content)) {
          parts.push(ev.content.filter((p) => p && p.type === "text").map((p) => p.text).join(""));
        }
      }
    } catch { /* non-JSON line */ }
  }
  return parts.join("\n") || stdout.slice(-2000);
}

async function runAgent(message, state) {
  const backend = await detectBackend();
  return backend === "sdk" ? runAgentSdk(message, state) : runAgentCompat(message, state);
}

/* ------------------------------------------------------- plan handling */

function extractJson(text) {
  const fence = /```(?:json)?\s*([\s\S]*?)```/.exec(text);
  const candidate = fence ? fence[1] : text;
  const start = candidate.indexOf("{");
  const end = candidate.lastIndexOf("}");
  if (start < 0 || end <= start) return null;
  try { return JSON.parse(candidate.slice(start, end + 1)); } catch { return null; }
}

function validatePlan(plan) {
  const warnings = [];
  const actions = [];
  const list = Array.isArray(plan && plan.actions) ? plan.actions.slice(0, 12) : [];
  for (const a of list) {
    if (!a || typeof a.action !== "string") { warnings.push("dropped malformed action"); continue; }
    switch (a.action) {
      case "set_board":
        if (BOARDS.includes(a.board)) actions.push({ action: "set_board", board: a.board });
        else warnings.push("set_board: unknown board " + JSON.stringify(a.board));
        break;
      case "create_card": {
        if (!BOARDS.includes(a.board) || typeof a.title !== "string" || !a.title.trim()) {
          warnings.push("create_card: needs board + title"); break;
        }
        const c = { action: "create_card", board: a.board, title: a.title.slice(0, 200) };
        if (["recruited", "initiated"].includes(a.source)) c.source = a.source;
        if (["grounded", "terminated"].includes(a.target)) c.target = a.target;
        if (["position", "note"].includes(a.kind)) c.kind = a.kind;
        if (typeof a.reliability === "number") c.reliability = Math.max(0, Math.min(1, a.reliability));
        if (Array.isArray(a.tags)) c.tags = a.tags.filter((t) => typeof t === "string").slice(0, 8);
        if (typeof a.note === "string") c.note = a.note.slice(0, 2000);
        actions.push(c);
        break;
      }
      case "move_card":
        if (typeof a.id === "string" && typeof a.column === "string") {
          actions.push({ action: "move_card", id: a.id, column: a.column });
        } else warnings.push("move_card: needs id + column");
        break;
      case "edit_card":
        if (typeof a.id === "string" && a.patch && typeof a.patch === "object") {
          actions.push({ action: "edit_card", id: a.id, patch: a.patch });
        } else warnings.push("edit_card: needs id + patch");
        break;
      case "promote_card":
        if (typeof a.id === "string" && BOARDS.includes(a.board)) {
          actions.push({ action: "promote_card", id: a.id, board: a.board });
        } else warnings.push("promote_card: needs id + valid board");
        break;
      case "evaluate_card":
        if (typeof a.id === "string") actions.push({ action: "evaluate_card", id: a.id });
        else warnings.push("evaluate_card: needs id");
        break;
      default:
        warnings.push("unknown action " + JSON.stringify(a.action));
    }
  }
  const reply = plan && typeof plan.reply === "string" ? plan.reply : "Done.";
  return { reply, actions, warnings };
}

/* ------------------------------------------------------------- server */

function sendJson(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, {
    "content-type": "application/json; charset=utf-8",
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "GET,POST,OPTIONS",
    "access-control-allow-headers": "content-type",
  });
  res.end(body);
}

async function handleAgent(req, res) {
  let body = "";
  for await (const chunk of req) body += chunk;
  let parsed;
  try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
  const message = typeof parsed.message === "string" ? parsed.message.trim() : "";
  if (!message) return sendJson(res, 400, { error: "message required" });
  const state = parsed.state && typeof parsed.state === "object" ? parsed.state : {};

  if (MOCK) {
    return sendJson(res, 200, {
      reply: "[mock] Pipeline check: I created a living transformation card on the protocol board. Run without QPT_AGENT_MOCK for real reasoning.",
      actions: [{
        action: "create_card", board: "protocol",
        title: "Mock agent card — the bridge works",
        source: "recruited", target: "grounded",
        note: "Created by the mock agent path (QPT_AGENT_MOCK=1).",
      }],
      warnings: [],
    });
  }

  try {
    const text = await runAgent(message, state);
    const plan = extractJson(text);
    if (!plan) {
      return sendJson(res, 200, {
        reply: "The agent did not return a parseable plan. Raw reply: " + text.slice(0, 1500),
        actions: [],
        warnings: ["unparseable plan"],
      });
    }
    return sendJson(res, 200, validatePlan(plan));
  } catch (err) {
    sessionPromise = null; // next request rebuilds the session
    return sendJson(res, 503, {
      error: "agent unavailable: " + String((err && err.message) || err),
      hint: "Check that the kimi CLI is installed and logged in (kimi --version), then retry.",
    });
  }
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  if (req.method === "OPTIONS") return sendJson(res, 204, {});
  if (url.pathname === "/api/health") {
    return sendJson(res, 200, { ok: true, mock: MOCK, backend: MOCK ? "mock" : await detectBackend(), executable: EXE });
  }
  if (url.pathname === "/api/agent" && req.method === "POST") return handleAgent(req, res);

  // static files
  let p = decodeURIComponent(url.pathname);
  if (p === "/") p = "/index.html";
  const file = path.normalize(path.join(__dirname, p));
  if (!file.startsWith(__dirname)) { res.writeHead(403); return res.end("forbidden"); }
  try {
    const data = await readFile(file);
    res.writeHead(200, { "content-type": MIME[path.extname(file)] || "application/octet-stream" });
    res.end(data);
  } catch {
    res.writeHead(404); res.end("not found");
  }
});

server.listen(PORT, () => {
  console.log(`QPT Workbench → http://localhost:${PORT}  (agent bridge ${MOCK ? "MOCK" : "kimi CLI"})`);
});
