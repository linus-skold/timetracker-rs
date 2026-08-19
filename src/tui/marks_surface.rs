//! The collapsible `Marks` surface: the phase marks left open by `tt agent
//! begin`, read from `App.marks` on the event loop's tick — a frame never reads
//! the directory. Display-only: no focus ring, no cursor, no `Enter`, no scroll.

use super::App;
use super::panes::surface_count;
use crate::marks::Mark;

/// Most marks the surface lists; the border count reports the rest.
const MAX_VISIBLE_MARKS: usize = 3;

impl App {
    /// The newest [`MAX_VISIBLE_MARKS`], in `App.marks`' own newest-first order.
    pub(crate) fn visible_marks(&self) -> &[Mark] {
        let shown = self.marks.len().min(MAX_VISIBLE_MARKS);
        &self.marks[..shown]
    }

    /// Height including borders, or 0 while hidden so the layout drops the row.
    pub(crate) fn marks_surface_height(&self) -> u16 {
        if !self.show_marks {
            return 0;
        }
        2 + self.marks.len().clamp(1, MAX_VISIBLE_MARKS) as u16
    }

    /// The top border's count: `N` while all fit, `3/N` once more are open.
    pub(crate) fn marks_count(&self, visible_rows: usize) -> Option<String> {
        surface_count(None, self.marks.len(), visible_rows)
    }

    /// `Shift-A`: show or hide the surface.
    pub(crate) fn toggle_marks(&mut self) {
        self.show_marks = !self.show_marks;
    }
}
