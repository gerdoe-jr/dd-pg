use std::path::PathBuf;

use egui::Color32;
use egui_extras::{Size, StripBuilder};
use ui_base::{
    types::UiRenderPipe,
    utils::{add_horizontal_margins, icon_font_plus_text},
};

use crate::{
    events::UiEvent,
    main_menu::{constants::MENU_DEMO_NAME, user_data::UserData},
};

pub fn render(
    ui: &mut egui::Ui,
    pipe: &mut UiRenderPipe<UserData>,
    cur_page: &str,
    main_frame_only: bool,
) {
    if cur_page == MENU_DEMO_NAME {
        StripBuilder::new(ui)
            .size(Size::remainder())
            .size(Size::exact(300.0))
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgba_unmultiplied(0, 0, 0, 100))
                        .rounding(5.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.set_height(ui.available_height());
                            if !main_frame_only {
                                add_horizontal_margins(ui, |ui| {
                                    StripBuilder::new(ui)
                                        .size(Size::exact(30.0))
                                        .size(Size::remainder())
                                        .size(Size::exact(30.0))
                                        .vertical(|mut strip| {
                                            strip.cell(|ui| {
                                                super::search::render(ui, pipe);
                                            });
                                            strip.cell(|ui| {
                                                super::list::render(ui, pipe);
                                            });
                                            strip.cell(|ui| {
                                                ui.horizontal(|ui| {
                                                    if ui.button("play").clicked() {
                                                        let cur_path: String = pipe
                                                            .user_data
                                                            .config
                                                            .storage("demo-path");
                                                        let cur_path: PathBuf = cur_path.into();
                                                        let name: String = pipe
                                                            .user_data
                                                            .config
                                                            .storage("selected-demo");

                                                        let new_path = cur_path.join(name);
                                                        pipe.user_data.events.push(
                                                            UiEvent::PlayDemo { name: new_path },
                                                        );
                                                    }
                                                    if ui.button("record").clicked() {
                                                        let cur_path: String = pipe
                                                            .user_data
                                                            .config
                                                            .storage("demo-path");
                                                        let cur_path: PathBuf = cur_path.into();
                                                        let name: String = pipe
                                                            .user_data
                                                            .config
                                                            .storage("selected-demo");

                                                        let new_path = cur_path.join(name);
                                                        pipe.user_data.events.push(
                                                            UiEvent::EncodeDemoToVideo {
                                                                name: new_path,
                                                            },
                                                        );
                                                    }
                                                });
                                            });
                                        });
                                });
                            }
                        });
                });
                strip.cell(|ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgba_unmultiplied(0, 0, 0, 100))
                        .rounding(5.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.set_height(ui.available_height());
                            if !main_frame_only {
                                StripBuilder::new(ui)
                                    .size(Size::exact(30.0))
                                    .size(Size::remainder())
                                    .vertical(|mut strip| {
                                        strip.cell(|ui| {
                                            ui.centered_and_justified(|ui| {
                                                ui.label(icon_font_plus_text(
                                                    ui,
                                                    "\u{f05a}",
                                                    "Demo information",
                                                ));
                                            });
                                        });
                                        strip.cell(|ui| {});
                                    });
                            }
                        });
                });
            });
    }
}
