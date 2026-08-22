#!/usr/bin/env node
// Stop hook: warns (never blocks) about two things when the agent stops —
// installed by install-hooks.mjs.
//
//   1. tt agent marks still open for this project — a forgotten
//      `tt agent end` doesn't silently lose the phase.
//   2. this session's own activity window being unaccounted for — see
//      docs/decisions/0001-agent-activity-tracking.md and
//      `tt agent activity check`. Catches the gap immediately rather than
//      waiting for the next `tt agent audit`.

import { execSync, execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

function projectName() {
  try {
    const root = execSync("git rev-parse --show-toplevel", { encoding: "utf8" }).trim();
    return root.split(/[\\/]/).pop();
  } catch {
    return null;
  }
}

function sessionId() {
  try {
    return JSON.parse(readFileSync(0, "utf8")).session_id;
  } catch {
    return undefined;
  }
}

const messages = [];

const proj = projectName();
if (proj) {
  let list = "";
  try {
    list = execSync("tt agent list", { encoding: "utf8" });
  } catch (e) {
    list = e.stdout?.toString() ?? "";
  }

  const openLines = list.split("\n").filter((line) => line.includes(proj));
  if (openLines.length > 0) {
    messages.push(
      `tt-time-logging: open mark(s) for '${proj}' are still unclosed. ` +
        `Close with 'tt agent end <project> <issue> <phase> "<summary>"' ` +
        `(or 'tt agent cancel' if it shouldn't be logged) before stopping.\n` +
        openLines.join("\n"),
    );
  }
}

const session = sessionId();
if (session) {
  let checked = "";
  try {
    checked = execFileSync("tt", ["agent", "activity", "check", session], {
      encoding: "utf8",
    });
  } catch {
    checked = "";
  }
  if (checked.trim()) {
    messages.push(
      `tt-time-logging: this session's activity is unaccounted for — no open mark ` +
        `or logged #agent entry covers it:\n${checked.trim()}`,
    );
  }
}

process.stdout.write(
  messages.length > 0 ? JSON.stringify({ systemMessage: messages.join("\n\n") }) : "{}",
);
