use super::App;
use super::types::{ConfirmAction, InputMode, PendingConfirm, ViewMode};
use crate::storage::PathStamp;
use anyhow::Result;
use chrono::{Datelike, Duration, Local, NaiveDate};
use crossterm::event::KeyCode;

impl App {
    /// Record the store's fingerprint, then load it. Stamp *before* the read —
    /// stamping after records a fingerprint newer than the data and loses the update.
    pub(crate) fn reload(&mut self) -> Result<()> {
        self.store_stamp = crate::storage::store_stamp();
        self.set_data(crate::storage::load_data()?);
        Ok(())
    }

    /// Whether the store on disk still matches the snapshot in memory.
    pub(crate) fn store_is_unchanged(&self) -> bool {
        PathStamp::unchanged(self.store_stamp, crate::storage::store_stamp())
    }

    /// Pick up marks begun, ended or cancelled outside the TUI, on the same tick as
    /// [`sync_from_store`](Self::sync_from_store). Unguarded by `input_mode` and by
    /// `show_marks`, since the mark list is display-only; costs one `stat` per tick.
    pub(crate) fn sync_from_marks(&mut self) {
        let Some(dir) = crate::marks::mark_dir() else {
            self.marks.clear();
            return;
        };
        // Stamp before reading, as `reload` does.
        let current = PathStamp::read(&dir);
        if PathStamp::unchanged(self.marks_stamp, current) {
            return;
        }
        self.marks_stamp = current;
        self.marks = crate::marks::open_marks_in(&dir);
    }

    /// Pick up activity-ledger writes made outside the TUI (by hooks in other
    /// sessions), and recompute [`App::unaccounted`] from whatever `marks` and
    /// `data` already hold this tick. Call after
    /// [`sync_from_marks`](Self::sync_from_marks) and
    /// [`sync_from_store`](Self::sync_from_store) so both are current;
    /// reconciliation itself is cheap enough to redo every tick; only the
    /// session directory read is gated on its own `stat`.
    pub(crate) fn sync_from_activity(&mut self) {
        if let Some(dir) = crate::activity::activity_dir() {
            let current = PathStamp::read(&dir);
            if !PathStamp::unchanged(self.activity_stamp, current) {
                self.activity_stamp = current;
                self.activity_sessions = crate::activity::read_sessions_in(&dir);
            }
        } else {
            self.activity_sessions.clear();
        }
        self.unaccounted = crate::audit::unaccounted(
            &self.activity_sessions,
            &self.marks,
            &self.data.entries,
            Local::now(),
            crate::audit::max_unvouched_minutes(),
        );
    }

    /// Pick up writes made outside the TUI, once per event-loop tick.
    pub(crate) fn sync_from_store(&mut self) -> Result<()> {
        // Replacing `data` mid-form would clobber the text being typed; a later tick
        // picks the change up once the mode is Normal again.
        if matches!(
            self.input_mode,
            InputMode::AddingEntry | InputMode::EditingEntry | InputMode::Searching
        ) {
            return Ok(());
        }
        if self.store_is_unchanged() {
            return Ok(());
        }

        // The index means nothing once the data changes, so anchor on the id.
        let previous_idx = self.table_state.selected();
        let anchor_id =
            previous_idx.and_then(|idx| self.filtered_entries().get(idx).map(|entry| entry.id));

        self.reload()?;

        let (anchored_idx, len) = {
            let entries = self.filtered_entries();
            let anchored = anchor_id.and_then(|id| entries.iter().position(|e| e.id == id));
            (anchored, entries.len())
        };
        self.table_state.select(match (anchored_idx, previous_idx) {
            (Some(idx), _) => Some(idx),
            // The anchor is gone: stay as near the old position as the list allows.
            (None, Some(idx)) => Some(idx.min(len.saturating_sub(1))),
            (None, None) => None,
        });

        // `Confirm` is absent from the guard above, so the reload can carry away the
        // entry a prompt is describing; drop the prompt rather than leave it asking.
        let target_vanished = self
            .pending_confirm
            .is_some_and(|pending| self.data.get_entry(pending.entry_id).is_none());
        if target_vanished {
            self.cancel_confirm();
        }
        Ok(())
    }

    pub(crate) fn next(&mut self) {
        let len = self.filtered_len();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub(crate) fn previous(&mut self) {
        let len = self.filtered_len();
        if len == 0 {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    len - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
    }

    pub(crate) fn selected_entry(&self) -> Option<&crate::tracker::TimeEntry> {
        let idx = self.table_state.selected()?;
        self.filtered_entries().into_iter().nth(idx)
    }

    /// `Enter` on the table: show the entry in full, or nothing when none is selected.
    pub(crate) fn open_detail(&mut self) {
        if self.selected_entry().is_some() {
            self.input_mode = InputMode::Detail;
        }
    }

    /// Remove one entry by id, then keep the table cursor inside the shorter list.
    /// By id and never by selection — see [`PendingConfirm`].
    pub(crate) fn delete_entry(&mut self, entry_id: u64) -> Result<()> {
        // An id that is already gone matches nothing — not an error.
        self.mutate_store(|data| data.entries.retain(|e| e.id != entry_id))?;

        let new_len = self.filtered_len();
        let past_the_end = self
            .table_state
            .selected()
            .is_some_and(|idx| idx >= new_len);
        if past_the_end && new_len > 0 {
            self.table_state.select(Some(new_len - 1));
        }
        Ok(())
    }

    /// `[t]` in the detail popover, once confirmed: split the entry at its idle.
    /// Re-selects **by id**, since `selected_entry` is positional and the inserted
    /// pieces shift the list.
    pub(crate) fn trim_entry(&mut self, entry_id: u64) -> Result<()> {
        let pieces = self.mutate_store(|data| data.split_at_idle(entry_id))?;
        if pieces.is_empty() {
            return Ok(());
        }
        self.select_by_id(entry_id);
        Ok(())
    }

    /// Raise the prompt for a destructive action on the selected entry. **The target
    /// id is captured here, now**, because the 250 ms poll can re-anchor the table
    /// cursor while the prompt is up. A trim with no idle to trim raises nothing.
    pub(crate) fn request_confirm(&mut self, action: ConfirmAction) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let entry_id = entry.id;
        if action == ConfirmAction::Trim && entry.trim_spans().is_empty() {
            return;
        }
        self.pending_confirm = Some(PendingConfirm {
            action,
            entry_id,
            from: self.input_mode,
        });
        self.input_mode = InputMode::Confirm;
    }

    /// Answer the prompt with a keypress — the whole body of the `Confirm` arm. `n`,
    /// `Esc` and `Enter` are all a no; anything else is ignored and the prompt stays.
    pub(crate) fn answer_confirm(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char(c) if self.confirms_pending(c) => self.confirm_pending()?,
            KeyCode::Char('n') | KeyCode::Esc | KeyCode::Enter => self.cancel_confirm(),
            _ => {}
        }
        Ok(())
    }

    /// Whether `c` is a yes: `y`, or the key that raised this prompt — so a `d` on a
    /// trim prompt and a `t` on a delete prompt are both inert.
    pub(crate) fn confirms_pending(&self, c: char) -> bool {
        self.pending_confirm
            .is_some_and(|pending| c == 'y' || c == pending.action.key())
    }

    /// The yes: act on **the captured id and nothing else**, or self-cancel when that
    /// id is no longer in the store.
    pub(crate) fn confirm_pending(&mut self) -> Result<()> {
        let Some(pending) = self.pending_confirm.take() else {
            return Ok(());
        };
        if self.data.get_entry(pending.entry_id).is_some() {
            match pending.action {
                ConfirmAction::Delete => self.delete_entry(pending.entry_id)?,
                ConfirmAction::Trim => self.trim_entry(pending.entry_id)?,
            }
        }
        self.close_confirm(pending.from);
        Ok(())
    }

    /// The no: `n`, `Esc` and `Enter` all land here, and the store is untouched.
    pub(crate) fn cancel_confirm(&mut self) {
        let from = self
            .pending_confirm
            .take()
            .map_or(InputMode::Normal, |pending| pending.from);
        self.close_confirm(from);
    }

    /// Back to the mode that raised the prompt, or `Normal` if its view has emptied.
    fn close_confirm(&mut self, from: InputMode) {
        self.pending_confirm = None;
        self.input_mode = if from == InputMode::Detail && self.selected_entry().is_none() {
            InputMode::Normal
        } else {
            from
        };
    }

    /// Put the table cursor on this id, or leave it alone if the view lacks that row.
    pub(crate) fn select_by_id(&mut self, id: u64) {
        if let Some(idx) = self.filtered_entries().iter().position(|e| e.id == id) {
            self.table_state.select(Some(idx));
        }
    }

    /// The detail popover's footer hints, in render order. `[t]` appears only on an
    /// entry that has idle; `d` and `t` trail a `…` because they raise a confirmation.
    pub(crate) fn detail_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = vec![("j/k", "move"), ("e", "edit"), ("d", "delete…")];
        if self.selected_entry().is_some_and(|e| !e.idle.is_empty()) {
            // "trim" is the user-facing verb, though the store call is `split_at_idle`.
            hints.push(("t", "trim…"));
        }
        hints.push(("esc", "close"));
        hints
    }

    pub(crate) fn stop_active(&mut self) -> Result<()> {
        self.mutate_store(|data| data.stop_active())?;
        Ok(())
    }

    pub(crate) fn next_period(&mut self) {
        match self.view_mode {
            ViewMode::All => {}
            ViewMode::Day => self.selected_date += Duration::days(1),
            ViewMode::Week => self.selected_date += Duration::days(7),
            ViewMode::Overview => self.selected_date = shift_year(self.selected_date, 1),
        }
        self.table_state.select(Some(0));
    }

    pub(crate) fn previous_period(&mut self) {
        match self.view_mode {
            ViewMode::All => {}
            ViewMode::Day => self.selected_date -= Duration::days(1),
            ViewMode::Week => self.selected_date -= Duration::days(7),
            ViewMode::Overview => self.selected_date = shift_year(self.selected_date, -1),
        }
        self.table_state.select(Some(0));
    }

    pub(crate) fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
        self.table_state.select(Some(0));
    }

    pub(crate) fn go_to_today(&mut self) {
        self.selected_date = Local::now().date_naive();
        self.table_state.select(Some(0));
    }

    pub(crate) fn toggle_sort_order(&mut self) {
        self.sort_order = self.sort_order.toggle();
        self.table_state.select(Some(0));
    }
}

/// `date` moved `delta` years, falling back to Feb 28 when `date` is a Feb 29
/// that the target year does not have.
fn shift_year(date: NaiveDate, delta: i32) -> NaiveDate {
    let year = date.year() + delta;
    date.with_year(year)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, 2, 28).unwrap())
}
