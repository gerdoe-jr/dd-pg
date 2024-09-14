use egui_extras::TableRow;

/// single server list entry
pub fn render(mut row: TableRow<'_, '_>, row_index: usize) {
    row.col(|ui| {
        ui.label("time".to_string());
    });
    row.col(|ui| {
        ui.label(format!("{row_index}"));
    });
    row.col(|ui| {
        ui.label("flag");
    });
}
