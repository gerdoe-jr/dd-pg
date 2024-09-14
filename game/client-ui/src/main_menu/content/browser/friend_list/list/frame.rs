use egui_extras::TableBody;

/// server list frame (scrollable)
pub fn render(body: TableBody<'_>) {
    body.rows(25.0, 100, |row| {
        let row_index = row.index();
        super::entry::render(row, row_index);
    });
}
