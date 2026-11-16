//! Close or drop an open agent mark from the TUI: the `x` picker, the
//! Edit / Discard prompt, and the save of the prefilled entry form.
//! The CLI twin is `tt agent end` / `tt agent cancel` in `src/agent.rs`.

use super::App;
use super::entry_form::EntryFields;
use super::types::InputMode;
use crate::marks;
use anyhow::Result;
use chrono::{Duration, Local};

/// How the form shows a time, as `start_editing` does.
const FORM_TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

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
            .set_from(&now.format(FORM_TIME_FORMAT).to_string());
        self.input_duration.set_from(&crate::duration::format(
            (now - mark.start).max(Duration::zero()),
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
        if marks::is_closing_in(&dir, key) {
            return self.finish_mark_close(format!(
                "{} has an unfinished close. Check the report, then discard it.",
                mark.label()
            ));
        }
        let still_open = matches!(
            marks::read_phase_in(&dir, key),
            Ok(Some(phase)) if phase.started == mark.start.timestamp()
        );
        if !still_open {
            return self.finish_mark_close(format!(
                "{} is closed already. Nothing logged.",
                mark.label()
            ));
        }

        // Written before the entry, as `tt agent end` does.
        marks::start_closing_in(&dir, key)?;
        self.mutate_store(|store| {
            let id = store
                .add_entry(description, project, tags, start_time, Some(end_time))
                .id;
            store.set_entry_data(id, data);
        })?;
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
