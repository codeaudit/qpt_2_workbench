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
import { listSkills, readSkill, createSkill, updateSkill, deleteSkill, listAllSkills, importSkill, readCliConfigToml, ensureSkillGenerator, GENERATOR_SKILL_ID, slugify } from "./server-skills.js";
import { registerHandler, submitJob, getJob, listJobs, cancelJob, note } from "./server-jobs.js";
import { loadSettings } from "./server-settings.js";
import { listFunctions, readFunction, writeFunction, deleteFunction, runFunction } from "./server-functions.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, ".."); // project root (the spec lives here)
const PORT = parseInt(process.env.PORT || "8787", 10);
const MOCK = process.env.QPT_AGENT_MOCK === "1";
const TURN_TIMEOUT_MS = 150000;

const store = await openStore(); // canonical state: cards, board, skills (data/store.json)
await ensureSkillGenerator();      // the meta-skill, editable like any other SKILL.md
const settings = await loadSettings(); // workbench config: keys, prompt, MCP (data/config.json)

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
  {"action": "evaluate_card", "id": "..."},
  {"action": "run_function", "name": "...", "args": {...}}
 ]}
Use only the fields each action needs. create_card: source/target only matter on protocol; kind/reliability only on dialectic; tags only on resolution. Card ids in the state are current — use them verbatim. At most 12 actions; order them so each is legal when it runs (remember horizon = 1: multi-column journeys need one move per column).

EXECUTION: your actions run on the canonical server-side store via the shared domain core — the same code the UI uses. Anything illegal comes back refused in the results.

USER FUNCTIONS: state.functions lists user-generated functions (name, description). Call them with run_function — they compute and may return further actions, which execute through the same core. Prefer a function over hand-rolling a procedure when one matches.

MCP: if the CLI session exposes MCP tools (state.mcpServers lists configured servers), you may use them for reading/investigating; your REQUIRED output channel remains the JSON plan above.

SKILLS: state.skills lists user-authored instruction sets. When a skill's description matches the request, follow its content — skills are the user's way of teaching you their house rules. Users can also drive the store directly through a text CLI (help · state · cards · create · move · edit · evaluate · promote · skills …) exposed at POST /api/cli; you do not need it — your channel is the action vocabulary above.

You may read files in your working directory for exact semantics: QPT_2x_Consolidated_Specification_r2.7.md (the full spec) and qpt-ui/qpt-data.js (seeded content).`;

/* --------------------------------------------------------- SDK session */

const EXE = process.env.KIMI_EXECUTABLE || "kimi";
let sessionPromise = null;
let currentModel = process.env.KIMI_MODEL || null; // null = CLI default

// The SDK's parseConfig() is lossy with real-world config.toml files (it dropped
// 2 of 3 registered models and misread the default), so we read the CLI config
// ourselves (via server-skills) and keep the SDK call as a fallback.
function parseModelsToml(text) {
  const models = [];
  let defaultModel = null;
  let cur = null;
  const val = (s) => {
    s = s.trim();
    if (s.startsWith('"')) return s.slice(1, s.lastIndexOf('"'));
    if (s === "true") return true;
    if (s === "false") return false;
    if (s.startsWith("[")) return [...s.matchAll(/"([^"]*)"/g)].map((m) => m[1]);
    const n = Number(s);
    return isNaN(n) ? s : n;
  };
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const sec = /^\[models\."([^"]+)"\]\s*$/.exec(line);
    if (sec) { cur = { id: sec[1] }; models.push(cur); continue; }
    if (line.startsWith("[")) { cur = null; continue; }
    const kv = /^([A-Za-z_]+)\s*=\s*(.+)$/.exec(line);
    if (!kv) continue;
    if (cur) cur[kv[1]] = val(kv[2]);
    else if (kv[1] === "default_model") defaultModel = val(kv[2]);
  }
  return {
    defaultModel,
    models: models.map((m) => ({
      id: m.id,
      name: m.display_name || m.id,
      provider: m.provider,
      model: m.model,
      capabilities: m.capabilities || [],
      maxContextSize: m.max_context_size,
      defaultEffort: m.default_effort,
    })),
  };
}

async function listModels() {
  const toml = await readCliConfigToml();
  if (toml) {
    const parsed = parseModelsToml(toml);
    if (parsed.models.length) return { ...parsed, source: "config.toml" };
  }
  try {
    const { parseConfig } = await import("@moonshot-ai/kimi-agent-sdk");
    const cfg = parseConfig();
    return { models: cfg.models || [], defaultModel: cfg.defaultModel || null, defaultThinking: !!cfg.defaultThinking, source: "sdk" };
  } catch {
    return { models: [], defaultModel: null, defaultThinking: false, source: "none" };
  }
}

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
        env: { ...process.env, ...settings.envForAgent() },
      };
      if (currentModel) opts.model = currentModel;
      const session = createSession(opts);
      return { session, greeted: false };
    })();
    sessionPromise.catch(() => { sessionPromise = null; });
  }
  return sessionPromise;
}

function buildPrompt(message, state, includeInstructions, instructions) {
  return (
    (includeInstructions ? (instructions || settings.cfg.systemPrompt || INSTRUCTIONS) + "\n\n" : "") +
    "CURRENT STATE (JSON):\n" + JSON.stringify(state) +
    "\n\nUSER REQUEST:\n" + message +
    "\n\nRespond with ONLY the JSON object described."
  );
}

async function runAgentSdk(message, state, onProgress, instructions, cancelReg) {
  const ctx = await getSession();
  const turn = ctx.session.prompt(buildPrompt(message, state, ctx.greeted && !instructions, instructions));
  ctx.greeted = true;
  if (cancelReg) cancelReg(() => turn.interrupt());
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
async function runAgentCompat(message, state, onProgress, instructions, cancelReg) {
  const { spawn } = await import("node:child_process");
  const args = ["-p", buildPrompt(message, state, true, instructions), "--output-format", "stream-json"];
  if (currentModel) args.push("--model", currentModel);
  return new Promise((resolve, reject) => {
    const child = spawn(EXE, args, { cwd: ROOT, env: { ...process.env, ...settings.envForAgent() } });
    if (cancelReg) cancelReg(() => child.kill("SIGKILL"));
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

async function runAgent(message, state, onProgress, instructions, cancelReg) {
  const backend = await detectBackend();
  return backend === "sdk"
    ? runAgentSdk(message, state, onProgress, instructions, cancelReg)
    : runAgentCompat(message, state, onProgress, instructions, cancelReg);
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
      case "run_function":
        if (typeof a.name === "string" && /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(a.name)) {
          actions.push({ action: "run_function", name: a.name, args: a.args && typeof a.args === "object" ? a.args : {} });
        } else warnings.push("run_function: needs a valid function name");
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

async function agentState() {
  const s = store.CORE.compactState(store.data);
  const { skills } = await listSkills();
  s.skills = skills.map((sk) => ({ id: sk.id, name: sk.name, description: sk.description, content: sk.content }));
  s.functions = await listFunctions();
  s.mcpServers = Object.keys(settings.cfg.mcpServers);
  return s;
}

// execute the validated plan on the canonical store via the shared core
async function executePlan(validated) {
  const results = [];
  let changed = false;
  for (const a of validated.actions) {
    if (a.action === "run_function") {
      try {
        const out = await runFunction(a.name, a.args, store.CORE.compactState(store.data));
        let sub = [];
        if (Array.isArray(out.actions)) {
          for (const sa of out.actions.slice(0, 6)) {
            const r = store.CORE.applyAction(store.data, Object.assign({ via: "function " + a.name }, sa));
            sub.push(r.message);
            if (r.ok) changed = true;
          }
        }
        results.push({ action: "run_function", ok: true, message: (out.message || a.name + " done") + (sub.length ? " [" + sub.join("; ") + "]" : "") });
      } catch (e) {
        results.push({ action: "run_function", ok: false, message: String((e && e.message) || e) });
      }
      continue;
    }
    const r = store.CORE.applyAction(store.data, Object.assign({ via: "the Kimi agent" }, a));
    results.push({ action: a.action, ok: r.ok, message: r.message });
    if (r.ok) changed = true;
  }
  if (changed) await store.save();
  return { ...validated, results, version: store.data.version };
}

async function produce(message, _clientState, onProgress, cancelReg) {
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
  const text = await runAgent(message, await agentState(), onProgress, null, cancelReg);
  const plan = extractJson(text);
  const validated = plan
    ? validatePlan(plan)
    : { reply: "The agent did not return a parseable plan. Raw reply: " + text.slice(0, 1500), actions: [], warnings: ["unparseable plan"] };
  return executePlan(validated);
}

/* ---------------------------------------------------------- job handlers */

function waitJob(job, timeoutMs) {
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error("job timed out")), timeoutMs || TURN_TIMEOUT_MS + 30000);
    job.emitter.on("done", () => {
      clearTimeout(t);
      if (job.status === "done") resolve(job.result);
      else {
        const e = new Error(job.error || "job " + job.status);
        e.status = job.status === "cancelled" ? 499 : 500;
        reject(e);
      }
    });
  });
}

registerHandler("agent", async (job) =>
  produce(job.payload.message, null, (d) => note(job, d), (fn) => { job._cancel = fn; }));

registerHandler("generate", async (job) => {
  const hint = job.payload.hint;
  if (MOCK) {
    note(job, "mock backend: generating canned draft…");
    const words = hint.toLowerCase().replace(/[^a-z0-9\s]/g, "").split(/\s+/).filter(Boolean).slice(0, 3);
    return {
      draft: {
        name: slugify(words.join("-")) || "generated-skill",
        description: "[mock] Skill for: " + hint.slice(0, 120) + ". Use when the request involves it.",
        content: "# " + hint.slice(0, 80) + "\n\n1. Understand the request.\n2. Apply the procedure.\n3. Report the result.\n\n[mock draft — run without QPT_AGENT_MOCK for a real generation]",
      },
    };
  }
  const generator = await readSkill(GENERATOR_SKILL_ID);
  const taken = (await listAllSkills()).scopes.flatMap((sc) => sc.skills.map((s) => s.name.toLowerCase()));
  const message =
    "HINT for the new skill: " + hint +
    "\n\nTAKEN skill names (do not reuse): " + (taken.join(", ") || "(none)") +
    "\n\nWrite the skill now, per your output contract.";
  const text = await runAgent(message, {}, (d) => note(job, d), generator.content, (fn) => { job._cancel = fn; });
  const plan = extractJson(text);
  const draft = plan && plan.draft ? plan.draft : plan;
  if (!draft || typeof draft.name !== "string" || !draft.description || !draft.content) {
    const err = new Error("the generator returned no usable draft");
    err.raw = text.slice(0, 1200);
    throw err;
  }
  const name = slugify(draft.name);
  if (taken.includes(name)) {
    const err = new Error("generated name collides with an existing skill: " + name);
    err.status = 409;
    err.draft = { ...draft, name };
    throw err;
  }
  return { draft: { name, description: String(draft.description), content: String(draft.content) } };
});

async function handleAgent(req, res, url) {
  let body = "";
  for await (const chunk of req) body += chunk;
  let parsed;
  try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
  const message = typeof parsed.message === "string" ? parsed.message.trim() : "";
  if (!message) return sendJson(res, 400, { error: "message required" });
  const stream = url.searchParams.get("stream") === "1";

  const job = submitJob("agent", { message });

  if (stream) {
    res.writeHead(200, {
      "content-type": "application/x-ndjson; charset=utf-8",
      "access-control-allow-origin": "*",
      "cache-control": "no-cache",
    });
    const send = (obj) => res.write(JSON.stringify(obj) + "\n");
    send({ type: "queued", jobId: job.id });
    const onProgress = (detail) => send({ type: "progress", detail });
    const onDone = () => {
      res.removeListener("close", onClose);
      if (job.status === "done") send({ type: "result", jobId: job.id, ...job.result });
      else if (job.status === "cancelled") send({ type: "error", jobId: job.id, error: "job cancelled" });
      else send({ type: "error", jobId: job.id, error: job.error || "job failed", hint: "Check that the kimi CLI is installed and logged in (kimi --version), then retry." });
      res.end();
    };
    const onClose = () => {
      job.emitter.removeListener("progress", onProgress);
      job.emitter.removeListener("done", onDone);
      // the job keeps running server-side — inspect or cancel it via /api/jobs
    };
    job.emitter.on("progress", onProgress);
    job.emitter.on("done", onDone);
    res.on("close", onClose);
    return; // response ends when the job does
  }

  try {
    const plan = await waitJob(job);
    return sendJson(res, 200, { jobId: job.id, ...plan });
  } catch (err) {
    return sendJson(res, err.status === 499 ? 499 : 503, {
      jobId: job.id,
      error: "agent unavailable: " + String((err && err.message) || err),
      hint: "Check that the kimi CLI is installed and logged in (kimi --version), then retry.",
    });
  }
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  if (req.method === "OPTIONS") return sendJson(res, 204, {});
  if (url.pathname === "/api/health") {
    return sendJson(res, 200, { ok: true, mock: MOCK, backend: MOCK ? "mock" : await detectBackend(), executable: EXE, model: currentModel, storeVersion: store.data.version });
  }

  if (url.pathname === "/api/models" && req.method === "GET") {
    const cfg = await listModels();
    return sendJson(res, 200, {
      models: cfg.models,
      defaultModel: cfg.defaultModel,
      current: currentModel || cfg.defaultModel,
      source: cfg.source,
    });
  }

  if (url.pathname === "/api/model" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    const cfg = await listModels();
    const wanted = typeof parsed.model === "string" && parsed.model.trim() ? parsed.model.trim() : null;
    if (wanted && cfg.models.length && !cfg.models.some((m) => m.id === wanted)) {
      return sendJson(res, 400, { error: "unknown model " + wanted, models: cfg.models.map((m) => m.id) });
    }
    currentModel = wanted; // null → CLI default
    sessionPromise = null; // rebuild the SDK session with the new model
    return sendJson(res, 200, { current: currentModel || cfg.defaultModel });
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
    const { skills, warnings } = await listSkills();
    return sendJson(res, 200, { skills, warnings });
  }

  // browse skills across every scope the CLI knows: workbench · user · project · extra
  if (url.pathname === "/api/skills/all" && req.method === "GET") {
    return sendJson(res, 200, await listAllSkills());
  }

  // copy an external skill into the workbench store
  if (url.pathname === "/api/skills/import" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    try {
      const skill = await importSkill(parsed.id);
      return sendJson(res, 200, { skill });
    } catch (e) {
      const msg = String((e && e.message) || e);
      return sendJson(res, msg.includes("already exists") ? 409 : 404, { error: msg });
    }
  }

  // invoke the skill-generator skill: hint → draft {name, description, content}
  if (url.pathname === "/api/skills/generate" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    const hint = typeof parsed.hint === "string" ? parsed.hint.trim() : "";
    if (!hint) return sendJson(res, 400, { error: "hint required" });
    const job = submitJob("generate", { hint });
    try {
      const result = await waitJob(job);
      return sendJson(res, 200, { jobId: job.id, ...result });
    } catch (e) {
      return sendJson(res, e.status === 409 ? 409 : e.status === 499 ? 499 : 503, {
        jobId: job.id,
        error: String((e && e.message) || e),
        draft: e.draft,
        raw: e.raw,
      });
    }
  }

  /* ------------------------------------------------------------- settings */

  if (url.pathname === "/api/settings" && req.method === "GET") {
    return sendJson(res, 200, {
      systemPrompt: settings.cfg.systemPrompt,
      defaultSystemPrompt: INSTRUCTIONS,
      keys: settings.maskedKeys(),
      mcpServers: settings.cfg.mcpServers,
      projectMcpPath: settings.projectMcpPath,
    });
  }

  if (url.pathname === "/api/settings" && req.method === "PUT") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    if ("systemPrompt" in parsed) await settings.setSystemPrompt(parsed.systemPrompt);
    return sendJson(res, 200, { systemPrompt: settings.cfg.systemPrompt });
  }

  if (url.pathname === "/api/keys" && req.method === "GET") {
    return sendJson(res, 200, { keys: settings.maskedKeys() });
  }
  if (url.pathname === "/api/keys" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    try {
      const name = await settings.setKey(parsed.name, parsed.value);
      return sendJson(res, 200, { name, keys: settings.maskedKeys() });
    } catch (e) { return sendJson(res, 400, { error: String((e && e.message) || e) }); }
  }
  if (url.pathname === "/api/keys" && req.method === "DELETE") {
    await settings.deleteKey(url.searchParams.get("name") || "");
    return sendJson(res, 200, { keys: settings.maskedKeys() });
  }

  if (url.pathname === "/api/mcp" && req.method === "GET") {
    return sendJson(res, 200, { mcpServers: settings.cfg.mcpServers, projectMcpPath: settings.projectMcpPath });
  }
  if (url.pathname === "/api/mcp" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    try {
      const name = await settings.setMcpServer(parsed.name, parsed.entry || parsed);
      return sendJson(res, 200, { name, mcpServers: settings.cfg.mcpServers });
    } catch (e) { return sendJson(res, 400, { error: String((e && e.message) || e) }); }
  }
  if (url.pathname === "/api/mcp" && req.method === "DELETE") {
    try {
      await settings.deleteMcpServer(url.searchParams.get("name") || "");
      return sendJson(res, 200, { mcpServers: settings.cfg.mcpServers });
    } catch (e) { return sendJson(res, 400, { error: String((e && e.message) || e) }); }
  }

  if (url.pathname === "/api/functions" && req.method === "GET") {
    return sendJson(res, 200, { functions: await listFunctions() });
  }
  if (url.pathname === "/api/functions" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    try {
      const name = await writeFunction(parsed.name, parsed.code);
      return sendJson(res, 200, { name, functions: await listFunctions() });
    } catch (e) { return sendJson(res, 400, { error: String((e && e.message) || e) }); }
  }
  if (url.pathname === "/api/functions" && req.method === "DELETE") {
    try {
      const name = await deleteFunction(url.searchParams.get("name") || "");
      return sendJson(res, 200, { name, functions: await listFunctions() });
    } catch (e) { return sendJson(res, 404, { error: String((e && e.message) || e) }); }
  }

  const funMatch = /^\/api\/functions\/([a-z0-9](?:[a-z0-9-]*[a-z0-9])?)\/run$/.exec(url.pathname);
  if (funMatch && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body || "{}"); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    try {
      const out = await runFunction(funMatch[1], parsed.args, store.CORE.compactState(store.data));
      let applied = [];
      if (Array.isArray(out.actions)) {
        for (const sa of out.actions.slice(0, 6)) {
          const r = store.CORE.applyAction(store.data, Object.assign({ via: "function " + funMatch[1] }, sa));
          applied.push({ ok: r.ok, message: r.message });
        }
        if (applied.some((x) => x.ok)) await store.save();
      }
      return sendJson(res, 200, { output: out, applied, version: store.data.version });
    } catch (e) {
      return sendJson(res, 400, { error: String((e && e.message) || e) });
    }
  }

  /* ------------------------------------------------ agentic job inspection */

  if (url.pathname === "/api/jobs" && req.method === "GET") {
    return sendJson(res, 200, { jobs: listJobs() });
  }

  // the common way in: submit any agentic job, get its id back immediately
  if (url.pathname === "/api/jobs" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    try {
      let payload;
      if (parsed.kind === "generate") {
        payload = { hint: String(parsed.hint || "").trim() };
        if (!payload.hint) return sendJson(res, 400, { error: "hint required" });
      } else if (parsed.kind === "agent") {
        payload = { message: String(parsed.message || "").trim() };
        if (!payload.message) return sendJson(res, 400, { error: "message required" });
      } else {
        return sendJson(res, 400, { error: "unknown job kind " + JSON.stringify(parsed.kind) });
      }
      const job = submitJob(parsed.kind, payload);
      return sendJson(res, 200, { jobId: job.id, status: job.status });
    } catch (e) {
      return sendJson(res, 400, { error: String((e && e.message) || e) });
    }
  }

  const jobMatch = /^\/api\/jobs\/([^/]+)(\/cancel)?$/.exec(url.pathname);
  if (jobMatch) {
    const job = getJob(jobMatch[1]);
    if (!job) return sendJson(res, 404, { error: "no job " + jobMatch[1] });
    if (jobMatch[2] === "/cancel" && req.method === "POST") {
      cancelJob(job.id);
      return sendJson(res, 200, { job: listJobs().find((j) => j.id === job.id) });
    }
    if (!jobMatch[2] && req.method === "GET") {
      return sendJson(res, 200, {
        job: listJobs().find((j) => j.id === job.id),
        progress: job.progress,
        result: job.status === "done" ? job.result : undefined,
        error: job.error,
      });
    }
  }

  if (url.pathname === "/api/skills" && req.method === "POST") {
    let body = "";
    for await (const chunk of req) body += chunk;
    let parsed;
    try { parsed = JSON.parse(body); } catch { return sendJson(res, 400, { error: "invalid JSON body" }); }
    try {
      if (parsed.id) {
        // update (name, description, content, and spec optional fields)
        const skill = await updateSkill(parsed.id, parsed);
        return sendJson(res, 200, { skill });
      }
      const skill = await createSkill(parsed);
      return sendJson(res, 200, { skill });
    } catch (e) {
      const msg = String((e && e.message) || e);
      return sendJson(res, msg.includes("already exists") ? 409 : msg.includes("no valid") || msg.includes("ENOENT") ? 404 : 400, { error: msg });
    }
  }

  if (url.pathname === "/api/skills" && req.method === "DELETE") {
    try {
      const id = await deleteSkill(url.searchParams.get("id"));
      return sendJson(res, 200, { deleted: id });
    } catch (e) {
      return sendJson(res, 404, { error: "no skill " + url.searchParams.get("id") });
    }
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
