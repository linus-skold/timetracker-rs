//! Project grouping: one header row per project inside each day partition,
//! with that project's entries and issue groups nested under it.

use super::App;
use super::rows::{Member, Owner, ProjectHeader, VisibleRow};
use std::collections::HashMap;

impl App {
    /// Flip project grouping, keeping the cursor on what it was on.
    pub(crate) fn toggle_group_by_project(&mut self) {
        let anchor = self.cursor_anchor();
        let header_member = self.first_member_id_under_cursor();
        self.group_by_project = !self.group_by_project;
        // A header that has no twin after the flip falls back to its first
        // member, which resolves to its own row or the header holding it.
        let moved_to = anchor
            .and_then(|a| self.selectable_index_for(&a))
            .or_else(|| header_member.and_then(|id| self.selectable_index_of(id)));
        let selected = moved_to.or_else(|| {
            let last = self.selectable_len().checked_sub(1)?;
            Some(self.table_state.selected()?.min(last))
        });
        self.table_state.select(selected);
    }

    /// The first member's id when the cursor is on a group or project header.
    fn first_member_id_under_cursor(&self) -> Option<u64> {
        let members = match self.selected_row()? {
            VisibleRow::GroupHeader(header) => header.members,
            VisibleRow::ProjectHeader(header) => header.members,
            _ => return None,
        };
        let first = *members.first()?;
        self.data.entries.get(first).map(|entry| entry.id)
    }

    /// Open or close `project`'s header on every day it appears in.
    pub(super) fn toggle_project(&mut self, project: &str) {
        let anchor = self.cursor_anchor();
        if self.collapsed_projects.remove(project) {
            self.reveal_group_members = true;
        } else {
            self.collapsed_projects.insert(project.to_string());
        }
        // Other days' rows of this project move this header's index.
        let moved_to = anchor.and_then(|a| self.selectable_index_for(&a));
        if let Some(idx) = moved_to {
            self.table_state.select(Some(idx));
        }
    }

    /// Append one partition's rows: plain issue grouping, or project headers
    /// that each hold the issue grouping of their own entries.
    pub(super) fn push_partition(&self, partition: &[usize], rows: &mut Vec<VisibleRow>) {
        if !self.group_by_project {
            self.push_segment(partition, rows);
            return;
        }
        for (project, members) in self.split_by_project(partition) {
            let expanded = !self.collapsed_projects.contains(project);
            let (start, end) = self.member_span(&members);
            rows.push(VisibleRow::ProjectHeader(ProjectHeader {
                project: project.to_string(),
                members: members.clone(),
                expanded,
                start,
                end,
            }));
            if expanded {
                let mut children = Vec::new();
                self.push_segment(&members, &mut children);
                rows.extend(nest_under_project(children, project));
            }
        }
    }

    /// The partition's entries by project key, in order of first appearance.
    fn split_by_project(&self, partition: &[usize]) -> Vec<(&str, Vec<usize>)> {
        let mut projects: Vec<(&str, Vec<usize>)> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for index in partition {
            let Some(entry) = self.data.entries.get(*index) else {
                continue;
            };
            let key = super::summary::project_key(entry);
            match seen.get(key) {
                Some(slot) => projects[*slot].1.push(*index),
                None => {
                    seen.insert(key, projects.len());
                    projects.push((key, vec![*index]));
                }
            }
        }
        projects
    }
}

/// The rows that sit under the `selectable`th selectable row when that is a
/// project header: its entries and any issue headers and members, up to the
/// next project or day header.
pub(super) fn rows_under_project_header(rows: &[VisibleRow], selectable: usize) -> Option<usize> {
    let position = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| !matches!(row, VisibleRow::DayHeader { .. }))
        .nth(selectable)?
        .0;
    if !matches!(rows[position], VisibleRow::ProjectHeader(_)) {
        return None;
    }
    Some(
        rows[position + 1..]
            .iter()
            .take_while(|row| {
                !matches!(
                    row,
                    VisibleRow::ProjectHeader(_) | VisibleRow::DayHeader { .. }
                )
            })
            .count(),
    )
}

/// Mark `children`, a project's issue-grouped rows, as hanging off its header.
/// Only the last direct child closes the tree.
fn nest_under_project(children: Vec<VisibleRow>, project: &str) -> Vec<VisibleRow> {
    let is_direct = |row: &VisibleRow| {
        matches!(
            row,
            VisibleRow::GroupHeader(_) | VisibleRow::Entry { member: None, .. }
        )
    };
    let last_direct = children.iter().rposition(is_direct);
    // Whether the latest issue header closed the tree, for its members.
    let mut issue_is_last = false;
    children
        .into_iter()
        .enumerate()
        .map(|(position, row)| {
            let last = Some(position) == last_direct;
            match row {
                VisibleRow::GroupHeader(mut header) => {
                    header.branch = Some(last);
                    issue_is_last = last;
                    VisibleRow::GroupHeader(header)
                }
                VisibleRow::Entry {
                    index,
                    member: None,
                } => VisibleRow::Entry {
                    index,
                    member: Some(Member {
                        owner: Owner::Project(project.to_string()),
                        last,
                        outer_last: None,
                    }),
                },
                VisibleRow::Entry {
                    index,
                    member: Some(mut member),
                } => {
                    member.outer_last = Some(issue_is_last);
                    VisibleRow::Entry {
                        index,
                        member: Some(member),
                    }
                }
                other => other,
            }
        })
        .collect()
}
