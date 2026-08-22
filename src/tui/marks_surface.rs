//! The collapsible `Marks` surface: the phase marks left open by `tt agent
//! begin`, read from `App.marks` on the event loop's tick — a frame never reads
//! the directory. Display-only: no focus ring, no cursor, no `Enter`, no scroll.
//!
//! Below the marks, a second state — unaccounted activity, from `App.unaccounted`
//! (see `src/audit.rs`) — appears only when there is any, so an operator who never
//! sees it pays no visual cost. Same reconciliation the CLI's `tt agent audit`
//! runs; nothing here recomputes it.

use super::App;
use super::panes::surface_count;
use crate::audit::Unaccounted;
use crate::marks::Mark;

/// Most marks the surface lists; the border count reports the rest.
const MAX_VISIBLE_MARKS: usize = 3;
/// Most unaccounted windows the surface lists below the marks.
const MAX_VISIBLE_UNACCOUNTED: usize = 3;

impl App {
    /// The newest [`MAX_VISIBLE_MARKS`], in `App.marks`' own newest-first order.
    pub(crate) fn visible_marks(&self) -> &[Mark] {
        let shown = self.marks.len().min(MAX_VISIBLE_MARKS);
        &self.marks[..shown]
    }

    /// The newest [`MAX_VISIBLE_UNACCOUNTED`], in `App.unaccounted`'s own
    /// newest-first order.
    pub(crate) fn visible_unaccounted(&self) -> &[Unaccounted] {
        let shown = self.unaccounted.len().min(MAX_VISIBLE_UNACCOUNTED);
        &self.unaccounted[..shown]
    }

    /// Height including borders. 0 while hidden, so the layout drops the row
    /// entirely; an empty `unaccounted` adds nothing, so a clean session looks
    /// exactly as it did before this existed.
    pub(crate) fn marks_surface_height(&self) -> u16 {
        if !self.show_marks {
            return 0;
        }
        let marks_rows = self.marks.len().clamp(1, MAX_VISIBLE_MARKS) as u16;
        let unaccounted_rows = if self.unaccounted.is_empty() {
            0
        } else {
            // One header line plus the rows it introduces.
            1 + self.unaccounted.len().min(MAX_VISIBLE_UNACCOUNTED) as u16
        };
        2 + marks_rows + unaccounted_rows
    }

    /// The top border's count: `N` while all fit, `3/N` once more are open.
    pub(crate) fn marks_count(&self, visible_rows: usize) -> Option<String> {
        surface_count(None, self.marks.len(), visible_rows)
    }

    /// The unaccounted section's own count, same shape as [`marks_count`],
    /// `None` when there's nothing to flag.
    pub(crate) fn unaccounted_count(&self) -> Option<String> {
        surface_count(None, self.unaccounted.len(), MAX_VISIBLE_UNACCOUNTED)
    }

    /// `Shift-A`: show or hide the surface.
    pub(crate) fn toggle_marks(&mut self) {
        self.show_marks = !self.show_marks;
    }
}
