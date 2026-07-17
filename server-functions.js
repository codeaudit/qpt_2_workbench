/* QPT Workbench — user-generated functions.
 *
 * Functions are single JS files in data/functions/<name>.js, written by the user
 * (or the agent). Contract: read JSON { args, state } from stdin, write ONE JSON
 * result to stdout:
 *
 *   { "message": "what it did", "actions": [ …action-vocabulary entries… ] }
 *
 * Actions are executed afterwards by the server through qpt-core.js — functions
 * can never bypass the specification; they compute, the core enforces.
 * A leading comment line `// @description …` provides the agent-facing summary.
 */
import { readFile, writeFile, mkdir, readdir, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const FUN_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), "data", "functions");
const RUN_TIMEOUT_MS = 20000;

function slugError(name) {
  if (!name || !/^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(name)) return "function name: lowercase slug (a-z 0-9 -)";
  return null;
}

export async function listFunctions() {
  const out = [];
  try {
    const files = await readdir(FUN_DIR);
    for (const f of files.filter((x) => x.endsWith(".js"))) {
      const name = f.slice(0, -3);
      const code = await readFile(path.join(FUN_DIR, f), "utf8");
      const m = /^\/\/\s*@description\s+(.+)$/m.exec(code);
      out.push({ name, description: m ? m[1].trim() : "", lines: code.split("\n").length });
    }
  } catch { /* no functions dir yet */ }
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}

export async function readFunction(name) {
  if (slugError(name)) throw new Error(slugError(name));
  return { name, code: await readFile(path.join(FUN_DIR, name + ".js"), "utf8") };
}

export async function writeFunction(name, code) {
  const e = slugError(name);
  if (e) throw new Error(e);
  if (!code || !String(code).trim()) throw new Error("function code required");
  await mkdir(FUN_DIR, { recursive: true });
  await writeFile(path.join(FUN_DIR, name + ".js"), String(code));
  return name;
}

export async function deleteFunction(name) {
  if (slugError(name)) throw new Error(slugError(name));
  await rm(path.join(FUN_DIR, name + ".js"));
  return name;
}

// run one function in a child process: JSON in, JSON out, hard timeout
export async function runFunction(name, args, state) {
  await readFunction(name); // throws if missing
  const payload = JSON.stringify({ args: args || {}, state });
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.join(FUN_DIR, name + ".js")], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env },
    });
    let stdout = "", stderr = "";
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("function " + name + " timed out after " + RUN_TIMEOUT_MS / 1000 + "s"));
    }, RUN_TIMEOUT_MS);
    child.stdout.on("data", (d) => { stdout += d; });
    child.stderr.on("data", (d) => { stderr += d; });
    child.on("error", (e) => { clearTimeout(timer); reject(e); });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (code !== 0) return reject(new Error("function " + name + " exited " + code + (stderr ? ": " + stderr.slice(0, 300) : "")));
      const line = stdout.trim().split("\n").filter(Boolean).pop() || "";
      try {
        resolve(JSON.parse(line));
      } catch {
        resolve({ message: stdout.trim().slice(0, 2000), actions: [] });
      }
    });
    child.stdin.write(payload);
    child.stdin.end();
  });
}

export const FUNCTIONS_DIR = FUN_DIR;
