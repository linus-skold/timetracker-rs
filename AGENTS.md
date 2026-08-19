# Time logging contract

For any coding agent working in a repo where the operator tracks time with `tt`.
Tool-agnostic — this needs a shell and nothing else.

Everything here is `tt` itself. Run
`cargo install --git https://github.com/linus-skold/timetracker-rs` and the whole
workflow is on your `PATH`.

## Rules

1. **Only one writer.** If you orchestrate subagents, only the orchestrator logs.
   Subagents report back; the orchestrator records. This is not about safety —
   concurrent writes are serialized under an exclusive lock and no entry is lost —
   it is about the **unit**: if every worker logged its own turn, one issue would
   produce twenty rows instead of one and the rollup would stop meaning anything.
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

```sh
# time a phase
tt agent begin <project> <issue|-> <phase>
tt agent touch <project> <issue|-> <phase>     # work confirmed still happening
tt agent end   <project> <issue|-> <phase> "<summary>"

# or log a known duration outright
tt agent item  <project> <issue|-> <phase> "<summary>" <minutes>

tt agent list                                  # what is still open
tt agent cancel <project> <issue|-> <phase>    # drop without logging

tt report [--week|--all|--since DATE [--until DATE]] [--project NAME] [--json]
```

Durations are rounded to 15 minutes, floor 15.

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
