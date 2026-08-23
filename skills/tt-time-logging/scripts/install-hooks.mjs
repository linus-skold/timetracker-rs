#!/usr/bin/env node
// Claude-Code-specific enforcement for the tt-time-logging skill.
//
// `npx skills add` only copies SKILL.md (and this directory) into place — it
// knows nothing about Claude Code's hooks, and prose alone gets skipped under
// context pressure. This script wires hooks into the user's *global*
// ~/.claude/settings.json (not project-local, since they must fire in every
// session):
//
//   SessionStart     - injects this skill's full contract; opens the activity
//                      window.
//   UserPromptSubmit - re-injects only the operating card on every prompt, so
//                      the begin/touch/end discipline survives context getting
//                      pushed out in a long session without re-spending the
//                      whole document every turn. See tt-contract-hook.mjs.
//   Stop             - warns about open marks; closes the activity window.
//   SubagentStop     - records a subagent dispatch on the activity window.
//
// It also appends a one-line pointer to this skill into the user's global
// ~/.claude/CLAUDE.md (if that file exists and doesn't already mention the
// skill), mirroring how other global skills advertise themselves there.
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
// Sits beside the SKILL.md copy on purpose: it reads the contract from its own
// directory, so the pair stays self-contained wherever it's installed.
const contractHookDest = join(destDir, "tt-contract-hook.mjs");

mkdirSync(destDir, { recursive: true });
copyFileSync(join(skillDir, "SKILL.md"), skillMdDest);
copyFileSync(join(scriptDir, "tt-stop-check.mjs"), stopCheckDest);
copyFileSync(join(scriptDir, "tt-activity-hook.mjs"), activityHookDest);
copyFileSync(join(scriptDir, "tt-contract-hook.mjs"), contractHookDest);

// Forward slashes only: Node's fs calls accept them on every OS, and it keeps
// the commands written into settings.json free of escaped backslashes.
const toFwd = (p) => p.replace(/\\/g, "/");
const stopCheckAbs = toFwd(stopCheckDest);
const activityHookAbs = toFwd(activityHookDest);
const contractHookAbs = toFwd(contractHookDest);

let settings = {};
if (existsSync(settingsPath)) {
  settings = JSON.parse(readFileSync(settingsPath, "utf8"));
}

settings.hooks ??= {};
settings.hooks.SessionStart ??= [];
settings.hooks.UserPromptSubmit ??= [];
settings.hooks.Stop ??= [];
settings.hooks.SubagentStop ??= [];

// Earlier versions inlined the injection as a `node -e "…readFileSync(SKILL.md)…"`
// one-liner on both events. Drop those: left in place they'd fire alongside the
// new hook, injecting the contract twice per prompt — and the inline form always
// injected the whole file, which is the thing the card split exists to stop.
const isLegacyInjector = (command) =>
  typeof command === "string" &&
  command.startsWith("node -e ") &&
  command.includes("tt-time-logging");

let removedLegacy = 0;
for (const event of ["SessionStart", "UserPromptSubmit"]) {
  settings.hooks[event] = settings.hooks[event]
    .map((entry) => {
      if (!Array.isArray(entry.hooks)) return entry;
      const kept = entry.hooks.filter((h) => !isLegacyInjector(h.command));
      removedLegacy += entry.hooks.length - kept.length;
      return { ...entry, hooks: kept };
    })
    // An entry that held nothing but the legacy injector is now empty — prune it
    // rather than leaving a hook entry with no hooks in it.
    .filter((entry) => !Array.isArray(entry.hooks) || entry.hooks.length > 0);
}

// Quoted: an absolute home path can contain spaces (e.g. "C:/Users/John Doe/...").
const sessionStartCmd = `node "${contractHookAbs}" session`;
const userPromptSubmitCmd = `node "${contractHookAbs}" prompt`;
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

if (!hasCommand("UserPromptSubmit", userPromptSubmitCmd)) {
  settings.hooks.UserPromptSubmit.push({
    hooks: [{ type: "command", command: userPromptSubmitCmd, statusMessage: "Reinforcing tt-time-logging contract" }],
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
console.log(`Contract, stop-check, activity- and contract-hook scripts copied into ${destDir}.`);
if (removedLegacy > 0) {
  console.log(`Removed ${removedLegacy} legacy inline contract-injection hook(s).`);
}

// Append a one-line pointer into the global CLAUDE.md, mirroring the
// pattern other global skills already use there (e.g. the "graphify"
// entry). Only touches the file if it exists; only appends if the skill
// isn't already mentioned, so this stays safe to re-run.
const claudeMdPath = join(claudeHome, "CLAUDE.md");
if (existsSync(claudeMdPath)) {
  const claudeMd = readFileSync(claudeMdPath, "utf8");
  if (!claudeMd.includes("tt-time-logging")) {
    // Point at the copy the hooks actually read. The skill itself may have been
    // installed project-locally, in which case ~/.claude/skills/… doesn't exist.
    const pointer =
      "\n# tt-time-logging\n" +
      `- **tt-time-logging** (\`${toFwd(skillMdDest).replace(homedir().replace(/\\/g, "/"), "~")}\`) - time logging contract for the \`tt\` CLI: begin/touch/end phase marks, naming, summary/tag rules. Trigger: the operating card is injected every session and prompt via hooks; \`/tt-time-logging\` loads the full contract on demand.\n`;
    writeFileSync(claudeMdPath, claudeMd.replace(/\n*$/, "\n") + pointer);
    console.log(`Appended a tt-time-logging pointer to ${claudeMdPath}.`);
  } else {
    console.log(`${claudeMdPath} already mentions tt-time-logging — left untouched.`);
  }
} else {
  console.log(`No ${claudeMdPath} found — skipped the CLAUDE.md pointer.`);
}

console.log("Restart Claude Code or open /hooks once so the new settings file is picked up.");
