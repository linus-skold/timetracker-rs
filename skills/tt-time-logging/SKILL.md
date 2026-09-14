---
name: tt-time-logging
description: Time-logging contract for coding agents working in a repo tracked with the `tt` CLI (timetracker-rs) — when to log, how to name entries, and how to handle agent-phase marks (begin/touch/end).
---

# Time logging contract

**Before your first file-changing tool call, open a mark; close it when that
phase of work finishes** — `tt agent begin|touch|end <project> <issue|-> <phase>
["<summary>" on end]`. Project: `$TT_PROJECT`, else the repo directory name.
Phase: `plan|impl|qa|review|docs|spike|explore|ops`. Full contract: session-start
context or `/tt-time-logging`.

<!-- card:end — everything above is the per-prompt card; see scripts/tt-contract-hook.mjs -->

```sh
tt agent begin <project> <issue|-> <phase>
tt agent touch <project> <issue|-> <phase>                  # still working
tt agent end   <project> <issue|-> <phase> "<summary>"      # done — logs real elapsed time
```

`<project>` is `$TT_PROJECT`, else the repo directory name. `<issue>` is the
issue number, or `-`. `<phase>` is one of:

| Work | Phase |
|---|---|
| planning, breaking work down, writing a spec | `plan` |
| writing or changing code | `impl` |
| verifying behaviour, running or fixing tests | `qa` |
| reading code to judge it, whether or not it changes | `review` |
| documentation | `docs` |
| investigation that produces no artifact, answering a specific question | `spike` |
| open-ended exploration with no specific question | `explore` |
| tooling, config, environment, release | `ops` |

Everything below is detail on those three commands. Setup and installation live
in [README.md](README.md), not here.

## Rules

1. **Only one writer.** If you orchestrate subagents, only the orchestrator logs.
   Subagents report back; the orchestrator records. This is not about safety —
   concurrent writes are serialized under an exclusive lock and no entry is lost —
   it is about the **unit**: if every worker logged its own turn, one issue would
   produce twenty rows instead of one and the rollup would stop meaning anything.
   When several subagents work the same project/issue/phase **concurrently**,
   the orchestrator holds one mark per dispatch under `--agent <label>` and
   closes each on its own span — see [Parallel subagents on one
   phase](#parallel-subagents-on-one-phase).
2. **The unit is a completed piece of work, not a commit.** Planning is work. So is
   a review that concludes "don't ship" and an investigation that produces no code.
   One entry per phase — several passes and a QA loop on one issue is one entry,
   not five.
3. **Never batch at day's end.** `tt log` back-dates from now, so entries written in
   a batch claim overlapping slots. Totals stay right; the timeline stops being
   true. Log when each phase finishes. `tt report` counts the overlaps so this stays
   visible rather than quietly rotting. Two agents working one project in the same
   minute produce entries that overlap **by design**, so a climbing counter no
   longer implies drift on its own.

## Naming

- **project** — resolve in order: **`$TT_PROJECT`** if it is set, else the repo
  directory name `basename $(git rev-parse --show-toplevel)`, else ask. A directory
  name is a guess that is usually right; `$TT_PROJECT` is a statement, and the two
  diverge for clones renamed on disk, for multi-repo projects, and for monorepos
  whose directory is not what time is billed to. (If the operator uses a per-repo
  environment manager such as `mise` or `direnv`, `$TT_PROJECT` is a natural thing
  for them to declare there, so the value travels with the repo.)
- **issue** — the tracked issue number, or `-` if untracked
- **phase** — one of `plan` `impl` `qa` `review` `docs` `spike` `explore` `ops`; see the
  phase table at the top of this file

**project is a real field** on the entry, not a tag: the agent commands pass it as
`tt log --project <project>`, so it is stored explicitly rather than guessed. This
matters when reading rollups back — `tt report` groups on the field.

## Summary and tags

The **summary** is a short descriptor, **3-6 words**, plain prose. No issue number,
no phase word, no `#` anything — the fields and tags already carry all three, and a
`#` in prose becomes a junk tag (`tt` harvests every `#word`; the agent commands
strip leading ones, so "see #12" logs as "see 12", which reads oddly). Write what
the work *was*: `"store/links boundary"`, `"pane focus and cursor markers"`,
`"heartbeat gap threshold"`.

**Tags are deliberately sparse** — three, one per axis the fields cannot express:

| Tag | Axis |
|---|---|
| `#<project>/<issue>` | item — per-issue rollups inside a project; omitted when issue is `-` |
| `#<phase>` | phase |
| `#agent` | written by an agent, not by hand |

There is **no bare `#<project>` tag**. Project is a real field with its own axis; a
tag duplicating it only made every project appear twice in the tag list.

`#agent` comes only from `item` and `end`. A plain `tt log …` written by hand stays
unmarked, which is what makes the two distinguishable at all.

## Commands

`begin`/`touch`/`end` is the primary flow: open a mark when a piece of work
starts, touch it if it runs long, and close it when the work is done. `end`
measures the real elapsed span from the mark's own timestamps, so the logged
duration is what actually happened, never a guess.

```sh
# time a phase — the default flow
tt agent begin <project> <issue|-> <phase>
tt agent touch <project> <issue|-> <phase>     # work confirmed still happening
tt agent end   <project> <issue|-> <phase> "<summary>"

# fallback only: the duration is already known some other way (no mark to
# measure from — e.g. reporting someone else's already-finished work).
# Never a substitute for measuring a span you could have marked instead.
tt agent item  <project> <issue|-> <phase> "<summary>" <minutes>

# one in-flight subagent's own mark on a phase, for a parallel fan-out — see
# Parallel subagents on one phase. `begin`, `touch`, `end` and `cancel` take it.
tt agent begin <project> <issue|-> <phase> --agent <label>

# this session's id, from the per-prompt card: dismisses whatever the close
# leaves uncovered, so it stops showing up as unaccounted activity.
tt agent end   <project> <issue|-> <phase> "<summary>" <minutes> --session <id>

tt agent list [PROJECT]                        # what is still open
tt agent cancel <project> <issue|-> <phase>    # drop without logging
tt agent audit [--json] [--project NAME] [--auto-log]   # unaccounted activity; see below

# clear one unaccounted row — see Sweeping the unaccounted list
tt agent resolve --session <id> <start> <issue|-> <phase> "<summary>"
tt agent dismiss --session <id> <project> <start>-<end> ["<reason>"]

tt report [--week|--all|--since DATE [--until DATE]] [--project NAME] [--json]
```

Durations are logged as the actual minutes. Set `agent.round_minutes = N` in
the config file to round `end` and `item` up to the next N minutes instead,
never below N; `audit --auto-log` stays unrounded either way.

A mark's start time survives the agent's context being truncated or compacted, so do
not hold start times in context. Marks live in the application's own cache directory;
`TT_MARK_DIR` overrides it. The dismissal ledger sits beside them, with
`TT_DISMISSED_DIR` overriding that one.

`touch` matters twice over: `end` measures start → your last touch, not start → now,
so idle time after the work finished is not counted — and the heartbeats it appends
are what let a long phase log without a question. `end` refuses on a *silent gap*,
never on length, so a stretch between heartbeats over `TT_MAX_GAP_MINUTES`
or `agent.max_gap_minutes` (default 45) is what gets flagged.

A phase **you never touched** is judged on its own threshold instead,
`TT_MAX_UNVOUCHED_MINUTES` or `agent.max_unvouched_minutes` (default 120). No touch is
the absence of instrumentation — a session that compacted, or `begin`/`end` without
any `touch` — where a hole between heartbeats is positive evidence that work stopped,
so the unmeasured phase gets the longer allowance. Long enough is still refused: 120
minutes with nothing to show for it wants a human.

A phase you never touched is judged **whole-span**: one silence from `begin` to
where `end` measures, exactly as if the hooks had never beaten it. Once you have
touched, only the holes between *your* touches are judged, at the 45-minute
standard — so a phase touched at minute 0 and again at minute 60 is refused for
that hour however many hook beats fill it. Touch as the work runs, or pass the
real minutes.

Two kinds of heartbeat share one file, and only one of them vouches for time. **Your
`tt agent touch` is the vouch**: `end` measures to it, and it is what lifts a phase off
the unvouched threshold. **The hooks' beats prove the session is alive**, nothing more —
Claude Code's `UserPromptSubmit`, `SubagentStop` and `Stop` each beat the open marks of
the project the beating session resolved, and only that project's, at every turn
boundary. They keep a mark from expiring; they never move what `end` bills and never
make an untouched phase look touched. So `tt agent touch` is still yours to run when a
phase runs long inside a single turn.

A mark **expires** when it is not renewed: `max_gap_minutes` past its last
heartbeat from either source, or `max_unvouched_minutes` past `begin` if nothing beat
at all. An
expired mark stops vouching for its project, so that project's activity shows up
as unaccounted again, and `tt agent list` marks the row `[stale]` and prints
under it the exact `tt agent end` line that logs the work and clears it —
`--trim` when you touched the phase, explicit minutes when only the hooks beat it
or nothing did, since `--trim` on an untouched phase reads the whole span as one
gap and cuts nothing, logging all of it.

**Unaccounted activity is reconciled per session.** Two sessions working one
project in the same minute are two agents, so the audit prints a row for each
and the two sum; two rows can read the same for the same minute, which is the
sum and not a duplicate. A session whose `Stop` hook never wrote its end is
measured to now while its grace still has time left, and past that to its own
last evidence of life — its last subagent dispatch, or its start when there was
none — plus the same grace a mark gets. Only once that grace has run out does
its last row read `[abandoned]`, and the session then stops widening with every
audit. It reports no row at all while a mark opened alongside it covers that
whole bound; `[stale]` in `tt agent list` is the only nudge left in that case.

**An existing install must re-run `tt skill install`** (or `install-hooks.mjs`
directly). The hook scripts are copied into Claude Code's own hooks directory,
so a machine still holding the old copies gets no automatic beat at all, and
every mark then expires on the unvouched grace. Until it is re-run the
per-prompt card also carries no session id, so nothing can pass `--session`.

## Working in parallel

Marks are keyed `project/issue/phase`, plus an optional `--agent <label>`, so any
number can be open at once — several issues in one repo, several repos, several
subagents on one phase, or all of them. They are independent: ending one never
touches another.

If you run no subagents, rule 1 costs you nothing, since you are the only writer. Log
as each phase finishes rather than at the end of the session.

**Never end or cancel a mark you did not open.** `tt agent list` shows every open mark,
including other sessions'. One you did not open means work is in flight elsewhere, not
that something broke.

At session start you cannot tell a crashed session's leftover from a live sibling's
work — not even for your own project and issue. Say what you found and let the operator
decide.

The TUI shows the same open phases in its **Agents** panel, on `Shift-A`.

### Parallel subagents on one phase

Fanning several subagents out at once onto the same `project`/`issue`/`phase` —
a judge panel, a set of parallel reviewers, several independent finders — puts
work that ran side by side inside one wall-clock span, which measures *elapsed*
time rather than the effort spent. Three subagents at 20 minutes each is 60
minutes of work, not the 20 minutes the clock shows. So the unit is one mark per
in-flight subagent, keyed by a fourth `--agent <label>` segment, and every span
is measured rather than summed from what the tooling reported.

1. **Open one mark per dispatch, as you dispatch it.** `tt agent begin <project>
   <issue> <phase> --agent <label>`, with a label naming that subagent's job:
   `code-review`, `style-review`, `finder-2`. Two labels on one phase are two
   independent marks, and `tt agent list` and the TUI's **Agents** panel show
   one row each.
2. **Close each one as that subagent reports.** `tt agent end <project> <issue>
   <phase> --agent <label> "<summary>"` — its own summary, on its own measured
   span. A label addresses only its own mark, so a close never clears a
   sibling's. Pass what the report says about the run as custom data, under the
   `agent` namespace:

   ```sh
   tt agent end <project> <issue> <phase> --agent <label> "<summary>" \
     --data '{"task": "#175", "agent": {"model": "opus", "effort": "high", "tokens": {"input": 1200, "output": 300}}}'
   ```

   `task` names the unit of work inside the issue, such as the Task sub-issue
   the subagent worked on. Omit it, and any of `model`, `effort` and `tokens`,
   that the report does not give you rather than guessing a value. `tt` writes `agent.label` itself from
   `--agent`, so never pass that. The full schema is the "Well-known keys"
   table in `docs/usage.md`.
3. **Bill your own time under one held `--agent orchestrator` mark.**
   Dispatching, relaying and reading reports is real work on the same phase.
   Open that mark at the **first** dispatch and `touch` it at each later one —
   never open and close one around each dispatch — then close it once with
   **explicit minutes**, never `--full` and never `--trim`, because its wall
   clock spans the whole fan-out that the subagents' own spans already bill.
   Holding it open is also what keeps the relay span off the unaccounted list:
   an open mark covers its span in the audit by existing.
4. **A per-agent mark needs no `touch`.** The hooks beat every open mark of the
   project, tagged, so a mark never expires while its subagent runs and the beat
   never moves what `end` bills. An untouched span inside the 120-minute
   unvouched grace closes on what it measured.

No code reserves `orchestrator` or validates any label. It is a convention.

The label is no tag axis: an entry carries the same `#<project>/<issue>`,
`#<phase>` and `#agent` tags whatever label closed it, so `tt report` still sums
a whole fan-out under its one issue and phase. It is recorded as `agent.label`
in the entry's own data instead. Name the subagent in the summary prose too when
the row itself should say which one it was.

A different phase — even on the same issue, even dispatched in the same
breath — is never folded into this. Give it its own marks; a mark is keyed
`project/issue/phase` plus the label for exactly this reason.

Subagents dispatched one after another rather than concurrently need no labels:
one mark's wall-clock span already accumulates sequential work correctly, so
the plain begin → touch → end flow is enough.

## When `end` refuses

- **exit 65, silent gap over threshold** — the heartbeats show a stretch with no sign
  of work in it, and the message names that stretch's length and clock interval, then
  quotes what `--full` and what `--trim` would log. Length alone is never the
  complaint: a long session that kept heartbeating logs silently. Which threshold
  applied depends on the evidence — 45 minutes between beats, 120 for a phase you
  never touched.

  **Ask the operator about the named gap — never pick between `--full` and `--trim`
  yourself**, and reach for neither by reflex; only the person who was there knows
  whether the silence was work or a break. Then re-run with `--full` to accept the
  measured span, with `--trim` to log it minus **every** flagged gap, or pass the real
  minutes as a trailing argument — which wins over both flags. `--trim` never fires on
  its own and is never a default.

  `--full` records the flagged gaps on the entry (`tt log --idle=<start>-<end>`, one
  per gap), so the evidence survives rather than being discarded, and the intervals
  can be trimmed later from the TUI's detail popover with `[t]`. `--trim` is
  **destructive and unconfirmed**: it splits the entry into the pieces between the
  gaps there and then, so the silence is gone and what it reports back is what it
  stored — a smaller figure than the span it was given.
- **exit 64, no mark** — never marked, or the mark was lost. Use `item` with a
  duration you can justify, or ask. A missing summary is the same exit code.
- **exit 75, a close left unfinished** — an earlier close started and never finished, so
  it may already have recorded the entry. `begin` refuses on the same leftover. Read
  `tt report` first: if the entry is there, `tt agent cancel` the phase to clear the
  leftover; if it is not, cancel and then close the phase again. Never clear it blind.
- **exit 74, recorded but not cleared** — the entry **is** in the store and only the
  mark cleanup failed, an unwritable mark directory being the usual cause. **Do not
  retry the close**: a retry is exactly what would log the span twice. Say what
  happened, and `tt agent cancel` the phase once the directory is writable again.

## Auto-logged entries

`tt agent audit --auto-log` can write a fallback entry for a window that has
sat unaccounted for well past the normal warning threshold. It is opt-in
(`agent.auto_log_after_minutes`, unset by default — most operators will
never see one of these) and, when it does run, it never guesses:

- **Phase is always the literal `auto`**, summary always the literal
  `"unattended activity"` — never generated, never inferred from anything.
- **Tagged `#auto`, never `#agent`.** This was not an agent's self-report,
  so it does not carry that tag's meaning.

**If you find a `#auto` entry, do not reclassify it.** Guessing a real phase
for it after the fact is exactly what this mechanism exists to avoid —
leave it as `auto`, and if it matters, say so and let the operator decide
whether to split or re-tag it by hand.

`agent.auto_log_on_stop` extends the same mechanism to the `Stop` hook
itself: when set, `tt-activity-hook.mjs`'s `tt agent activity check --auto-log`
call auto-logs the ending session's own unaccounted window instead of only
warning about it — same fixed phase/summary/tags, same idempotency (a window
an `#auto` entry already covers is never logged twice). It requires
`agent.auto_log_after_minutes` to already be set: without it, `tt` warns and
ignores `auto_log_on_stop` rather than auto-logging anything. The hook's systemMessage
says "auto-logged" when this fired, and the plain unaccounted-activity
wording otherwise, so you can always tell which happened.

## Sweeping the unaccounted list

**If the `Stop` hook or an audit names unaccounted activity, sweep it** rather
than letting rows pile up until the list stops meaning anything. The list's
default state is **empty**:

```sh
tt agent audit --json --project <project>   # this project's rows, with their addresses
tt agent resolve --session <id> <start> <issue|-> <phase> "<summary>"   # it was work
tt agent dismiss --session <id> <project> <start>-<end> ["<reason>"]    # it was not
```

Put one proposed resolution per row to the owner — `resolve` for real work,
`dismiss` for a seam or a break — apply the approved ones, and repeat until
`tt agent audit --project <project>` prints `No unaccounted agent activity.`
This project's rows only, and each addressed by **its own session id from the
JSON**, so a concurrent agent's row is never touched. The one row you may act on
unasked is one you can attribute to a phase you just closed yourself — your own
relay tail.

`resolve` logs an entry spanning the row exactly, with the project taken from the
row and the phase you name, and it never rounds. A `--session`/`<start>` pair
matching no row exits 64 and writes nothing. The entry names the session it was
logged for, so a concurrent agent's row over the same minutes stays open for that
agent to resolve in turn and the two sum.

`dismiss` writes no entry, so a dismissed stretch appears in no `tt report`
total, and it records exactly the span you give it whether or not a row matches.

The warning threshold does not narrow the sweep: `--json` lists every row of the
project even when a session's remaining total is too small to be warned about, so
sweeping a session down to its last few minutes still leaves them addressable.

**A session id names an orchestrator; `--agent <label>` names a dispatch under
it.** Two orchestrators working one project are two sessions with distinct ids,
and their work sums. Subagents dispatched by one orchestrator share that
orchestrator's session id and differ only by label, so a dismissal is keyed by
session and never by mark. The per-prompt card names **this** session's id: pass
it as `--session <id>` on `end`, and on `resolve`/`dismiss` for this session's
own rows, taking the id from `tt agent audit --json` for any other row.

## Reading back

Use `tt report`, never parse `tt list` — that output is emoji-decorated text for
humans. `tt report --json` is the machine-readable form.

- With no scope it reports today; `--week` runs from Monday, `--all` is unbounded,
  `--since DATE` opens a range.
- `--until DATE` **narrows** one of those, and is a usage error on its own — there
  would be nothing to narrow but the single default day.
- Projects come from the **project field**, so `--project NAME` filters on what was
  stored rather than on a tag.

`tt report` is a pure read: it takes no lock and does not touch the store, so a
rollup never blocks a close that is happening at the same time.

Its overlap counter is a health check on rule 3, once the per-session overlap two
agents on one project produce by design is accounted for: a climb beyond that is
logging drifting away from the moments it should be attached to.
