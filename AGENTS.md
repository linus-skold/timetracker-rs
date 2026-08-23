# Time logging contract

For any coding agent working in a repo where the operator tracks time with `tt`.
Tool-agnostic — this needs a shell and nothing else.

Everything here is `tt` itself. Run
`cargo install --git https://github.com/linus-skold/timetracker-rs` and the whole
workflow is on your `PATH`.

Also published as an installable skill — `npx skills add linus-skold/timetracker-rs`
pulls in `skills/tt-time-logging/SKILL.md`, which mirrors this file. Keep the two
in sync when either changes.

For Claude Code specifically, `skills/tt-time-logging/SKILL.md` documents a
one-time `install-hooks.mjs` step that wires this contract into
`SessionStart`/`Stop` hooks instead of relying on prose alone.

## Rules

1. **Only one writer.** If you orchestrate subagents, only the orchestrator logs.
   Subagents report back; the orchestrator records. This is not about safety —
   concurrent writes are serialized under an exclusive lock and no entry is lost —
   it is about the **unit**: if every worker logged its own turn, one issue would
   produce twenty rows instead of one and the rollup would stop meaning anything.
   When several subagents work the same project/issue/phase **concurrently**,
   their time still lands as one entry — see [Parallel subagents on one
   phase](#parallel-subagents-on-one-phase).
2. **The unit is a completed piece of work, not a commit.** Planning is work. So is
   a review that concludes "don't ship" and an investigation that produces no code.
   One entry per phase — several passes and a QA loop on one issue is one entry,
   not five.
3. **Never batch at day's end.** `tt log` back-dates from now, so entries written in
   a batch claim overlapping slots. Totals stay right; the timeline stops being
   true. Log when each phase finishes. `tt report` counts the overlaps so this stays
   visible rather than quietly rotting.

## Naming

- **project** — resolve in order: **`$TT_PROJECT`** if it is set, else the repo
  directory name `basename $(git rev-parse --show-toplevel)`, else ask. A directory
  name is a guess that is usually right; `$TT_PROJECT` is a statement, and the two
  diverge for clones renamed on disk, for multi-repo projects, and for monorepos
  whose directory is not what time is billed to. (If the operator uses a per-repo
  environment manager such as `mise` or `direnv`, `$TT_PROJECT` is a natural thing
  for them to declare there, so the value travels with the repo.)
- **issue** — the tracked issue number, or `-` if untracked
- **phase** — one of `plan` `impl` `qa` `review` `docs` `spike` `ops`; see
  [Which phase](#which-phase)

**project is a real field** on the entry, not a tag: the agent commands pass it as
`tt log --project <project>`, so it is stored explicitly rather than guessed. This
matters when reading rollups back — `tt report` groups on the field.

### Which phase

| Work | Phase |
|---|---|
| planning, breaking work down, writing a spec | `plan` |
| writing or changing code | `impl` |
| verifying behaviour, running or fixing tests | `qa` |
| reading code to judge it, whether or not it changes | `review` |
| documentation | `docs` |
| investigation that produces no artifact | `spike` |
| tooling, config, environment, release | `ops` |

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

tt agent list                                  # what is still open
tt agent cancel <project> <issue|-> <phase>    # drop without logging
tt agent audit [--auto-log]                    # unaccounted activity; see below

tt report [--week|--all|--since DATE [--until DATE]] [--project NAME] [--json]
```

Durations are rounded **up** to the nearest 5 minutes, never below 5 — a
ceiling, not nearest, so a logged span never reads shorter than what was
actually spent.

A mark's start time survives the agent's context being truncated or compacted, so do
not hold start times in context. Marks live in the application's own cache directory;
`TT_MARK_DIR` overrides it.

`touch` matters twice over: `end` measures start → last touch, not start → now, so
idle time after the work finished is not counted — and the heartbeats it appends are
what let a long phase log without a question. `end` refuses on a *silent gap*, never
on length, so a stretch between heartbeats over `TT_MAX_GAP_MINUTES`
or `agent.max_gap_minutes` (default 45) is what gets flagged.

A phase that produced **no heartbeat at all** is judged on its own threshold instead,
`TT_MAX_UNVOUCHED_MINUTES` or `agent.max_unvouched_minutes` (default 120). No beats is the absence of instrumentation —
a session that compacted, or `begin`/`end` without any `touch` — where a hole between
beats is positive evidence that work stopped, so the unmeasured phase gets the longer
allowance. Long enough is still refused: 120 minutes with nothing to show for it wants
a human.

## Working in parallel

Marks are keyed `project/issue/phase`, so any number can be open at once — several
issues in one repo, several repos, or both. They are independent: ending one never
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
a judge panel, a set of parallel reviewers, several independent finders — needs
a different close than the default begin → touch → end: a wall-clock span
across a parallel batch measures *elapsed* time, not the effort actually spent,
and would undercount it. Three subagents at 20 minutes each is 60 minutes of
work, not the 20 minutes the clock shows.

1. **Open the mark once, before the first dispatch.** `tt agent begin <project>
   <issue> <phase>` — if `tt agent list` already shows this exact key open,
   it's either your own earlier work on it (reuse it) or another session's
   (leave it; say what you found). This mark exists for visibility while the
   batch runs (`tt agent list`, the TUI's **Agents** panel), not for its own
   duration.
2. **Do not end it as each subagent returns.** Wait for every subagent
   dispatched in that batch to report back, however many rounds that takes.
3. **Sum each contributor's time.** Use each subagent's own elapsed wall-clock
   time for its dispatch — most agent tooling reports this on completion — not
   anything it says about itself in its own report. Add your own time too, if
   you did real work on the same phase yourself rather than only dispatching
   and reading results.
4. **Close it once, after the last one reports.** `tt agent cancel <project>
   <issue> <phase>` to drop the now-unwanted wall-clock mark, then
   `tt agent item <project> <issue> <phase> "<summary>" <summed minutes>` to
   log the real total as one entry.

A different phase — even on the same issue, even dispatched in the same
breath — is never folded into this. Give it its own `begin`/`item` pair; marks
are keyed `project/issue/phase` for exactly this reason.

Subagents dispatched one after another rather than concurrently don't need any
of this: a wall-clock span already accumulates sequential work correctly, so
the plain begin → touch → end flow is enough.

## When `end` refuses

- **exit 65, silent gap over threshold** — the heartbeats show a stretch with no sign
  of work in it, and the message names that stretch's length and clock interval, then
  quotes what `--full` and what `--trim` would log. Length alone is never the
  complaint: a long session that kept heartbeating logs silently. Which threshold
  applied depends on the evidence — 45 minutes between beats, 120 for a phase that
  never beat at all.

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
sat unaccounted for well past the normal warning threshold — see
`docs/decisions/0002-auto-logging-unaccounted-activity.md`. It is opt-in
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
itself: when set, `tt-stop-check.mjs`'s `tt agent activity check --auto-log`
call auto-logs the ending session's own unaccounted window instead of only
warning about it — same fixed phase/summary/tags, same idempotency. It
requires `agent.auto_log_after_minutes` to already be set (a config error
otherwise) — see `docs/decisions/0003-auto-log-on-stop.md`.

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

Its overlap counter is a health check on rule 3: if it climbs, logging has drifted
away from the moments it should be attached to.
