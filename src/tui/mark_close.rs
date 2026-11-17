//! Close or drop an open agent mark from the TUI: the `x` picker, the
//! Edit / Discard prompt, and the save of the prefilled entry form.
//! The CLI twin is `tt agent end` / `tt agent cancel` in `src/agent.rs`.

use super::App;
use super::entry_form::EntryFields;
use super::types::InputMode;
use crate::marks;
use anyhow::Result;
use chrono::{Duration, DurationRound, Local};

/// How the form shows a time, as `start_editing` does.
const FORM_TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

/// Whether the mark on disk is still the one the user picked, by key and start.
fn mark_is_still_open(dir: &std::path::Path, mark: &marks::Mark) -> bool {
    matches!(
        marks::read_phase_in(dir, mark.as_key()),
        Ok(Some(phase)) if phase.started == mark.start.timestamp()
    )
}

/// The time cut to the minute, as `FORM_TIME_FORMAT` shows it.
fn whole_minute(time: chrono::DateTime<Local>) -> chrono::DateTime<Local> {
    time.duration_trunc(Duration::minutes(1)).unwrap_or(time)
}

fn closed_already(mark: &marks::Mark) -> String {
    format!("{} is closed already. Nothing logged.", mark.label())
}

/// The form warning for silences `tt agent end` would refuse over, or `None`.
fn idle_gap_notice(gaps: &[(i64, i64)]) -> Option<String> {
    if gaps.is_empty() {
        return None;
    }
    let clock = |epoch: i64| {
        crate::time::instant(epoch).map_or_else(
            || "??:??".to_string(),
            |instant| instant.format("%H:%M").to_string(),
        )
    };
    let spans: Vec<String> = gaps
        .iter()
        .map(|&(from, to)| format!("{}-{}", clock(from), clock(to)))
        .collect();
    Some(format!("idle gaps: {}, check the times", spans.join(", ")))
}

impl App {
    /// `x`: list every open mark. With none, say so and open nothing.
    pub(crate) fn open_mark_picker(&mut self) {
        if self.marks.is_empty() {
            self.status_message = Some("No open marks.".to_string());
            return;
        }
        self.mark_cursor = 0;
        self.input_mode = InputMode::MarkPicker;
    }

    pub(crate) fn mark_picker_move(&mut self, delta: isize) {
        let len = self.marks.len() as isize;
        if len == 0 {
            return;
        }
        self.mark_cursor = (self.mark_cursor as isize + delta).rem_euclid(len) as usize;
    }

    /// `Enter` in the picker: capture the mark, then ask what to do with it.
    pub(crate) fn mark_picker_choose(&mut self) {
        let Some(mark) = self.marks.get(self.mark_cursor).cloned() else {
            self.cancel_mark_popup();
            return;
        };
        // No command can address it, so neither can the TUI.
        if !marks::closable(&mark) {
            self.status_message = Some(marks::remove_by_hand(&mark));
            self.cancel_mark_popup();
            return;
        }
        self.picked_mark = Some(mark);
        self.input_mode = InputMode::MarkAction;
    }

    /// `Esc` in the picker or the prompt: nothing changes.
    pub(crate) fn cancel_mark_popup(&mut self) {
        self.picked_mark = None;
        self.input_mode = InputMode::Normal;
    }

    /// Discard: drop the mark and log nothing. Like `tt agent cancel`, a mark
    /// that is already gone is not an error.
    pub(crate) fn discard_picked_mark(&mut self) {
        let Some(mark) = self.picked_mark.take() else {
            return;
        };
        self.input_mode = InputMode::Normal;
        let Some(dir) = marks::mark_dir() else {
            self.status_message = Some("No mark directory.".to_string());
            return;
        };
        // An agent may have ended it, or begun a new phase under the same key.
        if !mark_is_still_open(&dir, &mark) {
            self.status_message = Some(closed_already(&mark));
            self.refresh_marks();
            return;
        }
        self.status_message = Some(match marks::cancel_in(&dir, mark.as_key()) {
            Ok(()) => format!("Dropped mark {}.", mark.label()),
            Err(err) => format!("Could not drop mark {}: {err}", mark.label()),
        });
        self.refresh_marks();
    }

    /// Edit: open the entry form with what `tt agent end` would log. The
    /// summary stays empty, as the CLI needs one from the user.
    pub(crate) fn edit_picked_mark(&mut self) {
        let Some(mark) = self.picked_mark.take() else {
            return;
        };
        self.start_adding();
        let key = mark.as_key();
        let (summary, tags) = crate::tracker::parse_tags(&crate::agent::description(
            key.project,
            key.issue,
            key.phase,
            "",
        ));
        let now = Local::now();
        // Measured as `tt agent end` does: to the last bare beat, else to now.
        let phase = marks::mark_dir()
            .and_then(|dir| marks::read_phase_in(&dir, key).ok().flatten())
            .filter(|phase| phase.started == mark.start.timestamp());
        let ended = phase
            .as_ref()
            .map_or(now.timestamp(), |phase| phase.measured_end(now.timestamp()));
        let end = crate::time::instant(ended).unwrap_or(now);
        if let Some(phase) = &phase {
            self.form_notice =
                idle_gap_notice(&phase.flagged_gaps(ended, crate::audit::thresholds()));
        }
        let label_data = mark
            .agent
            .as_deref()
            .and_then(|label| crate::entry_data::with_agent_label(None, label).ok());

        self.input_description.set_from(&summary);
        self.input_project.set_from(&mark.project);
        self.input_tags.set_from(&tags.join(" "));
        self.input_start_time
            .set_from(&mark.start.format(FORM_TIME_FORMAT).to_string());
        self.input_end_time
            .set_from(&end.format(FORM_TIME_FORMAT).to_string());
        // Between the minutes the form shows, so Duration agrees with Start/End.
        self.input_duration.set_from(&crate::duration::format(
            (whole_minute(end) - whole_minute(mark.start)).max(Duration::zero()),
        ));
        self.input_data
            .set_from(&crate::entry_data::to_edit_string(label_data.as_ref()));
        self.closing_mark = Some(mark);
    }

    /// Save the close form: log the entry, then drop the mark. A mark that
    /// someone else closed or replaced logs nothing.
    pub(crate) fn submit_mark_close(&mut self) -> Result<()> {
        let Some(mark) = self.closing_mark.clone() else {
            return Ok(());
        };
        let Some(fields) = self.fields_to_save() else {
            return Ok(());
        };
        let EntryFields {
            description,
            tags,
            start_time,
            end_time,
            data,
        } = fields;
        let Some(end_time) = end_time else {
            self.form_error = Some("A closed mark needs an end time".to_string());
            return Ok(());
        };
        let project = self.parse_project();

        let Some(dir) = marks::mark_dir() else {
            return self.finish_mark_close("No mark directory.".to_string());
        };
        let key = mark.as_key();

        // The gate and the entry share one store lock, as in `tt agent end`. A
        // failure keeps the form open: it must not leave `handle_key`.
        let mut sentinel_is_ours = false;
        let written = self.mutate_store(|store| {
            let started = marks::begin_close_in(&dir, key, Some(mark.start.timestamp()))?;
            if started == marks::CloseStart::Started {
                sentinel_is_ours = true;
                let id = store
                    .add_entry(description, project, tags, start_time, Some(end_time))
                    .id;
                store.set_entry_data(id, data);
            }
            Ok::<_, std::io::Error>(started)
        });
        let started = match written {
            Ok(Ok(started)) => started,
            Ok(Err(err)) => {
                self.form_error = Some(format!("Could not start the close: {err}"));
                return Ok(());
            }
            Err(err) => {
                // Only a sentinel this call wrote may go.
                let cleanup = if sentinel_is_ours {
                    marks::abandon_close_in(&dir, key).err()
                } else {
                    None
                };
                self.form_error = Some(match cleanup {
                    Some(cleanup) => format!(
                        "Could not save the entry: {err}. The closing file could not be removed: {cleanup}"
                    ),
                    None => format!("Could not save the entry: {err}"),
                });
                return Ok(());
            }
        };
        match started {
            marks::CloseStart::Started => {}
            marks::CloseStart::Unfinished => {
                return self.finish_mark_close(format!(
                    "{} has an unfinished close. Check the report, then discard it.",
                    mark.label()
                ));
            }
            marks::CloseStart::Gone => return self.finish_mark_close(closed_already(&mark)),
        }
        let message = match marks::cancel_in(&dir, key) {
            Ok(()) => format!("Closed mark {}.", mark.label()),
            Err(err) => format!(
                "Logged {}, but its mark could not be cleared: {err}",
                mark.label()
            ),
        };
        self.finish_mark_close(message)
    }

    /// Leave the form with a message, and show the new marks at once.
    fn finish_mark_close(&mut self, message: String) -> Result<()> {
        self.cancel_adding();
        self.status_message = Some(message);
        self.refresh_marks();
        Ok(())
    }

    /// Re-read the marks now, not on the next tick.
    fn refresh_marks(&mut self) {
        self.marks_stamp = None;
        self.sync_from_marks();
        self.sync_from_activity();
    }
}
