/* QPT Workbench — settings store.
 *
 * Workbench-owned configuration in data/config.json (gitignored):
 *   apiKeys      named secrets, injected as env into agent turns (masked in the UI)
 *   systemPrompt override for the agent's instructions (null = built-in default)
 *   mcpServers   MCP servers we manage, synced into the project-level
 *                .kimi-code/mcp.json that agent turns load (other entries preserved)
 */
import { readFile, writeFile, mkdir, rename, copyFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const CONFIG_FILE = path.join(__dirname, "data", "config.json");
const PROJECT_MCP = path.join(ROOT, ".kimi-code", "mcp.json");

const DEFAULTS = { apiKeys: {}, systemPrompt: null, mcpServers: {}, managedMcpNames: [] };

export async function loadSettings() {
  let cfg;
  try {
    cfg = JSON.parse(await readFile(CONFIG_FILE, "utf8"));
  } catch {
    cfg = { ...DEFAULTS };
  }
  cfg.apiKeys = cfg.apiKeys || {};
  cfg.mcpServers = cfg.mcpServers || {};
  cfg.managedMcpNames = cfg.managedMcpNames || [];

  async function save() {
    await mkdir(path.dirname(CONFIG_FILE), { recursive: true });
    const tmp = CONFIG_FILE + ".tmp";
    await writeFile(tmp, JSON.stringify(cfg, null, 1));
    await rename(tmp, CONFIG_FILE);
  }

  /* ---- api keys (values never leave the server; masked in all responses) ---- */

  function maskedKeys() {
    return Object.entries(cfg.apiKeys).map(([name, value]) => ({
      name,
      masked: value.length <= 8 ? "••••••••" : value.slice(0, 4) + "…" + value.slice(-4),
    }));
  }
  async function setKey(name, value) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error("key name must be an env-var style identifier");
    if (!value || !String(value).trim()) throw new Error("key value required");
    cfg.apiKeys[name] = String(value).trim();
    await save();
    return name;
  }
  async function deleteKey(name) {
    delete cfg.apiKeys[name];
    await save();
  }
  function envForAgent() {
    // injected into the child process / SDK session environment for agent turns
    return { ...cfg.apiKeys };
  }

  /* ---- system prompt override ---- */

  async function setSystemPrompt(text) {
    cfg.systemPrompt = text && text.trim() ? String(text) : null;
    await save();
  }

  /* ---- MCP servers (synced to the project-level mcp.json) ---- */

  async function syncMcpFile() {
    let existing = { mcpServers: {} };
    try {
      existing = JSON.parse(await readFile(PROJECT_MCP, "utf8"));
      if (!existing.mcpServers) existing.mcpServers = {};
    } catch { /* new file */ }
    // preserve entries we do not manage; replace ours wholesale
    for (const name of cfg.managedMcpNames) delete existing.mcpServers[name];
    Object.assign(existing.mcpServers, cfg.mcpServers);
    cfg.managedMcpNames = Object.keys(cfg.mcpServers);
    await mkdir(path.dirname(PROJECT_MCP), { recursive: true });
    try {
      await copyFile(PROJECT_MCP, PROJECT_MCP + ".bak");
    } catch { /* no previous file to back up */ }
    await writeFile(PROJECT_MCP, JSON.stringify(existing, null, 1));
    await save();
  }

  function validateMcpEntry(name, entry) {
    if (!/^[a-z0-9][a-z0-9-_]*$/i.test(name)) throw new Error("server name: slug (letters, numbers, - _)");
    if (!entry || typeof entry !== "object") throw new Error("server entry required");
    if (entry.command) {
      if (!Array.isArray(entry.args)) entry.args = entry.args ? String(entry.args).split(" ") : [];
      return { command: String(entry.command), args: entry.args.map(String), ...(entry.env ? { env: entry.env } : {}), ...(entry.cwd ? { cwd: String(entry.cwd) } : {}), ...(entry.enabled === false ? { enabled: false } : {}) };
    }
    if (entry.url) {
      const out = { url: String(entry.url) };
      if (entry.transport === "sse") out.transport = "sse";
      if (entry.headers) out.headers = entry.headers;
      if (entry.bearerTokenEnvVar) out.bearerTokenEnvVar = String(entry.bearerTokenEnvVar);
      if (entry.enabled === false) out.enabled = false;
      return out;
    }
    throw new Error("server needs either command (stdio) or url (http/sse)");
  }

  async function setMcpServer(name, entry) {
    cfg.mcpServers[name] = validateMcpEntry(name, entry);
    await syncMcpFile();
    return name;
  }
  async function deleteMcpServer(name) {
    delete cfg.mcpServers[name];
    await syncMcpFile();
  }

  return {
    cfg,
    save,
    maskedKeys,
    setKey,
    deleteKey,
    envForAgent,
    setSystemPrompt,
    setMcpServer,
    deleteMcpServer,
    projectMcpPath: PROJECT_MCP,
  };
}
