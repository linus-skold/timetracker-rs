//! `tt agent item` and `tt agent end`: a one-shot log, and closing an open mark on
//! the span derived from its timestamps and heartbeats.
//!
//! Observables are `commands::log`'s stdout line and the sandbox's `data.json`; every
//! expectation is derived from the fixture, never written down. Sandbox: [`common`].

mod common;

#[cfg(unix)]
use common::Mode;
use common::{Case, StoreRow, clock, logged_duration, now};
// Only `an_entry_recorded_with_the_mark_left_behind_exits_74` uses this, and it
// is unix-only — ungated, this import is dead code on Windows.
#[cfg(unix)]
use std::fs;

// --- item ------------------------------------------------------------------

#[test]
fn item_logs_the_three_convention_tags() {
    let case = Case::new("item-tags");
    let run = case.run(&[
        "item",
        "loremind",
        "77",
        "impl",
        "store/links boundary",
        "43",
    ]);
    run.assert_status(0);

    let store = case.store();
    assert_eq!(store.entries.len(), 1, "one entry");
    let entry = &store.entries[0];
    assert_eq!(entry.description, "store/links boundary");
    assert_eq!(entry.project.as_deref(), Some("loremind"));
    assert_eq!(entry.tags, ["loremind/77", "impl", "agent"]);
}

#[test]
fn item_drops_the_item_tag_for_the_sentinel_issue() {
    let case = Case::new("item-sentinel");
    case.run(&["item", "loremind", "-", "plan", "sketched the shape", "20"])
        .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(entry.tags, ["plan", "agent"]);
    // The project is a real field with its own axis — never a tag.
    assert!(
        !entry.tags.iter().any(|tag| tag == "loremind"),
        "a bare project tag was emitted: {:?}",
        entry.tags
    );
}

#[test]
fn item_logs_the_minutes_it_was_given() {
    let case = Case::new("item-actual");
    let run = case.run(&["item", "proj", "7", "impl", "did the thing", "43"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(43));

    let short = Case::new("item-actual-short");
    let run = short.run(&["item", "proj", "7", "impl", "a quick errand", "2"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(2));
}

#[test]
fn round_minutes_rounds_an_item_up_to_the_next_step() {
    let case = Case::new("item-rounding");
    case.write_config("[agent]\nround_minutes = 5\n");
    let run = case.run(&["item", "proj", "7", "impl", "did the thing", "43"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(common::round_to(43, 5)));

    let floored = Case::new("item-rounding-floor");
    floored.write_config("[agent]\nround_minutes = 5\n");
    let run = floored.run(&["item", "proj", "7", "impl", "a quick errand", "2"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(5));
}

#[test]
fn round_minutes_rounds_a_measured_close_up_to_the_next_step() {
    let case = Case::new("end-rounding");
    case.write_config("[agent]\nround_minutes = 15\n");
    let start = now() - 43 * 60;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[now()]);

    let run = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    run.assert_status(0);
    assert_eq!(run.logged_minutes(), common::round_to(43, 15));
}

/// A summary that merely mentions an issue number does not become a tag.
#[test]
fn item_strips_a_stray_hash_from_the_summary() {
    let case = Case::new("item-stray-hash");
    case.run(&[
        "item",
        "proj",
        "7",
        "impl",
        "closed #12 with the C# bridge",
        "30",
    ])
    .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(entry.description, "closed 12 with the C# bridge");
    assert_eq!(entry.tags, ["proj/7", "impl", "agent"]);
}

#[test]
fn item_creates_no_mark_file() {
    let case = Case::new("item-no-mark");
    case.run(&["item", "proj", "7", "impl", "did the thing", "30"])
        .assert_status(0);
    assert_eq!(case.mark_count(), 0, "no mark was written");
    assert!(!case.beats_file("proj.7.impl").exists());
}

/// A plain error message and exit 64, not clap's 2 and its usage block.
#[test]
fn item_with_non_numeric_minutes_exits_64() {
    let case = Case::new("item-bad-minutes");
    let run = case.run(&["item", "proj", "7", "impl", "did the thing", "half an hour"]);
    run.assert_status(64);
    run.assert_stderr_has("minutes must be a whole number, got 'half an hour'");
    // The store already exists by then, so the assertion is on the entries.
    assert!(
        case.store().entries.is_empty(),
        "a rejected item logged something"
    );
}

#[test]
fn item_without_minutes_exits_64() {
    let case = Case::new("item-no-minutes");
    let run = case.run(&["item", "proj", "7", "impl", "did the thing"]);
    run.assert_status(64);
    run.assert_stderr_has("usage: tt agent item");
}

#[test]
fn item_leaves_an_open_mark_alone() {
    let case = Case::new("item-leaves-marks");
    case.write_mark("proj.7.impl", now() - 600);
    case.run(&["item", "proj", "7", "impl", "something else entirely", "30"])
        .assert_status(0);
    assert!(case.mark_file("proj.7.impl").is_file(), "the mark survived");
}

// --- end -------------------------------------------------------------------

#[test]
fn end_derives_minutes_from_the_mark() {
    let case = Case::new("end-derives");
    let elapsed = 1800;
    case.write_mark("proj.7.impl", now() - elapsed);

    let run = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    run.assert_status(0);
    run.assert_stdout_has(&format!(
        "\"did the thing\" (proj) [#proj/7 #impl #agent] {}",
        logged_duration(elapsed / 60)
    ));
    assert!(!case.mark_file("proj.7.impl").exists());
}

#[test]
fn a_successful_end_clears_the_mark_and_its_beats() {
    let case = Case::new("end-clears");
    case.write_mark("proj.7.impl", now());
    case.run(&["touch", "proj", "7", "impl"]).assert_status(0);
    assert!(case.beats_file("proj.7.impl").is_file());

    case.run(&["end", "proj", "7", "impl", "did the thing"])
        .assert_status(0);
    assert!(!case.mark_file("proj.7.impl").exists());
    assert!(!case.beats_file("proj.7.impl").exists());
}

#[test]
fn an_explicit_trailing_minutes_argument_overrides_the_mark() {
    let case = Case::new("end-explicit");
    case.write_mark("proj.7.impl", now() - 1800);

    let run = case.run(&["end", "proj", "7", "impl", "did the thing", "90"]);
    run.assert_status(0);
    // 90, not the mark's 30: the argument skips the timestamps entirely.
    run.assert_stdout_has(&logged_duration(90));
}

#[test]
fn end_without_a_mark_exits_64() {
    let case = Case::new("end-unmarked");
    let run = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    run.assert_status(64);
    run.assert_stderr_has("no mark for proj/7 impl");
    assert!(case.store().entries.is_empty(), "nothing was logged");
}

/// 64 and not clap's 2, so `summary` must stay an `Option<String>` checked by hand.
#[test]
fn end_without_a_summary_exits_64() {
    let case = Case::new("end-no-summary");
    case.write_mark("proj.7.impl", now());

    let run = case.run(&["end", "proj", "7", "impl"]);
    run.assert_status(64);
    run.assert_stderr_has("need a summary");
    assert!(case.store().entries.is_empty(), "nothing was logged");
    assert!(case.mark_file("proj.7.impl").is_file(), "the mark survived");
}

// --- gaps ------------------------------------------------------------------

#[test]
fn steady_beats_log_the_full_span_however_long_it_ran() {
    let case = Case::new("gaps-steady");
    let step = 600;
    let start = now() - step * 24;
    let beats: Vec<i64> = (1..=24).map(|i| start + i * step).collect();
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &beats);

    let run = case.run(&["end", "proj", "7", "impl", "long active session"]);
    run.assert_status(0);
    // Measured to the last beat, which the fixture puts exactly at the span's end.
    let last_beat = *beats.last().unwrap();
    run.assert_stdout_has(&logged_duration((last_beat - start) / 60));
}

/// A 110-minute phase with an 80-minute hole, shared so no case restates a number.
struct GapFixture {
    hole: (i64, i64),
    /// The measured span.
    minutes: i64,
    /// The hole.
    hole_minutes: i64,
    /// The last heartbeat, where `end` measures to and so where the entry must end.
    last_beat: i64,
}

fn gap_fixture(case: &Case) -> GapFixture {
    let start = now() - 110 * 60;
    let beats = [start + 10 * 60, start + 30 * 60, start + 110 * 60];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);
    GapFixture {
        hole: (beats[1], beats[2]),
        minutes: (beats[2] - start) / 60,
        hole_minutes: (beats[2] - beats[1]) / 60,
        last_beat: beats[2],
    }
}

#[test]
fn a_single_over_threshold_hole_is_refused_and_named() {
    let case = Case::new("gaps-refused");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!(
        "proj/12 plan has an {}m gap",
        fixture.hole_minutes
    ));
    run.assert_stderr_has(&format!(
        "({}-{})",
        clock(fixture.hole.0),
        clock(fixture.hole.1)
    ));
    // `--trim`'s figure is derived from the fixture, not restated.
    run.assert_stderr_has(&format!(
        "--full logs {}m, --trim logs {}m",
        fixture.minutes,
        fixture.minutes - fixture.hole_minutes
    ));
    run.assert_stderr_has("or pass the real minutes instead.");
    assert!(case.store().entries.is_empty(), "nothing was logged");
    // A refusal leaves the mark and its beats intact so the phase can still close.
    assert!(case.mark_file("proj.12.plan").is_file());
    assert!(case.beats_file("proj.12.plan").is_file());
}

/// 70 minutes takes `a`, where the shared fixture's 80 takes `an`.
#[test]
fn the_refusal_names_the_gap_with_the_right_article() {
    let case = Case::new("gaps-article");
    let start = now() - 100 * 60;
    let beats = [start + 10 * 60, start + 80 * 60, start + 100 * 60];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing"]);
    run.assert_status(65);
    // The whole line, so the shape agents read stays pinned as well as the article.
    run.assert_stderr_has(&format!(
        "tt: proj/12 plan has a 70m gap ({}-{})",
        clock(beats[0]),
        clock(beats[1])
    ));
}

#[test]
fn full_accepts_a_flagged_span() {
    let case = Case::new("gaps-full");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(fixture.minutes));
    assert!(!case.mark_file("proj.12.plan").exists());
    assert!(!case.beats_file("proj.12.plan").exists());
}

#[test]
fn explicit_minutes_still_win_over_a_flagged_span() {
    let case = Case::new("gaps-explicit");
    gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "30"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(30));
}

#[test]
fn silence_before_the_first_beat_is_a_gap_too() {
    let case = Case::new("gaps-leading");
    let start = now() - 120 * 60;
    let first = start + 110 * 60;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[first, first + 300]);

    let run = case.run(&["end", "proj", "7", "impl", "late first beat"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!("gap ({}-{})", clock(start), clock(first)));
}

#[test]
fn the_threshold_is_configurable() {
    let case = Case::new("gaps-threshold");
    let span = 30 * 60;
    let start = now() - span;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[start + span]);

    // Passed explicitly, since the harness clears the environment.
    let run = case.run_with_env(
        &[
            "end",
            "proj",
            "7",
            "impl",
            "half an hour, no beats inside it",
        ],
        &[("TT_MAX_GAP_MINUTES", "10")],
    );
    run.assert_status(65);
    run.assert_stderr_has(&format!("{}m gap", span / 60));
}

/// An unvouched phase — no heartbeats at all — is judged as one silence across its
/// whole span, against its own longer threshold, so 90 minutes of it still logs.
#[test]
fn an_unvouched_span_under_the_unvouched_threshold_logs() {
    let case = Case::new("gaps-unvouched-under");
    let span = 90;
    case.write_mark("proj.7.impl", now() - span * 60);

    let run = case.run(&["end", "proj", "7", "impl", "no evidence either way"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(span));
}

/// The other side of that threshold: refused, naming the whole span as the gap.
#[test]
fn an_unvouched_span_over_the_unvouched_threshold_is_flagged() {
    let case = Case::new("gaps-unvouched-over");
    let start = now() - 150 * 60;
    case.write_mark("proj.7.impl", start);

    let run = case.run(&["end", "proj", "7", "impl", "no evidence either way"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!("gap ({}-", clock(start)));
    assert!(case.store().entries.is_empty(), "nothing was logged");
}

// --- automatic beats -------------------------------------------------------
//
// No automatic beat may shorten what `end` bills, or move a phase off the
// unvouched threshold.

/// The same 90-minute span `an_unvouched_span_under_the_unvouched_threshold_logs`
/// bills, with hook beats all through it.
#[test]
fn hook_beats_alone_bill_the_same_span_on_the_same_threshold() {
    let case = Case::new("gaps-hook-only");
    let span = 90;
    let start = now() - span * 60;
    case.write_mark("proj.7.impl", start);
    case.hook_beats_at("proj.7.impl", &[start + 20 * 60, start + 50 * 60]);

    let run = case.run(&["end", "proj", "7", "impl", "hooks beat, the model did not"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(span));
}

/// A hook beat after the model's last touch must not discard that touch: the
/// bill is the touch's, not the whole span to now.
#[test]
fn a_hook_beat_after_a_touch_does_not_discard_the_touchs_anchor() {
    let case = Case::new("gaps-hook-after-touch");
    let start = now() - 61 * 60;
    case.write_mark("proj.7.impl", start);
    let file = case.beats_file("proj.7.impl");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        format!("{}\n{} hook\n", start + 20 * 60, start + 21 * 60),
    )
    .unwrap();

    let run = case.run(&[
        "end",
        "proj",
        "7",
        "impl",
        "closing work after the last beat",
    ]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(20));
}

/// An abandoned mark whose only beats are automatic is judged across its whole
/// span, so the refusal names the mark's own start and `--trim` has nothing
/// left to bill: the printed close line asks for the minutes instead.
#[test]
fn an_abandoned_hook_beaten_mark_is_refused_across_its_whole_span() {
    let case = Case::new("gaps-hook-abandoned");
    let start = now() - 200 * 60;
    case.write_mark("proj.7.impl", start);
    case.hook_beats_at("proj.7.impl", &[start + 10 * 60, start + 40 * 60]);

    let refused = case.run(&["end", "proj", "7", "impl", "left open over the weekend"]);
    refused.assert_status(65);
    refused.assert_stderr_has(&format!("gap ({}-", clock(start)));
    assert!(case.store().entries.is_empty(), "nothing was logged");
}

// --- an unvouched phase is judged as if its beats file were empty ----------

/// Hook beats at either end of the span buy no interior allowance: 125 minutes
/// is one silence over the unvouched grace.
#[test]
fn hook_beats_do_not_bracket_an_over_grace_unvouched_span() {
    let case = Case::new("gaps-hook-bracketed");
    let span = 125;
    let start = now() - span * 60;
    case.write_mark("proj.7.impl", start);
    case.hook_beats_at("proj.7.impl", &[start + 5 * 60, start + (span - 2) * 60]);

    let run = case.run(&["end", "proj", "7", "impl", "hooks bracketed the idle"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!("gap ({}-", clock(start)));
    assert!(case.store().entries.is_empty(), "nothing was logged");
}

/// Nor does one cost the grace: 100 minutes still bills whole.
#[test]
fn a_hook_beat_does_not_shorten_an_under_grace_unvouched_span() {
    let case = Case::new("gaps-hook-under-grace");
    let span = 100;
    let start = now() - span * 60;
    case.write_mark("proj.7.impl", start);
    case.hook_beats_at("proj.7.impl", &[start + 5 * 60]);

    let run = case.run(&["end", "proj", "7", "impl", "one hook beat, then work"]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(span));
}

/// The rule itself: the beats file's hook lines change neither the bill nor the
/// refusal, on either side of the grace.
#[test]
fn an_unvouched_phase_is_judged_the_same_with_hook_beats_as_with_none() {
    for span in [90, 150] {
        let start = now() - span * 60;

        let empty = Case::new(&format!("gaps-unvouched-empty-{span}"));
        empty.write_mark("proj.7.impl", start);
        let without = empty.run(&["end", "proj", "7", "impl", "no evidence either way"]);

        let hooked = Case::new(&format!("gaps-unvouched-hooked-{span}"));
        hooked.write_mark("proj.7.impl", start);
        hooked.hook_beats_at("proj.7.impl", &[start + 5 * 60, start + (span - 2) * 60]);
        let with = hooked.run(&["end", "proj", "7", "impl", "no evidence either way"]);

        assert_eq!(with.status, without.status, "{span}m: exit code");
        assert_eq!(with.stdout, without.stdout, "{span}m: what was billed");
    }
}

/// However many hook beats follow the model's last touch, the bill is the
/// touch's.
#[test]
fn hook_beats_after_a_touch_never_move_the_bill() {
    let case = Case::new("gaps-hook-after-touch-many");
    let start = now() - 61 * 60;
    case.write_mark("proj.7.impl", start);
    let file = case.beats_file("proj.7.impl");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut body = format!("{}\n", start + 20 * 60);
    for minute in 21..=60 {
        body.push_str(&format!("{} hook\n", start + minute * 60));
    }
    std::fs::write(&file, body).unwrap();

    let run = case.run(&[
        "end",
        "proj",
        "7",
        "impl",
        "touched once, hooks all through",
    ]);
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(20));
}

/// Beating once does not buy the longer allowance: 46 minutes, one over.
#[test]
fn a_beaten_phase_is_still_judged_at_the_interior_threshold() {
    let case = Case::new("gaps-beaten-boundary");
    let start = now() - 60 * 60;
    let beats = [start + 5 * 60, start + 51 * 60, start + 60 * 60];
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &beats);

    let run = case.run(&["end", "proj", "7", "impl", "beat, then went quiet"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!(
        "has a 46m gap ({}-{})",
        clock(beats[0]),
        clock(beats[1])
    ));
}

/// Both thresholds are settable in `config.toml`, and the environment still wins.
#[test]
fn the_thresholds_come_from_the_config_file_unless_the_environment_says_otherwise() {
    let configured = Case::new("threshold-config");
    configured.write_config("[agent]\nmax_unvouched_minutes = 30\n");
    configured.write_mark("proj.7.impl", now() - 60 * 60);
    let run = configured.run(&["end", "proj", "7", "impl", "an hour of nothing"]);
    run.assert_status(65);
    run.assert_stderr_has("60m gap");

    let overridden = Case::new("threshold-config-env");
    overridden.write_config("[agent]\nmax_unvouched_minutes = 30\n");
    overridden.write_mark("proj.7.impl", now() - 60 * 60);
    let run = overridden.run_with_env(
        &["end", "proj", "7", "impl", "an hour of nothing"],
        &[("TT_MAX_UNVOUCHED_MINUTES", "90")],
    );
    run.assert_status(0);
    run.assert_stdout_has(&logged_duration(60));
}

/// A beaten phase reads its threshold from the same file.
#[test]
fn the_gap_threshold_comes_from_the_config_file_too() {
    let case = Case::new("gap-threshold-config");
    case.write_config("[agent]\nmax_gap_minutes = 10\n");
    let start = now() - 40 * 60;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[start + 5 * 60, start + 25 * 60]);

    let run = case.run(&["end", "proj", "7", "impl", "beat, then quiet"]);
    run.assert_status(65);
    run.assert_stderr_has("20m gap");
}

#[test]
fn the_unvouched_threshold_is_configurable() {
    let case = Case::new("gaps-unvouched-threshold");
    let span = 30;
    case.write_mark("proj.7.impl", now() - span * 60);

    // Passed explicitly, since the harness clears the environment.
    let run = case.run_with_env(
        &["end", "proj", "7", "impl", "half an hour of nothing"],
        &[("TT_MAX_UNVOUCHED_MINUTES", "10")],
    );
    run.assert_status(65);
    run.assert_stderr_has(&format!("{span}m gap"));
}

// --- idle and trim ---------------------------------------------------------
//
// `commands::log` takes the intervals as values and splits inside the same store
// transaction, so these assert on the epoch pairs in the sandbox's `data.json`.

/// Two holes in one phase, so the *order* of the recorded intervals matters.
struct TwoGapFixture {
    holes: [(i64, i64); 2],
}

fn two_gap_fixture(case: &Case) -> TwoGapFixture {
    let start = now() - 150 * 60;
    let beats = [
        start + 10 * 60,
        start + 70 * 60,
        start + 80 * 60,
        start + 140 * 60,
    ];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);
    TwoGapFixture {
        holes: [(beats[0], beats[1]), (beats[2], beats[3])],
    }
}

#[test]
fn every_flagged_gap_becomes_an_idle_interval_in_chronological_order() {
    let case = Case::new("idle-order");
    let fixture = two_gap_fixture(&case);

    case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"])
        .assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "--full never splits");
    // One interval per fabricated hole, counted from the fixture.
    let recorded: Vec<(i64, i64)> = entries[0].idle.iter().map(|gap| gap.epochs()).collect();
    assert_eq!(recorded, fixture.holes, "the fixture's holes, in order");
}

#[test]
fn a_phase_with_no_flagged_gap_records_none_and_no_trim() {
    let case = Case::new("idle-none");
    let step = 600;
    let start = now() - step * 24;
    let beats: Vec<i64> = (1..=24).map(|i| start + i * step).collect();
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &beats);

    case.run(&["end", "proj", "7", "impl", "long active session"])
        .assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    assert!(entries[0].idle.is_empty(), "no silence to record");
}

/// `--full` asks for no split: the entry stands whole with its interval on it.
#[test]
fn trim_adds_the_split_and_full_does_not() {
    let case = Case::new("idle-full-no-trim");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    run.assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "--full asks for no split");
    assert_eq!(
        entries[0]
            .idle
            .iter()
            .map(|gap| gap.epochs())
            .collect::<Vec<_>>(),
        vec![fixture.hole],
        "the silence is recorded, not removed"
    );
    assert_eq!(entries[0].seconds(), fixture.minutes * 60);
}

/// A mark-derived entry is pinned to the mark's timeline, not to `now`: `end`
/// measures to the last heartbeat, so the entry and its `idle` epochs end there.
#[test]
fn a_mark_derived_entry_ends_at_the_marks_last_heartbeat() {
    let case = Case::new("anchor-last-beat");
    let fixture = gap_fixture(&case);

    case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"])
        .assert_status(0);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(
        entry
            .end_time
            .expect("a logged entry is closed")
            .timestamp(),
        fixture.last_beat,
        "the entry ends at the last beat, not at now"
    );

    // Every recorded interval falls inside the entry that carries it, inclusive.
    let start = entry.start_time.timestamp();
    let end = fixture.last_beat;
    for gap in &entry.idle {
        let (from, to) = gap.epochs();
        assert!(
            from >= start && to <= end,
            "idle {from}-{to} escaped the entry {start}-{end}"
        );
    }
    assert!(!entry.idle.is_empty(), "the fixture's hole was recorded");
}

/// The duration floors to whole minutes, so the entry starts after the mark.
/// The first gap starts at the mark and `tt log` must still accept it.
#[test]
fn a_span_of_part_minutes_clamps_the_first_gap_to_the_entry_start() {
    let case = Case::new("idle-part-minute-span");
    let start = now() - 6100;
    let beats = [start + 3000, start + 6030];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);

    case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"])
        .assert_status(0);

    let entries = case.store().entries;
    let entry_start = entries[0].start_time.timestamp();
    assert_eq!(
        entry_start,
        start + 30,
        "100 whole minutes back from the end"
    );
    let recorded: Vec<(i64, i64)> = entries[0].idle.iter().map(|gap| gap.epochs()).collect();
    assert_eq!(
        recorded,
        vec![(entry_start, beats[0]), (beats[0], beats[1])]
    );
}

/// The **stored span** is the assertion: stdout's figure and an empty `idle` are
/// both satisfied by a split that subtracted twice and left a fragment behind.
#[test]
fn trim_subtracts_each_gap_exactly_once_and_reports_what_survived() {
    let case = Case::new("idle-trim");
    let fixture = gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--trim"]);
    run.assert_status(0);

    // `split_at_idle` does the subtraction, so what survives is the measured span
    // minus the hole.
    let survives = fixture.minutes - fixture.hole_minutes;
    let entries = case.store().entries;
    let stored: i64 = entries.iter().map(|entry| entry.seconds()).sum();
    assert_eq!(
        stored,
        survives * 60,
        "the store holds the span minus the hole, once: {entries:?}"
    );
    // A piece so short it is not work — a fragment plus a full piece would sum right.
    assert!(
        entries.iter().all(|entry| entry.seconds() >= 60),
        "a sub-second fragment survived the split: {entries:?}"
    );
    // And it reports what it stored, not what it was asked for.
    assert_eq!(run.logged_minutes(), survives);

    // The surviving piece ends where the silence began, not near `now`.
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.end_time.expect("closed").timestamp())
            .max(),
        Some(fixture.hole.0),
        "the last surviving piece ends at the start of the hole"
    );
    // A split on every interval consumes them all; `--full` keeps the evidence.
    assert!(
        entries.iter().all(|entry| entry.idle.is_empty()),
        "the split did not consume the interval: {entries:?}"
    );
    assert!(!case.mark_file("proj.12.plan").exists());
    assert!(!case.beats_file("proj.12.plan").exists());
}

/// The flag and the override are given *together*, and the override still wins.
#[test]
fn explicit_minutes_beat_trim_as_well_and_record_nothing() {
    let case = Case::new("idle-explicit");
    gap_fixture(&case);

    let run = case.run(&[
        "end",
        "proj",
        "12",
        "plan",
        "planned the thing",
        "30",
        "--trim",
    ]);
    run.assert_status(0);
    assert_eq!(run.logged_minutes(), 30);

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    // The mark's timestamps were skipped entirely, so there was no silence to find.
    assert!(entries[0].idle.is_empty(), "an idle interval was recorded");
}

/// Quarter-aligned, so `--full` and `--trim` differ by the hole alone.
struct AlignedGapFixture {
    hole_minutes: i64,
}

fn aligned_gap_fixture(case: &Case) -> AlignedGapFixture {
    let start = now() - 120 * 60;
    let beats = [start + 30 * 60, start + 120 * 60];
    case.write_mark("proj.12.plan", start);
    case.beats_at("proj.12.plan", &beats);
    AlignedGapFixture {
        hole_minutes: (beats[1] - beats[0]) / 60,
    }
}

#[test]
fn full_logs_the_whole_measured_span() {
    let case = Case::new("aligned-full");
    aligned_gap_fixture(&case);

    let run = case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    run.assert_status(0);
    assert!(run.logged_minutes() > 0, "no duration was logged");
}

/// Two runs compared as a **delta**, so neither figure is written down.
#[test]
fn trim_logs_the_span_minus_every_flagged_gap() {
    let full_case = Case::new("aligned-full-delta");
    let fixture = aligned_gap_fixture(&full_case);
    let full = full_case.run(&["end", "proj", "12", "plan", "planned the thing", "--full"]);
    full.assert_status(0);

    let trim_case = Case::new("aligned-trim-delta");
    aligned_gap_fixture(&trim_case);
    let trim = trim_case.run(&["end", "proj", "12", "plan", "planned the thing", "--trim"]);
    trim.assert_status(0);

    assert_eq!(
        full.logged_minutes() - trim.logged_minutes(),
        fixture.hole_minutes,
        "minutes removed by --trim"
    );
}

/// Not a usage error: nothing flagged simply logs the whole span.
#[test]
fn trim_on_a_phase_with_nothing_flagged_is_a_no_op() {
    let case = Case::new("idle-trim-noop");
    let span = 30 * 60;
    let start = now() - span;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[start + 10 * 60, start + span]);

    let run = case.run(&["end", "proj", "7", "impl", "nothing to trim", "--trim"]);
    run.assert_status(0);
    // `--trim` alone is a clap usage error, so `trim: true` must never be passed alone.
    run.assert_stdout_has(&logged_duration(span / 60));

    let entries = case.store().entries;
    assert_eq!(entries.len(), 1, "nothing was split");
    assert!(entries[0].idle.is_empty());
}

// --- an unwritable mark directory ------------------------------------------

/// A 30-minute beaten phase with one unrelated entry already in the store, so
/// every count below is a delta against something.
struct ClosableFixture {
    /// Entries in the store before the close.
    before: usize,
    start: i64,
}

fn closable_fixture(case: &Case) -> ClosableFixture {
    case.write_store(&[StoreRow {
        description: "an earlier phase",
        project: Some("proj"),
        tags: &["proj/7", "review", "agent"],
        start: now() - 7200,
        end: Some(now() - 5400),
    }]);
    let start = now() - 30 * 60;
    case.write_mark("proj.7.impl", start);
    case.beats_at("proj.7.impl", &[start + 10 * 60, start + 30 * 60]);
    ClosableFixture {
        before: case.store().entries.len(),
        start,
    }
}

/// The failure needs no crash: an unwritable mark directory alone used to log the
/// entry and leave the mark, so the retry the non-zero exit invites logged the
/// same span again.
/// Needs a mode change to make `unlink` fail, which has no Windows equivalent.
#[cfg(unix)]
#[test]
fn an_unwritable_mark_directory_logs_nothing_and_the_retry_logs_once() {
    let case = Case::new("end-unwritable");
    let fixture = closable_fixture(&case);

    let run = {
        let _mode = Mode::set(&case.marks, 0o555);
        case.run(&["end", "proj", "7", "impl", "did the thing"])
    };
    assert_eq!(run.status, Some(1), "stderr: {:?}", run.stderr);
    assert_ne!(
        run.status,
        Some(74),
        "74 would mean the entry was recorded after all"
    );
    assert_eq!(
        case.store().entries.len() - fixture.before,
        0,
        "the store gained an entry: {:?}",
        case.store().entries
    );
    assert!(case.mark_file("proj.7.impl").is_file(), "the mark survived");
    assert!(!case.closing_file("proj.7.impl").exists());

    let retry = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    retry.assert_status(0);
    assert_eq!(
        case.store().entries.len() - fixture.before,
        1,
        "the retry logged the span more than once: {:?}",
        case.store().entries
    );
    assert_eq!(case.mark_count(), 0);
    assert!(!case.closing_file("proj.7.impl").exists());
}

/// The residual state this narrows to: recorded, uncleared, and refused rather
/// than logged a second time.
#[test]
fn a_close_left_unfinished_refuses_instead_of_logging_again() {
    let case = Case::new("end-unfinished");
    let fixture = closable_fixture(&case);
    case.write_closing("proj.7.impl", fixture.start);

    let run = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    run.assert_status(75);
    run.assert_stderr_has("proj/7 impl has an unfinished close");
    run.assert_stderr_has("tt agent cancel proj 7 impl");
    assert_eq!(
        case.store().entries.len() - fixture.before,
        0,
        "the store gained an entry: {:?}",
        case.store().entries
    );
    // Named, never cleared: only the operator can tell whether the entry landed.
    assert!(case.closing_file("proj.7.impl").is_file());
    assert!(case.mark_file("proj.7.impl").is_file());
}

/// The one case where the entry does land and the cleanup does not: the `closing/`
/// directory is already there, so only the mark's own removal hits the mode.
/// Needs a mode change to make `unlink` fail, which has no Windows equivalent.
#[cfg(unix)]
#[test]
fn an_entry_recorded_with_the_mark_left_behind_exits_74() {
    let case = Case::new("end-uncleared");
    let fixture = closable_fixture(&case);
    fs::create_dir_all(case.closing_file("proj.7.impl").parent().unwrap()).unwrap();

    let run = {
        let _mode = Mode::set(&case.marks, 0o555);
        case.run(&["end", "proj", "7", "impl", "did the thing"])
    };
    run.assert_status(74);
    run.assert_stderr_has("proj/7 impl is recorded, but its mark could not be cleared");
    run.assert_stderr_has("do not retry the close");
    assert_eq!(
        case.store().entries.len() - fixture.before,
        1,
        "the entry is recorded exactly once: {:?}",
        case.store().entries
    );
    assert!(case.mark_file("proj.7.impl").is_file(), "the mark survived");

    // A caller that retries anyway is refused rather than logging the span twice.
    let retry = case.run(&["end", "proj", "7", "impl", "did the thing"]);
    retry.assert_status(75);
    assert_eq!(case.store().entries.len() - fixture.before, 1);
}

/// An unvouched phase's holes cover its whole span, so `--trim` would store
/// all of it: the refusal names only `--full` and the explicit minutes.
#[test]
fn an_unvouched_refusal_names_no_trim_figure() {
    let case = Case::new("gaps-unvouched-no-trim");
    let span = 150;
    let start = now() - span * 60;
    case.write_mark("proj.7.impl", start);

    let run = case.run(&["end", "proj", "7", "impl", "no evidence either way"]);
    run.assert_status(65);
    run.assert_stderr_has(&format!("tt: --full logs {span}m\n"));
    run.assert_stderr_has("or pass the real minutes instead.");
    assert!(
        !run.stderr.contains("--trim"),
        "the refusal named a --trim figure it will not honour: {:?}",
        run.stderr
    );
}

// --- the agent label -------------------------------------------------------

/// One labelled close bills its own span and leaves its sibling open; the label is
/// no tag axis, so the entry reads exactly as an unlabelled one.
#[test]
fn a_labelled_end_bills_and_clears_only_its_own_mark() {
    let case = Case::new("end-agent-label");
    let elapsed = 1800;
    case.write_mark("proj.7.review.code", now() - elapsed);
    case.write_mark("proj.7.review.style", now() - elapsed);

    let run = case.run(&[
        "end",
        "proj",
        "7",
        "review",
        "--agent",
        "code",
        "read the diff",
    ]);
    run.assert_status(0);
    run.assert_stdout_has(&format!(
        "\"read the diff\" (proj) [#proj/7 #review #agent] {}",
        logged_duration(elapsed / 60)
    ));
    assert!(!case.mark_file("proj.7.review.code").exists());
    assert!(
        case.mark_file("proj.7.review.style").is_file(),
        "the other label's mark was cleared"
    );
}

/// The label is stamped into the entry's own data, which is where a reader gets
/// it back: it is no tag axis.
#[test]
fn a_labelled_end_records_the_label_under_the_agent_namespace() {
    let case = Case::new("end-agent-data");
    case.write_mark("proj.7.review.code", now() - 1800);

    case.run(&[
        "end",
        "proj",
        "7",
        "review",
        "--agent",
        "code",
        "read the diff",
    ])
    .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(
        entry.data,
        Some(serde_json::json!({"agent": {"label": "code"}}))
    );
}

#[test]
fn a_labelled_end_merges_the_label_into_the_data_the_caller_passed() {
    let case = Case::new("end-agent-data-merge");
    case.write_mark("proj.7.review.code", now() - 1800);

    case.run(&[
        "end",
        "proj",
        "7",
        "review",
        "--agent",
        "code",
        "read the diff",
        "--data",
        r#"{"pr": 42, "agent": {"model": "opus", "tokens": {"input": 1200}}}"#,
    ])
    .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(
        entry.data,
        Some(serde_json::json!({
            "pr": 42,
            "agent": {"label": "code", "model": "opus", "tokens": {"input": 1200}},
        }))
    );
}

#[test]
fn an_unlabelled_end_leaves_the_data_exactly_as_it_was_passed() {
    let case = Case::new("end-no-agent-data");
    case.write_mark("proj.7.review", now() - 1800);

    case.run(&[
        "end",
        "proj",
        "7",
        "review",
        "read the diff",
        "--data",
        r#"{"pr": 42}"#,
    ])
    .assert_status(0);

    let entry = &case.store().entries[0];
    assert_eq!(entry.data, Some(serde_json::json!({"pr": 42})));

    let bare = Case::new("end-no-agent-no-data");
    bare.write_mark("proj.7.review", now() - 1800);
    bare.run(&["end", "proj", "7", "review", "read the diff"])
        .assert_status(0);
    assert_eq!(bare.store().entries[0].data, None);
}

/// Exit 64 and nothing recorded: the label has nowhere to go, and the mark is
/// left for the retry.
#[test]
fn a_labelled_end_over_a_non_object_agent_key_exits_64() {
    let case = Case::new("end-agent-data-clash");
    case.write_mark("proj.7.review.code", now() - 1800);

    let run = case.run(&[
        "end",
        "proj",
        "7",
        "review",
        "--agent",
        "code",
        "read the diff",
        "--data",
        r#"{"agent": "code"}"#,
    ]);
    run.assert_status(64);
    run.assert_stderr_has("expected \"agent\" to be a JSON object, got a string");
    assert!(case.store().entries.is_empty(), "an entry was recorded");
    assert!(
        case.mark_file("proj.7.review.code").is_file(),
        "the mark was cleared"
    );
}

#[test]
fn a_labelled_refusal_names_the_mark_its_recovery_clears() {
    let case = Case::new("end-agent-unfinished");
    let start = now() - 1800;
    case.write_mark("proj.7.review.code", start);
    case.write_closing("proj.7.review.code", start);

    let run = case.run(&[
        "end",
        "proj",
        "7",
        "review",
        "--agent",
        "code",
        "read the diff",
    ]);
    run.assert_status(75);
    run.assert_stderr_has("proj/7 review:code has an unfinished close");
    run.assert_stderr_has("tt agent cancel proj 7 review --agent code");
    assert!(case.store().entries.is_empty(), "nothing was logged");
}

// --- the close's own uncovered remainder -----------------------------------

/// Explicit minutes end the entry *now*, so the mark's earlier span is left
/// uncovered. With a session to name, that remainder is a dismissal.
#[test]
fn explicit_minutes_with_a_session_dismiss_the_mark_s_uncovered_head() {
    let case = Case::new("end-session-dismiss");
    let start = now() - 3 * 3600;
    case.write_mark("proj.7.impl", start);
    case.write_session("sess-1", "proj", start, Some(now()));

    let run = case.run(&[
        "end",
        "proj",
        "7",
        "impl",
        "relayed the phase",
        "20",
        "--session",
        "sess-1",
    ]);
    run.assert_status(0);

    let audit = case.run(&["audit"]);
    audit.assert_status(0);
    audit.assert_stdout_has("No unaccounted agent activity.");
    assert_eq!(common::count_files(&case.dismissed), 1);
}

/// A dismissal names one session, so a concurrent session's row over the same
/// minutes is untouched by another agent's close.
#[test]
fn a_close_s_dismissal_leaves_a_concurrent_session_s_row_alone() {
    let case = Case::new("end-session-dismiss-concurrent");
    let start = now() - 3 * 3600;
    case.write_mark("proj.7.impl", start);
    case.write_session("sess-1", "proj", start, Some(start + 60));
    case.write_session("sess-2", "proj", start, Some(now()));

    case.run(&[
        "end",
        "proj",
        "7",
        "impl",
        "relayed the phase",
        "20",
        "--session",
        "sess-1",
    ])
    .assert_status(0);

    let audit = case.run(&["audit"]);
    audit.assert_stdout_has("proj");
    audit.assert_stdout_has("sess-2");
}

#[test]
fn trim_with_a_session_dismisses_each_gap_it_cut() {
    let case = Case::new("end-session-dismiss-trim");
    let fixture = gap_fixture(&case);
    case.write_session("sess-1", "proj", fixture.hole.0, Some(now()));

    let run = case.run(&[
        "end",
        "proj",
        "12",
        "plan",
        "planned the thing",
        "--trim",
        "--session",
        "sess-1",
    ]);
    run.assert_status(0);
    assert_eq!(common::count_files(&case.dismissed), 1);
}

#[test]
fn full_writes_no_dismissal() {
    let case = Case::new("end-session-dismiss-full");
    gap_fixture(&case);

    case.run(&[
        "end",
        "proj",
        "12",
        "plan",
        "planned the thing",
        "--full",
        "--session",
        "sess-1",
    ])
    .assert_status(0);
    assert_eq!(common::count_files(&case.dismissed), 0);
}

/// No session, no guess: the remainder is named on stderr and the close is
/// otherwise exactly as before.
#[test]
fn without_a_session_the_close_names_the_remainder_and_writes_nothing() {
    let case = Case::new("end-no-session-remainder");
    let start = now() - 3 * 3600;
    case.write_mark("proj.7.impl", start);

    let run = case.run(&["end", "proj", "7", "impl", "relayed the phase", "20"]);
    run.assert_status(0);
    run.assert_stderr_has("tt agent audit");
    assert_eq!(common::count_files(&case.dismissed), 0);
    assert_eq!(case.store().entries.len(), 1);
}

/// `end` with explicit minutes works with no mark at all, and the remainder
/// path must not change that.
#[test]
fn explicit_minutes_with_a_session_and_no_mark_still_closes_and_writes_nothing() {
    let case = Case::new("end-session-no-mark");
    let run = case.run(&[
        "end",
        "proj",
        "7",
        "impl",
        "no mark to speak of",
        "20",
        "--session",
        "sess-1",
    ]);
    run.assert_status(0);
    assert_eq!(case.store().entries.len(), 1);
    assert_eq!(common::count_files(&case.dismissed), 0);
}
