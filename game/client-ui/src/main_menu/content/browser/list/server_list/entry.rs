use egui_extras::TableRow;
use shared_base::server_browser::ServerBrowserServer;

use ui_base::utils::icon_font_text_for_text;

/// single server list entry
pub fn render(mut row: TableRow<'_, '_>, server: &ServerBrowserServer) -> bool {
    let mut clicked = false;
    clicked |= row
        .col(|ui| {
            clicked |= if server.info.passworded {
                ui.label(icon_font_text_for_text(ui, "\u{f023}"))
            } else {
                ui.label("")
            }
            .clicked();
        })
        .1
        .clicked();
    clicked |= row
        .col(|ui| {
            clicked |= ui.label(&server.info.name).clicked();
        })
        .1
        .clicked();
    clicked |= row
        .col(|ui| {
            clicked |= ui.label(&server.info.game_type).clicked();
        })
        .1
        .clicked();
    clicked |= row
        .col(|ui| {
            clicked |= ui.label(&server.info.map.name).clicked();
        })
        .1
        .clicked();
    clicked |= row
        .col(|ui| {
            clicked |= ui.label(server.info.players.len().to_string()).clicked();
        })
        .1
        .clicked();
    clicked |= row
        .col(|ui| {
            clicked |= ui.label("EU").clicked();
        })
        .1
        .clicked();
    clicked
}
