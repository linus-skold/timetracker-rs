# 0003 — Wiring `--auto-log` into the `Stop` hook (`agent.auto_log_on_stop`)

## Status

Draft — for review. Not yet implemented. Addresses issue #21, which 0002's
Alternatives section deferred pending its own design pass.

## Context

0002 added `tt agent audit --auto-log`: an operator-run, opt-in fallback that
writes a fixed-phase `#auto` entry for any activity window that has sat
unaccounted past `agent.auto_log_after_minutes`. It deliberately stayed
operator-run — 0002's Alternatives section rejected wiring it into the `Stop`
hook directly, on the grounds that "a hook silently mutating the store on
every session end is a much bigger leap of trust than a flag the operator
explicitly chooses to run," and named three things to settle before
revisiting it:

1. `--auto-log` needs a track record of trustworthy entries.
2. Whether this is on by default once opted in, or needs a second
   confirmation gate.
3. Idempotency (0002 guardrail: running it twice writes one entry) needs to
   hold under a hook's stricter latency/failure constraints — a hook must
   never fail the harness event it's attached to (see `tt-stop-check.mjs`'s
   header comment and `activity_command`'s doc comment in `src/agent.rs`).

This doc proposes concrete answers to all three, and a shape to implement
against.

## Decision

### 1. Track record — defer to the operator, not a hardcoded date

Rather than the ADR itself declaring `--auto-log` "trusted now," gate
`auto_log_on_stop` so it is structurally impossible to reach without having
opted into `--auto-log`'s config first:

- `agent.auto_log_on_stop` **requires `agent.auto_log_after_minutes` to
  already be set.** If `auto_log_on_stop = true` but
  `auto_log_after_minutes` is unset, `tt` treats it as a config error at
  load time (loud, not a silent no-op) — the same "ordering enforced, not
  documented" guardrail 0002 used for the threshold pair.
- This makes "trust the mechanism first" a structural precondition instead
  of a subjective judgment call: an operator can only reach the hook-wired
  behavior by having already lived with the manual `--auto-log` flag turned
  on.

No calendar-based trust threshold (e.g. "N days since enabling") — that
state would have to be tracked somewhere and adds a moving part for no real
gain over the config-ordering constraint above.

### 2. On by default once opted in — no, single flag is the gate

No second confirmation gate. `agent.auto_log_on_stop = true` (plus the
precondition in #1) is sufficient by itself. Reasoning: 0002 already treats
"set `auto_log_after_minutes`" as the meaningful consent step for auto-logging
at all; `--auto-log` vs. `auto_log_on_stop` only changes *who* invokes the
already-consented-to write (operator's hand vs. the hook), not whether
writes happen. Layering a second gate on top of an already-opt-in boolean
mostly adds friction without adding safety — the real safety lever is
guardrail #3 (scope) below.

### 3. Idempotency and hook-latency safety — scope to the session, reuse `activity check`

Do **not** call the equivalent of full `tt agent audit --auto-log` from the
hook. That walks every session in the activity ledger, which is unbounded
work for a per-`Stop`-event hook and risks exactly the latency/failure
profile 0002 was worried about.

Instead, extend the machinery `tt-stop-check.mjs` already calls:
`tt agent activity check <session_id>` (`check_session` in `src/agent.rs`),
which is already scoped to one session and already silent/non-failing when
the session is unknown or has no resolved project. Add an `--auto-log` flag
to it:

- `tt agent activity check --auto-log <session_id>`: same reconciliation as
  today, narrowed to this one session's flagged windows; for any that
  exceed `auto_log_after_minutes`, write the `#auto` entry via the existing
  `write_auto_log` (unchanged — same fixed phase/summary/tags 0002 defined).
- Reuses `audit::covered_by_entry`'s existing `#auto` recognition for
  idempotency — a re-run (e.g. `SubagentStop` firing before the final
  `Stop`, or the hook firing twice) sees the window already covered and
  writes nothing, exactly as `--auto-log` already guarantees today. No new
  idempotency logic needed; the guarantee is inherited, not reimplemented.
- Bounded, single-session work — no risk of a `Stop` hook blocking on a
  large ledger scan.
- Failure mode matches the file's existing contract: any error (unreadable
  activity dir, storage load failure) is swallowed and logged nowhere
  louder than the hook's existing best-effort `try { } catch { }` — the
  hook already never fails the harness event, and this must not change
  that.

### 3a. `SubagentStop` vs `Stop`

These are different events: `SubagentStop` fires once per subagent as it
finishes; `Stop` fires once, when the orchestrator itself is done and
control returns to the user. Auto-logging is wired to `Stop` only — the
session-scoped `activity check --auto-log` call happens on the final stop,
not per subagent. Writing several small `#auto` entries within one session
(e.g. because the operator also runs it by hand mid-session, or a future
change fires it more often) is acceptable; 0002 already doesn't dedupe
across writes beyond "the covered window doesn't get logged twice."

A related, separate idea raised in review: today each `#auto` entry stands
alone with no link to the subagent dispatches inside its window (`Unaccounted.
subagents` is already counted, just not attributed). A follow-up could let an
orchestrator's `#auto` entry carry its subagents' detail as a sub-list,
collapsible in the TUI/details pane, rather than flattening everything into
one summary line. Out of scope for this doc — tracked separately (see
issues below) since it's a reporting/TUI concern, not a hook-safety one.

### 3b. Idle time must be subtracted before a window is auto-logged

`agent.max_gap_minutes` already exists and governs interior heartbeat
silence for **mark-based** closes (`tt agent end`, via `marks::gaps_over`
and `IdleInterval` — see `src/agent.rs` around `max_gap_minutes()`). But
`audit::unaccounted` — the function both `--auto-log` and this hook path
build on — does not use it at all: an `Unaccounted` window is just
`[session.start, session.end]` with no idle subtracted, so a session that
was mostly idle in the middle would still get its entire raw span logged
under `#auto`.

This matters more once `Stop`-triggered writes remove the human from the
loop entirely: today, an operator running `--auto-log` by hand can eyeball
`describe()`'s printed span before trusting it; a hook cannot. Before wiring
`auto_log_on_stop`, `write_auto_log` (or `unaccounted` itself) should
subtract idle intervals over `max_gap_minutes` from the logged span, the
same way `split_at_idle` already does for mark-based closes — so an
auto-logged entry reports only genuinely-active time, not wall-clock time
including idle. This is really an `--auto-log` correctness fix, not
specific to the `Stop`-hook wiring, but it becomes load-bearing once a human
is no longer there to notice an inflated window before it lands. Tracked as
its own issue (see below) since it can and should land independently,
before `auto_log_on_stop`.

### 4. Hook wiring

`tt-stop-check.mjs`, after its existing `activity check` warning: if that
call's config-derived behavior includes auto-logging (i.e. the flag is
simply always passed — `tt` itself decides whether to act on it based on
config, so the script doesn't need to know the operator's config), replace
the plain `check` call with `check --auto-log` and adjust the message:
distinguish "flagged, unaccounted for" (today's wording) from "flagged and
auto-logged" (new — report what got written, not just what's missing, so
the operator isn't surprised by entries appearing they didn't type).

`install-hooks.mjs` needs no changes — it installs the same hook script
either way; the new behavior is entirely config-gated inside `tt` itself,
consistent with 0002's precedent of `--auto-log` being "accepted but logs
nothing" when unconfigured.

### 5. Config

```rust
pub struct AgentConfig {
    pub max_gap_minutes: Option<i64>,
    pub max_unvouched_minutes: Option<i64>,
    pub auto_log_after_minutes: Option<i64>,
    /// Have the `Stop` hook's `tt agent activity check` call auto-log this
    /// session's own unaccounted window, the same way `--auto-log` would.
    /// Requires `auto_log_after_minutes` to be set — see
    /// docs/decisions/0003-auto-log-on-stop.md.
    pub auto_log_on_stop: Option<bool>,
}
```

Validated once, at the same point config-ordering (`auto_log_after_minutes >
max_unvouched_minutes`) is presumably already checked today — loud error,
not a silent downgrade to no-op, so a misconfigured operator finds out from
`tt`, not from missing hours weeks later.

## Guardrails (inherited + new)

- Off by default (inherited from 0002 — both thresholds unset).
- Structural precondition: `auto_log_on_stop` without `auto_log_after_minutes`
  is a config error, not a silent no-op.
- Session-scoped only — never a full-ledger scan from a hook.
- Idempotent by construction — same `#auto`-covers-window mechanism as
  `--auto-log`, not a parallel implementation.
- Hook never fails the harness event — errors swallowed exactly as
  `tt-stop-check.mjs` and `activity_command` already do today.
- Still never a phase guess — `write_auto_log` is unchanged.

## Consequences

- An unattended session that nobody watches, and that never gets a manual
  `tt agent audit --auto-log`, still gets its own trailing window logged
  honestly under `#auto` the moment it stops — closing the gap 0002's
  Alternatives section identified as the whole motivation for #21.
- Two opt-in booleans now gate write-on-stop instead of one, but the second
  (`auto_log_on_stop`) is meaningless without the first already being live,
  so there's effectively one on-ramp: start with `--auto-log` by hand, then
  flip the hook on once that's been lived with.
- `AGENTS.md` / `skills/tt-time-logging/SKILL.md`'s "Auto-logged entries"
  section needs a paragraph on `auto_log_on_stop`, and
  `install-hooks.mjs`'s "Enforcing this in Claude Code" section needs a
  note that the Stop hook may now write entries, not just warn — both
  documentation-only changes alongside the code.

## Alternatives considered

- **Call full `tt agent audit --auto-log` from the hook.** Rejected: unbounded
  ledger scan on every `Stop` event is the latency/failure risk 0002 flagged;
  session-scoped `activity check --auto-log` gets the same outcome for the
  one session that just ended, cheaply.
- **A second confirmation gate** (e.g. `auto_log_on_stop` needs an explicit
  `"i-understand"` string rather than a bool). Rejected as friction without
  a real safety gain over the existing config-ordering constraint — see
  decision #2.
- **Time-based trust gate** (e.g. don't honor `auto_log_on_stop` until
  `auto_log_after_minutes` has been set for N days). Rejected: requires
  tracking "when was this config value first set" somewhere, a new piece of
  state for a benefit the structural precondition in decision #1 already
  covers.
