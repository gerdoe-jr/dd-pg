use std::{str::FromStr, sync::Arc};

use base_io::io::Io;
use config::config::ConfigPath;
use ui_base::utils::icon_font_plus_text;

use crate::main_menu::{profiles_interface::ProfilesInterface, user_data::ProfileTasks};

use super::{back_bar::back_bar, constants::PROFILE_PAGE_QUERY};

/// overview
pub fn render(
    ui: &mut egui::Ui,
    accounts: &Arc<dyn ProfilesInterface>,
    tasks: &mut ProfileTasks,
    io: &Io,
    path: &mut ConfigPath,
) {
    back_bar(ui, "Login by email", path);
    ui.vertical_centered(|ui| {
        egui::Grid::new("login-email-token")
            .spacing([2.0, 4.0])
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Email:");

                let email = path.query.entry("email".into()).or_default();
                ui.text_edit_singleline(email);
                ui.end_row();
            });
        if let Some(url) = tasks.web_validations.first() {
            ui.label("A verification on this web page is needed:");
            ui.hyperlink(url);
            ui.label("Afterwards add the token from\nthe web page to this field:");
            egui::Grid::new("login-email-token-secret-key")
                .spacing([2.0, 4.0])
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Token:");
                    let veri_token = path.query.entry("veri-token".into()).or_default();
                    ui.text_edit_singleline(veri_token);
                    ui.end_row();
                });
        }

        if ui
            .button(icon_font_plus_text(
                ui,
                "\u{f2f6}",
                "Request token by email",
            ))
            .clicked()
        {
            if let (Some(email), veri_token) = (
                path.query
                    .get("email")
                    .and_then(|email| email_address::EmailAddress::from_str(email).ok()),
                path.query.get("veri-token"),
            ) {
                let accounts = accounts.clone();
                let veri_token = veri_token.cloned();
                tasks.login_tokens.push(
                    io.io_batcher
                        .spawn(
                            async move { Ok(accounts.login_email_token(email, veri_token).await) },
                        )
                        .abortable(),
                );
                path.route_query_only_single((
                    PROFILE_PAGE_QUERY.into(),
                    "login-email".to_string(),
                ));
            }
        }
    });
}
