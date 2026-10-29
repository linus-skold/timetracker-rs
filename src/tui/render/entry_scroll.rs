//! Where the Entries table starts drawing, in visual rows.

/// Keep the stored offset inside the list, so a shorter list leaves no gap.
pub(super) fn clamp_offset(offset: usize, row_count: usize, viewport: usize) -> usize {
    offset.min(row_count.saturating_sub(viewport))
}

/// The offset that shows as many of a group's new member rows as fit.
/// The group row stays on screen: the offset never passes it.
pub(super) fn reveal_group_offset(
    offset: usize,
    group_row: usize,
    member_count: usize,
    viewport: usize,
) -> usize {
    let last_member = group_row + member_count;
    if viewport == 0 || last_member < offset + viewport {
        return offset;
    }
    (last_member + 1 - viewport).min(group_row)
}
