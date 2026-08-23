#!/usr/bin/env node
// SessionStart/UserPromptSubmit hook: injects the time-logging contract as
// additionalContext. Installed by install-hooks.mjs.
//
// Usage: node tt-contract-hook.mjs <session|prompt>
//
//   session - the whole contract, once, at session start.
//   prompt  - only the operating card (everything above the `card:end` marker
//             in SKILL.md), on every prompt.
//
// Why the split: UserPromptSubmit fires on every turn, and re-injecting 15KB
// of unchanged reference text each time trains a "seen it, skip it" response —
// the reader learns the block is boilerplate, which is exactly when the one
// actionable line in it stops landing. The card is small enough to actually
// re-read, and it's the part with commands in it.
//
// The card is not a second file. It's the head of SKILL.md, cut at the marker,
// so there is nothing to keep in sync: editing the document's opening is
// editing the card.
//
// SKILL.md is read from this script's own directory — install-hooks.mjs copies
// both into ~/.claude/hooks/tt-time-logging/ so they travel together.
//
// A hook must never fail the harness event it's attached to: any error here
// emits an empty result and exits 0.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const CARD_MARKER = "<!-- card:end";

const EVENTS = {
  session: "SessionStart",
  prompt: "UserPromptSubmit",
};

function emit(hookEventName, additionalContext) {
  process.stdout.write(
    additionalContext
      ? JSON.stringify({ hookSpecificOutput: { hookEventName, additionalContext } })
      : "{}",
  );
  process.exit(0);
}

const mode = process.argv[2];
const hookEventName = EVENTS[mode];
if (!hookEventName) emit(null, null);

let contract;
try {
  const scriptDir = dirname(fileURLToPath(import.meta.url));
  contract = readFileSync(join(scriptDir, "SKILL.md"), "utf8");
} catch {
  emit(hookEventName, null);
}

if (mode === "session") emit(hookEventName, contract);

// Card: the head of the document, minus the skill frontmatter. A missing
// marker means someone edited SKILL.md without keeping one — fall back to the
// whole file rather than injecting nothing.
const cut = contract.indexOf(CARD_MARKER);
const head = cut === -1 ? contract : contract.slice(0, cut);
emit(hookEventName, head.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "").trim());
