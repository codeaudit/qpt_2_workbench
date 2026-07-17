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
import { openStore } from "./server-store.js";
import { execCommand } from "./cli-exec.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, ".."); // project root (the spec lives here)
const PORT = parseInt(process.env.PORT || "8787", 10);
const MOCK = process.env.QPT_AGENT_MOCK === "1";
const TURN_TIMEOUT_MS = 150000;

const store = await openStore(); // canonical state: cards, board, skills (data/store.json)

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

EXECUTION: your actions run on the canonical server-side store via the shared domain core — the same code the UI uses. Anything illegal comes back refused in the results.

SKILLS: state.skills lists user-authored instruction sets. When a skill's description matches the request, follow its content — skills are the user's way of teaching you their house rules. Users can also drive the store directly through a text CLI (help · state · cards · create · move · edit · evaluate · promote · skills …) exposed at POST /api/cli; you do not need it — your channel is the action vocabulary above.

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

async function runAgentSdk(message, state, onProgress) {
  const ctx = await getSession();
  const turn = ctx.session.prompt(buildPrompt(message, state, ctx.greeted));
  ctx.greeted = true;
  let text = "";
  const t0 = Date.now();
  const hb = onProgress && setInterval(() => onProgress("reasoning… " + Math.round((Date.now() - t0) / 1000) + "s"), 2500);
  const timeout = new Promise((_, rej) => setTimeout(() => rej(new Error("agent turn timed out")), TURN_TIMEOUT_MS));
  const collect = (async () => {
    for await (const ev of turn) {
      if (ev.type === "ContentPart" && ev.payload && ev.payload.type === "text") {
        if (!text && onProgress) onProgress("drafting the plan…");
        text += ev.payload.text;
      } else if (ev.type === "ToolCall" && onProgress) {
        const fn = ev.payload && ev.payload.function;
        onProgress("calling tool: " + ((fn && fn.name) || "unknown"));
      } else if (ev.type === "ApprovalRequest" && onProgress) {
        onProgress("waiting on a tool approval…");
      }
    }
    return turn.result;
  })();
  try {
    await Promise.race([collect, timeout]);
  } finally {
    if (hb) clearInterval(hb);
  }
  return text;
}

// compat: one stateless prompt per request via the installed CLI's stream-json mode
async function runAgentCompat(message, state, onProgress) {
  const { spawn } = await import("node:child_process");
  const args = ["-p", buildPrompt(message, state, true), "--output-format", "stream-json"];
  if (process.env.KIMI_MODEL) args.push("--model", process.env.KIMI_MODEL);
  return new Promise((resolve, reject) => {
    const child = spawn(EXE, args, { cwd: ROOT });
    const t0 = Date.now();
    let buf = "", errBuf = "";
    const parts = [];
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("agent turn timed out"));
    }, TURN_TIMEOUT_MS);
    const hb = onProgress && setInterval(() => onProgress("reasoning… " + Math.round((Date.now() - t0) / 1000) + "s"), 2500);

    function handleLine(line) {
      if (!line.trim()) return;
      let ev;
      try { ev = JSON.parse(line); } catch { return; }
      if (ev.role === "assistant") {
        const texts = typeof ev.content === "string"
          ? [ev.content]
          : (Array.isArray(ev.content) ? ev.content.filter((p) => p && p.type === "text").map((p) => p.text) : []);
        if (texts.length) {
          if (!parts.length && onProgress) onProgress("drafting the plan…");
          parts.push(texts.join(""));
        }
        if (Array.isArray(ev.content) && onProgress) {
          ev.content.filter((p) => p && p.type === "tool_use")
            .forEach((p) => onProgress("calling tool: " + (p.name || "unknown")));
        }
      } else if (ev.role !== "meta" && ev.type && onProgress) {
        onProgress("event: " + ev.type);
      }
    }

    child.stdout.on("data", (chunk) => {
      buf += chunk.toString();
      let idx;
      while ((idx = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, idx);
        buf = buf.slice(idx + 1);
        handleLine(line);
      }
    });
    child.stderr.on("data", (chunk) => { errBuf += chunk.toString(); });
    child.on("error", (e) => {
      clearTimeout(timer);
      if (hb) clearInterval(hb);
      reject(e);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (hb) clearInterval(hb);
      if (buf.trim()) handleLine(buf); // unterminated last line
      if (code !== 0) {
        return reject(new Error("CLI exited with code " + code + (errBuf ? ": " + errBuf.slice(0, 300) : "")));
      }
      resolve(parts.join("\n"));
    });
  });
}

async function runAgent(message, state, onProgress) {
  const backend = await detectBackend();
  return backend === "sdk"
    ? runAgentSdk(message, state, onProgress)
    : runAgentCompat(message, state, onProgress);
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

function agentState() {
  const s = store.CORE.compactState(store.data);
  s.skills = store.data.skills.map((sk) => ({ id: sk.id, name: sk.name, description: sk.description, content: sk.content }));
  return s;
}

// execute the validated plan on the canonical store via the shared core
async function executePlan(validated) {
  const results = [];
  let changed = false;
  for (const a of validated.actions) {
    const r = store.CORE.applyAction(store.data, Object.assign({ via: "the Kimi agent" }, a));
    results.push({ action: a.action, ok: r.ok, message: r.message });
    if (r.ok) changed = true;
  }
  if (changed) await store.save();
  return { ...validated, results, version: store.data.version };
}

async function produce(message, _clientState, onProgress) {
  if (MOCK) {
    if (onProgress) onProgress("mock backend: generating canned plan…");
    await new Promise((r) => setTimeout(r, parseInt(process.env.QPT_MOCK_DELAY_MS || "500", 10)));
    return executePlan({
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
  const text = await runAgent(message, agentState(), onProgress);
  const plan = extractJson(text);
  const validated = plan
    ? validatePlan(plan)
    : { reply: "The agent did not return a parseable plan. Raw reply: " + text.slice(0, 1500), actions: [], warnings: ["unparseable plan"] };
  return executePlan(validated);
}

async function handleAgent(req, res, url) {
  let body = "";
  for await (const chunk of req) body += chunk;
  let parsed;
  try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
  const message = typeof parsed.message === "string" ? parsed.message.trim() : "";
  if (!message) return sendJson(res, 400, { error: "message required" });
  const state = parsed.state && typeof parsed.state === "object" ? parsed.state : {};
  const stream = url.searchParams.get("stream") === "1";

  if (stream) {
    res.writeHead(200, {
      "content-type": "application/x-ndjson; charset=utf-8",
      "access-control-allow-origin": "*",
      "cache-control": "no-cache",
    });
    const send = (obj) => res.write(JSON.stringify(obj) + "\n");
    try {
      const plan = await produce(message, state, (detail) => send({ type: "progress", detail }));
      send({ type: "result", ...plan });
    } catch (err) {
      sessionPromise = null;
      send({
        type: "error",
        error: "agent unavailable: " + String((err && err.message) || err),
        hint: "Check that the kimi CLI is installed and logged in (kimi --version), then retry.",
      });
    }
    return res.end();
  }

  try {
    return sendJson(res, 200, await produce(message, state, null));
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
    return sendJson(res, 200, { ok: true, mock: MOCK, backend: MOCK ? "mock" : await detectBackend(), executable: EXE, storeVersion: store.data.version });
  }

  if (url.pathname === "/api/state" && req.method === "GET") {
    return sendJson(res, 200, {
      version: store.data.version,
      boardId: store.data.boardId,
      customSeq: store.data.customSeq,
      cards: store.data.cards,
      skills: store.data.skills,
    });
  }

  if (url.pathname === "/api/state" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    if (typeof parsed.clientVersion === "number" && parsed.clientVersion !== store.data.version) {
      return sendJson(res, 409, {
        error: "version conflict",
        version: store.data.version,
        boardId: store.data.boardId,
        customSeq: store.data.customSeq,
        cards: store.data.cards,
        skills: store.data.skills,
      });
    }
    if (typeof parsed.boardId === "string") store.data.boardId = parsed.boardId;
    if (parsed.cards && typeof parsed.cards === "object") store.data.cards = parsed.cards;
    if (parsed.customSeq != null) store.data.customSeq = parsed.customSeq;
    await store.save();
    return sendJson(res, 200, { version: store.data.version });
  }

  if (url.pathname === "/api/cli" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    const command = typeof parsed.command === "string" ? parsed.command : "";
    const r = await execCommand(command, store);
    return sendJson(res, r.ok ? 200 : 400, { ok: r.ok, output: r.output, changed: r.changed, version: store.data.version });
  }

  if (url.pathname === "/api/skills" && req.method === "GET") {
    return sendJson(res, 200, { skills: store.data.skills });
  }

  if (url.pathname === "/api/skills" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    const existing = parsed.id && store.data.skills.find((s) => s.id === parsed.id);
    if (existing) {
      if (parsed.name != null) existing.name = String(parsed.name).slice(0, 120);
      if (parsed.description != null) existing.description = String(parsed.description).slice(0, 300);
      if (parsed.content != null) existing.content = String(parsed.content);
      existing.updated = Date.now();
      await store.save();
      return sendJson(res, 200, { skill: existing, version: store.data.version });
    }
    const slug = String(parsed.id || parsed.name || "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
    if (!slug) return sendJson(res, 400, { error: "skill needs a name (used to form the id)" });
    if (store.data.skills.some((s) => s.id === slug)) return sendJson(res, 409, { error: "skill " + slug + " already exists" });
    const skill = {
      id: slug,
      name: String(parsed.name || slug).slice(0, 120),
      description: String(parsed.description || "").slice(0, 300),
      content: String(parsed.content || ""),
      updated: Date.now(),
    };
    store.data.skills.push(skill);
    await store.save();
    return sendJson(res, 200, { skill, version: store.data.version });
  }

  if (url.pathname === "/api/skills" && req.method === "DELETE") {
    const id = url.searchParams.get("id");
    const i = store.data.skills.findIndex((s) => s.id === id);
    if (i < 0) return sendJson(res, 404, { error: "no skill " + id });
    const gone = store.data.skills.splice(i, 1)[0];
    await store.save();
    return sendJson(res, 200, { deleted: gone.id, version: store.data.version });
  }

  if (url.pathname === "/api/agent" && req.method === "POST") return handleAgent(req, res, url);

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
