#!/usr/bin/env node
// Claude-Code-specific enforcement for the tt-time-logging skill.
//
// `npx skills add` only copies SKILL.md (and this directory) into place — it
// knows nothing about Claude Code's hooks, and prose alone gets skipped under
// context pressure. This script wires hooks into the user's *global*
// ~/.claude/settings.json (not project-local, since they must fire in every
// session):
//
//   SessionStart - injects this skill's contract; opens the activity window.
//   Stop         - warns about open marks; closes the activity window.
//   SubagentStop - records a subagent dispatch on the activity window.
//
// The activity ledger is a second, model-independent signal that a session
// was active — see docs/decisions/0001-agent-activity-tracking.md. Keyed by
// Claude Code's own `session_id`, read off the hook's stdin JSON.
//
// All hook commands use absolute paths under ~/.claude/hooks/ — relative
// paths silently fail to resolve on at least Windows.
//
// Safe to re-run: entries are added only if not already present.
//
// Usage (from anywhere, after `npx skills add ...`):
//   node <wherever the skill landed>/scripts/install-hooks.mjs

import { existsSync, mkdirSync, readFileSync, writeFileSync, copyFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skillDir = dirname(scriptDir); // .../tt-time-logging

const claudeHome = join(homedir(), ".claude");
if (!existsSync(claudeHome)) {
  console.log(
    "Claude Code doesn't appear to be installed for this user (no " +
      `${claudeHome} found) — skipping hook installation. Run Claude Code ` +
      "at least once, then re-run this script.",
  );
  process.exit(0);
}

const settingsPath = join(claudeHome, "settings.json");
// Namespaced by skill name so it can't collide with another skill's hooks,
// and so the files the hooks reference live at a stable path independent of
// wherever `npx skills add` put this skill (project-local or global).
const destDir = join(claudeHome, "hooks", "tt-time-logging");
const skillMdDest = join(destDir, "SKILL.md");
const stopCheckDest = join(destDir, "tt-stop-check.mjs");
const activityHookDest = join(destDir, "tt-activity-hook.mjs");

mkdirSync(destDir, { recursive: true });
copyFileSync(join(skillDir, "SKILL.md"), skillMdDest);
copyFileSync(join(scriptDir, "tt-stop-check.mjs"), stopCheckDest);
copyFileSync(join(scriptDir, "tt-activity-hook.mjs"), activityHookDest);

// Forward slashes only: Node's fs calls accept them on every OS, and it
// sidesteps having to escape backslashes inside the nested JS string literal
// below.
const toFwd = (p) => p.replace(/\\/g, "/");
const skillMdAbs = toFwd(skillMdDest);
const stopCheckAbs = toFwd(stopCheckDest);
const activityHookAbs = toFwd(activityHookDest);

let settings = {};
if (existsSync(settingsPath)) {
  settings = JSON.parse(readFileSync(settingsPath, "utf8"));
}

settings.hooks ??= {};
settings.hooks.SessionStart ??= [];
settings.hooks.Stop ??= [];
settings.hooks.SubagentStop ??= [];

const sessionStartCmd = `node -e "const fs=require('fs');process.stdout.write(JSON.stringify({hookSpecificOutput:{hookEventName:'SessionStart',additionalContext:fs.readFileSync('${skillMdAbs}','utf8')}}))"`;
// Quoted: an absolute home path can contain spaces (e.g. "C:/Users/John Doe/...").
const stopCmd = `node "${stopCheckAbs}"`;
const activityBeginCmd = `node "${activityHookAbs}" begin`;
const activityEndCmd = `node "${activityHookAbs}" end`;
const activitySubagentCmd = `node "${activityHookAbs}" subagent`;

const hasCommand = (event, command) =>
  settings.hooks[event].some((entry) => entry.hooks?.some((h) => h.command === command));

if (!hasCommand("SessionStart", sessionStartCmd)) {
  settings.hooks.SessionStart.push({
    hooks: [{ type: "command", command: sessionStartCmd, statusMessage: "Loading tt-time-logging contract" }],
  });
}

if (!hasCommand("SessionStart", activityBeginCmd)) {
  settings.hooks.SessionStart.push({
    hooks: [{ type: "command", command: activityBeginCmd, statusMessage: "Opening tt activity window" }],
  });
}

if (!hasCommand("Stop", stopCmd)) {
  settings.hooks.Stop.push({
    hooks: [{ type: "command", command: stopCmd, statusMessage: "Checking for unclosed tt marks" }],
  });
}

if (!hasCommand("Stop", activityEndCmd)) {
  settings.hooks.Stop.push({
    hooks: [{ type: "command", command: activityEndCmd, statusMessage: "Closing tt activity window" }],
  });
}

if (!hasCommand("SubagentStop", activitySubagentCmd)) {
  settings.hooks.SubagentStop.push({
    hooks: [{ type: "command", command: activitySubagentCmd, statusMessage: "Recording tt subagent activity" }],
  });
}

writeFileSync(settingsPath, JSON.stringify(settings, null, 2) + "\n");

console.log(`tt-time-logging hooks installed into ${settingsPath}.`);
console.log(`Contract, stop-check and activity-hook scripts copied into ${destDir}.`);
console.log("Restart Claude Code or open /hooks once so the new settings file is picked up.");
