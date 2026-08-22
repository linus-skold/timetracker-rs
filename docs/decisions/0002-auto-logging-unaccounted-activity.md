# 0002 — Auto-logging long-unaccounted activity

## Status

Proposed.

## Context

[0001](0001-agent-activity-tracking.md) added an independent activity ledger
and a reconciliation pass (`tt agent audit`, the `Stop` hook's immediate
warning, the TUI's Agents panel) that detects when real agent work happened
with no mark and no logged entry to show for it. All three surfaces are
**passive**: they tell the operator something is unaccounted for and leave
the decision to them.

That is a deliberate design choice in 0001 and it is right for a session
someone is watching. It does nothing for two cases 0001 did not try to
solve:

- **An unattended run.** A scheduled or long-running agent session with
  nobody at the TUI and no human reading Stop-hook warnings accumulates
  unaccounted windows that never get resolved, because resolving them was
  never anyone's job in that run.
- **A warning that gets ignored.** Even in a watched session, the operator
  may see the warning repeatedly and still not act on it — the gap stays
  open indefinitely, and `tt report` keeps under-reporting the same hours
  forever.

The underlying tension is the same one 0001 already worked through for
`begin`: something has to write an entry eventually, but a hook cannot judge
phase, and guessing wrong pollutes `tt report`'s rollups in a way that is
worse than an honest gap. 0001 resolved this for *detection* by never
guessing and only ever warning. This decision extends that resolution to
*logging*, on the same principle: still never guess the phase, but allow an
operator to opt in to a fallback write once a window has been unaccounted
for long enough that a human is unlikely to still resolve it by hand.

## Decision

An opt-in layer on top of 0001's reconciliation — not a replacement for it,
and not the model deciding to do this on its own.

### 1. A second, longer threshold

`agent.auto_log_after_minutes` (config), `TT_AUTO_LOG_AFTER_MINUTES` (env).
**Unset by default — the feature is off unless an operator turns it on.**
When set, it must be strictly greater than `agent.max_unvouched_minutes`
(0001's own floor): auto-logging something the audit hasn't even flagged
yet would skip past the warning stage entirely.

### 2. `tt agent audit --auto-log`

For every window `unaccounted()` (see `src/audit.rs`) returns that also
exceeds `auto_log_after_minutes`, log it the same way `tt agent item` does
— project field, rounded to the nearest quarter hour, floor 15 — with:

- **phase `auto`**, a literal, not a guess. Never `spike` — `spike` means a
  human or agent judged this to be investigation with no artifact; an
  auto-logged window was never judged at all, and folding it into `spike`'s
  rollup would misrepresent both.
- **summary fixed to `"unattended activity"`**, not generated — there is
  nothing here to summarize honestly beyond "something ran."
- **tags `#auto` and `#<phase>`, no `#agent`.** `#agent` means "written by
  an agent, not by hand" (0001, and the skill contract); this is written by
  neither an agent's judgment nor a human's hand, so it gets its own tag
  rather than borrowing one that would misrepresent its provenance.

Explicit, operator-run — not wired into any hook by default. See
Alternatives for why.

### 3. Reconciliation recognizes its own output

`audit::covered_by_entry` currently only treats a closed `#agent`-tagged
entry as covering a window. It must also treat a `#auto`-tagged entry as
covering, or the very entry `--auto-log` just wrote would still show up as
unaccounted on the next run — the mechanism would fight itself.

## Guardrails

- **Off by default.** Writing to the store without a human or an agent's
  judgment behind it is a bigger step than 0001's passive warning; it needs
  explicit opt-in, not a sane default.
- **The threshold ordering is enforced, not just documented.** A config
  where `auto_log_after_minutes <= max_unvouched_minutes` must not silently
  auto-log something the audit surfaces have never had a chance to warn
  about first.
- **Never a phase guess.** `auto` is the only phase this path ever writes.
  No heuristic (time of day, files touched, project history) gets to pick
  `impl` vs `qa` vs anything else on an agent's behalf.
- **Idempotent.** Running `--auto-log` twice over the same window must
  produce one entry, not two — the moment it logs a window, that window
  stops being unaccounted (guardrail 3 above), so a second run sees nothing
  left to do.

## Consequences

- `tt report` rollups can now include an honestly-labeled `#auto` bucket
  instead of silently missing hours forever, for operators who opt in.
- An `auto`-phase entry still needs a human to revisit it if they want real
  phase attribution — `tt`'s existing trim/edit story applies; this
  decision does not add a re-classification tool.
- No change to 0001's default behavior at all: with `auto_log_after_minutes`
  unset, `tt agent audit`, the `Stop` hook and the TUI panel behave exactly
  as they did before this decision existed.

## Alternatives considered

- **Wire `--auto-log` into the `Stop` hook directly**, so it runs
  automatically without the operator invoking it. Rejected for a first cut:
  a hook silently mutating the store on every session end is a much bigger
  leap of trust than a flag the operator explicitly chooses to run (by hand,
  or from their own cron / scheduled agent). Worth revisiting as a later,
  separately opted-in `agent.auto_log_on_stop`, once the core mechanism has
  been used long enough to trust unattended.
- **Guess the phase from a heuristic** (time of day, which files changed,
  project conventions). Rejected: no such signal is reliable enough, and a
  wrong guess corrupts `tt report`'s per-phase rollup in a way an honest
  `auto` bucket does not.
- **Fold auto-logged time into `spike`.** Rejected: `spike` already has a
  real meaning (judged investigation with no artifact); reusing it here
  would make it mean two different things depending on how an entry got
  created.
