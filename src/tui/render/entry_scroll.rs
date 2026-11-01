//! Where the Entries table starts drawing, in visual rows.
//!
//! Every helper takes the line height of each visual row, because a day
//! header is taller than an entry.

/// The first row from which the rows up to `last` fit in `viewport` lines.
fn first_offset_fitting(heights: &[usize], last: usize, viewport: usize) -> usize {
    let mut used = 0;
    let mut offset = last + 1;
    while offset > 0 && used + heights[offset - 1] <= viewport {
        offset -= 1;
        used += heights[offset];
    }
    offset
}

/// Keep the stored offset inside the list, so a shorter list leaves no gap.
pub(super) fn clamp_offset(offset: usize, heights: &[usize], viewport: usize) -> usize {
    match heights.len() {
        0 => 0,
        len => offset.min(first_offset_fitting(heights, len - 1, viewport).min(len - 1)),
    }
}

/// The offset that shows as many of a group's new member rows as fit.
/// The group row stays on screen: the offset never passes it.
pub(super) fn reveal_group_offset(
    offset: usize,
    group_row: usize,
    member_count: usize,
    heights: &[usize],
    viewport: usize,
) -> usize {
    if viewport == 0 || heights.is_empty() {
        return offset;
    }
    let last_member = (group_row + member_count).min(heights.len() - 1);
    let needed = first_offset_fitting(heights, last_member, viewport);
    if offset >= needed {
        return offset;
    }
    needed.min(group_row)
}
