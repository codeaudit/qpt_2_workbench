#!/usr/bin/env node
/* QPT Workbench — standalone CLI over the local store (no server needed).
 *
 *   node cli.js "cards --board protocol"
 *   node cli.js "move p1 gate"
 *   node cli.js "skill create grounding-protocol --name Grounding --content '…'"
 */
import { openStore } from "./server-store.js";
import { execCommand } from "./cli-exec.js";

const line = process.argv.slice(2).join(" ").trim();
if (!line) {
  console.log('usage: node cli.js "<command>" — try: node cli.js "help"');
  process.exit(1);
}

const store = await openStore();
const r = await execCommand(line, store);
console.log(r.output);
process.exit(r.ok ? 0 : 1);
