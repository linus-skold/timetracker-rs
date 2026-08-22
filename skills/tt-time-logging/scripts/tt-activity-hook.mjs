#!/usr/bin/env node
// SessionStart/Stop/SubagentStop hook: writes to `tt`'s hook-only activity
// ledger (`tt agent activity …`) — see
// docs/decisions/0001-agent-activity-tracking.md. Installed by
// install-hooks.mjs.
//
// Usage: node tt-activity-hook.mjs <begin|end|subagent>
//
// `session_id` (read from the hook's JSON payload on stdin) is the key every
// entry is filed under. Missing or unparseable payload: silently skipped —
// a hook must never fail the harness event it's attached to.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

function readStdin() {
  try {
    return readFileSync(0, "utf8");
  } catch {
    return "";
  }
}

function projectName() {
  if (process.env.TT_PROJECT) return process.env.TT_PROJECT;
  try {
    const root = execFileSync("git", ["rev-parse", "--show-toplevel"], {
      encoding: "utf8",
    }).trim();
    return root.split(/[\\/]/).pop();
  } catch {
    return null;
  }
}

const event = process.argv[2];
if (!["begin", "end", "subagent"].includes(event)) {
  process.stdout.write("{}");
  process.exit(0);
}

let sessionId;
try {
  const payload = JSON.parse(readStdin());
  sessionId = payload.session_id;
} catch {
  sessionId = undefined;
}

if (!sessionId) {
  process.stdout.write("{}");
  process.exit(0);
}

const args = ["agent", "activity", event, sessionId];
if (event === "begin") {
  const project = projectName();
  if (project) args.push(project);
}

try {
  execFileSync("tt", args, { stdio: "ignore" });
} catch {
  // never fail the harness event over `tt` being missing or erroring
}

process.stdout.write("{}");
