#!/usr/bin/env node
/* QPT parity harness — boots the Node server (qpt-ui) and the Rust server
 * (qpt_rust_ui) against identical scratch data dirs and deep-compares their
 * responses. Run from anywhere:
 *
 *   node tests/parity.mjs            # full battery
 *
 * Exits non-zero if any comparison fails. Requires: node, the built
 * qpt-server/qpt-cli binaries (cargo build), and a `kimi` CLI on PATH
 * (both servers probe it; mock mode means no real turns are made).
 */
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const RUST_ROOT = path.resolve(HERE, ".."); // qpt_rust_ui
const NODE_SRC = path.resolve(RUST_ROOT, "..", "qpt-ui"); // sibling qpt-ui
const RUST_BIN = path.join(RUST_ROOT, "target", "debug");

const BASE = fs.mkdtempSync(path.join(os.tmpdir(), "qpt-parity-"));
const NODE_DIR = path.join(BASE, "node", "workdir", "qpt-ui"); // app dir (node)
const RUST_DIR = path.join(BASE, "rust", "workdir", "app"); // app dir (rust)
const NODE_PORT = 8791;
const RUST_PORT = 8792;

/* ------------------------------------------------------------ setup */

function setupDirs() {
  fs.cpSync(NODE_SRC, NODE_DIR, {
    recursive: true,
    filter: (src) => {
      const rel = path.relative(NODE_SRC, src);
      return !(
        rel.startsWith(".git") ||
        rel === path.join("data", "store.json") ||
        rel.startsWith(path.join("data", "server.log")) ||
        rel === path.join("data", "config.json")
      );
    },
  });
  fs.mkdirSync(RUST_DIR, { recursive: true });
  fs.cpSync(path.join(RUST_ROOT, "public"), path.join(RUST_DIR, "public"), { recursive: true });
  // identical data dirs: same skills + functions, no store.json (both re-seed)
  fs.mkdirSync(path.join(RUST_DIR, "data"), { recursive: true });
  fs.cpSync(path.join(NODE_DIR, "data"), path.join(RUST_DIR, "data"), { recursive: true });
}

/* --------------------------------------------------------- processes */

const children = [];
function startServers() {
  const env = {
    ...process.env,
    QPT_AGENT_MOCK: "1",
    QPT_MOCK_DELAY_MS: "0",
    LOG_LEVEL: "error",
  };
  children.push(
    spawn("node", ["server.js"], { cwd: NODE_DIR, env: { ...env, PORT: String(NODE_PORT) }, stdio: "ignore" }),
  );
  children.push(
    spawn(path.join(RUST_BIN, "qpt-server"), [], {
      cwd: RUST_DIR,
      env: { ...env, PORT: String(RUST_PORT) },
      stdio: "ignore",
    }),
  );
}

async function waitReady(port) {
  for (let i = 0; i < 60; i++) {
    try {
      const r = await fetch(`http://localhost:${port}/api/health`);
      if (r.ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`server on ${port} never came up`);
}

function cleanup() {
  for (const c of children) {
    try { c.kill("SIGKILL"); } catch {}
  }
  try { fs.rmSync(BASE, { recursive: true, force: true }); } catch {}
}
process.on("exit", cleanup);
process.on("SIGINT", () => process.exit(130));

/* --------------------------------------------------------- comparison */

// keys scrubbed at any depth (versions, ids, times, machine paths)
const SCRUB = new Set([
  "version", "storeVersion", "jobId", "createdAt", "startedAt", "endedAt",
  "t", "updated", "dir", "location", "projectMcpPath", "builtin",
]);

function norm(v) {
  if (Array.isArray(v)) return v.map(norm);
  if (v && typeof v === "object") {
    const out = {};
    for (const k of Object.keys(v).sort()) {
      if (SCRUB.has(k)) continue;
      out[k] = norm(v[k]);
    }
    return out;
  }
  if (typeof v === "string") {
    // scrub scratch-dir paths and job ids embedded in strings
    return v
      .replaceAll(BASE, "<BASE>")
      .replace(/job-\d+-[a-z0-9]+/g, "<JOB>");
  }
  return v;
}

function deepEqual(a, b) {
  return JSON.stringify(norm(a)) === JSON.stringify(norm(b));
}

let pass = 0, fail = 0;
const failures = [];
function check(name, ok, detail) {
  if (ok) { pass++; console.log(`  ok    ${name}`); }
  else {
    fail++;
    failures.push({ name, detail });
    console.log(`  FAIL  ${name}`);
  }
}

async function battery(name, method, p, body) {
  const [rn, rr] = await Promise.all([
    fetch(`http://localhost:${NODE_PORT}${p}`, init(method, body)).catch((e) => ({ error: String(e) })),
    fetch(`http://localhost:${RUST_PORT}${p}`, init(method, body)).catch((e) => ({ error: String(e) })),
  ]);
  const [jn, jr] = [await readBody(rn), await readBody(rr)];
  check(
    name,
    rn.status === rr.status && deepEqual(jn, jr),
    { status: [rn.status, rr.status], node: jn, rust: jr },
  );
}

function init(method, body) {
  return {
    method,
    headers: body ? { "content-type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(15000),
  };
}

async function readBody(res) {
  if (res.error) return res.error;
  if (!res.text) return "<no response>";
  const text = await res.text();
  try { return JSON.parse(text); } catch { return text; }
}

/* -------------------------------------------------------------- main */

async function cliBattery(cmd) {
  const run = (bin, args, cwd) =>
    new Promise((res) => {
      const p = spawn(bin, args, { cwd });
      const killer = setTimeout(() => {
        try { p.kill("SIGKILL"); } catch {}
      }, 15000);
      let out = "", err = "";
      p.stdout.on("data", (d) => (out += d));
      p.stderr.on("data", (d) => (err += d));
      p.on("error", (e) => {
        clearTimeout(killer);
        res({ code: -1, out, err: String(e) });
      });
      p.on("close", (code) => {
        clearTimeout(killer);
        res({ code, out, err });
      });
    });
  const [n, r] = await Promise.all([
    run("node", ["cli.js", cmd], NODE_DIR),
    run(path.join(RUST_BIN, "qpt-cli"), [cmd], RUST_DIR),
  ]);
  check(`cli "${cmd}"`, n.code === r.code && n.out === r.out, {
    code: [n.code, r.code],
    node: n.out.slice(0, 400),
    rust: r.out.slice(0, 400),
  });
}

async function main() {
  setupDirs();
  startServers();
  await Promise.all([waitReady(NODE_PORT), waitReady(RUST_PORT)]);
  console.log(`parity dirs: ${BASE}\n-- api --`);

  await battery("health", "GET", "/api/health");
  await battery("state (fresh seed)", "GET", "/api/state");
  await battery("models", "GET", "/api/models");
  await battery("skills", "GET", "/api/skills");
  await battery("skills/all", "GET", "/api/skills/all");
  await battery("settings", "GET", "/api/settings");
  await battery("keys (empty)", "GET", "/api/keys");
  await battery("mcp (empty)", "GET", "/api/mcp");
  await battery("functions (empty)", "GET", "/api/functions");
  await battery("jobs (empty)", "GET", "/api/jobs");

  // CLI-over-HTTP battery (lockstep mutations keep both stores identical)
  const cliCmds = [
    "help", "boards", "cards", "cards --board dialectic", "card p1", "evaluate p1",
    "create --board protocol --title \"Ground the signup flow\" --source recruited --target grounded",
    "move custom-0 encounter", "move custom-0 gate",
    "edit custom-0 pathology=content theta=0.35",
    "move custom-0 encounter",
    "promote p9 --to dialectic",
    "skills", "skill show grounding-protocol",
    "skill create parity-skill --description \"parity check\" --content \"body text\"",
    "skill edit parity-skill description=changed",
    "skills", "skill delete parity-skill",
    "board dialectic", "cards", "evaluate d1", "board protocol",
    "bogus command",
  ];
  for (const c of cliCmds) {
    await battery(`POST /api/cli ${c}`, "POST", "/api/cli", { command: c });
  }
  await battery("state after cli battery", "GET", "/api/state");

  // state sync
  await battery("state 409 conflict", "POST", "/api/state", { clientVersion: 999, cards: {} });
  await battery("state push ok", "POST", "/api/state", { clientVersion: -1, boardId: "dialectic" }); // -1 ≠ current? both must agree
  await battery("state push (no clientVersion)", "POST", "/api/state", { boardId: "protocol" });
  await battery("state after sync", "GET", "/api/state");

  // keys
  await battery("key set", "POST", "/api/keys", { name: "PARITY_KEY", value: "abcdef1234567890" });
  await battery("keys list", "GET", "/api/keys");
  await battery("key set invalid", "POST", "/api/keys", { name: "1 bad", value: "x" });
  await battery("key delete", "DELETE", "/api/keys?name=PARITY_KEY");

  // mcp
  await battery("mcp set stdio", "POST", "/api/mcp", { name: "fs", entry: { command: "npx", args: ["-y", "@mcp/fs"] } });
  await battery("mcp set http", "POST", "/api/mcp", { name: "web", entry: { url: "https://x.example/mcp", transport: "sse" } });
  await battery("mcp list", "GET", "/api/mcp");
  await battery("mcp set invalid", "POST", "/api/mcp", { name: "bad", entry: {} });
  await battery("mcp delete", "DELETE", "/api/mcp?name=fs");

  // functions
  const fnCode = "// @description parity fn\nlet i='';process.stdin.on('data',d=>i+=d).on('end',()=>{console.log(JSON.stringify({message:'parity',actions:[{action:'evaluate_card',id:'p1'}]}))});";
  await battery("function write", "POST", "/api/functions", { name: "parity-fn", code: fnCode });
  await battery("functions list", "GET", "/api/functions");
  await battery("function run", "POST", "/api/functions/parity-fn/run", { args: {} });
  await battery("function delete", "DELETE", "/api/functions?name=parity-fn");

  // skills http CRUD
  await battery("skill create", "POST", "/api/skills", { name: "Http Skill", description: "via http", content: "body" });
  await battery("skill create dup", "POST", "/api/skills", { name: "http-skill", description: "dup", content: "x" });
  await battery("skill update", "POST", "/api/skills", { id: "http-skill", content: "v2" });
  await battery("skill delete", "DELETE", "/api/skills?id=http-skill");
  await battery("skill delete missing", "DELETE", "/api/skills?id=http-skill");
  await battery("skill import missing", "POST", "/api/skills/import", { id: "no-such-skill" });
  await battery("skill generate (mock)", "POST", "/api/skills/generate", { hint: "audit card titles for jargon" });

  // agent (mock) — one-shot
  await battery("agent mock turn", "POST", "/api/agent", { message: "create a living transformation card on the protocol board" });
  await battery("agent readOnly", "POST", "/api/agent", { message: "explain @OnboardingDropOff", readOnly: true });
  await battery("state after agent turns", "GET", "/api/state");

  // agent stream — compare event type sequences and full content
  const [sn, sr] = await Promise.all([
    fetch(`http://localhost:${NODE_PORT}/api/agent?stream=1`, init("POST", { message: "hello" })),
    fetch(`http://localhost:${RUST_PORT}/api/agent?stream=1`, init("POST", { message: "hello" })),
  ]);
  const [tn, tr] = [await readBodyLines(sn), await readBodyLines(sr)];
  check("agent stream (mock)", deepEqual(tn, tr), { node: tn, rust: tr });

  // jobs surface
  await battery("jobs submit generate", "POST", "/api/jobs", { kind: "generate", hint: "parity job" });
  await battery("jobs list", "GET", "/api/jobs");
  await battery("job 404", "GET", "/api/jobs/nope");
  await battery("jobs submit bad kind", "POST", "/api/jobs", { kind: "nope" });

  // static
  const [stn, str] = await Promise.all([
    fetch(`http://localhost:${NODE_PORT}/`).then((r) => r.text()),
    fetch(`http://localhost:${RUST_PORT}/`).then((r) => r.text()),
  ]);
  check("static / (index.html bytes)", stn === str, { node: stn.length, rust: str.length });
  await battery("static 404", "GET", "/no-such-file.js");

  // cli bins
  console.log("-- cli bins --");
  for (const c of ["help", "boards", "cards", "card p1", "evaluate p1", "state", "skills"]) {
    await cliBattery(c);
  }

  console.log(`\n${pass} ok, ${fail} FAIL`);
  if (failures.length) {
    for (const f of failures.slice(0, 5)) {
      console.log(`\n--- ${f.name}\nnode: ${JSON.stringify(f.detail?.node ?? f.detail, null, 1).slice(0, 1200)}\nrust: ${JSON.stringify(f.detail?.rust ?? "", null, 1).slice(0, 1200)}`);
    }
    process.exit(1);
  }
}

async function readBodyLines(res) {
  const text = await res.text();
  return text.split("\n").filter(Boolean).map((l) => {
    try { return JSON.parse(l); } catch { return l; }
  });
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
