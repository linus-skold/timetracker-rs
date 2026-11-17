//! Plain-text tables for the CLI listings: left-aligned columns, two spaces apart.

/// Render `headers` and `rows` with every column padded to its widest cell.
/// Trailing padding is trimmed, so no line ends in spaces.
pub fn render(headers: &[&str], rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = (0..headers.len())
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .chain(std::iter::once(headers[column].chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    std::iter::once(&header_cells)
        .chain(rows)
        .map(|row| format_row(row, &widths))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_row(cells: &[String], widths: &[usize]) -> String {
    let padded: Vec<String> = cells
        .iter()
        .zip(widths)
        .map(|(cell, width)| format!("{cell:<width$}"))
        .collect();
    padded.join("  ").trim_end().to_string()
}
