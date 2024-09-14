use egui::{Align, Layout};
use egui_extras::{Size, StripBuilder};

use ui_base::{types::UiRenderPipe, utils::icon_font_text_for_btn};

use crate::main_menu::user_data::UserData;

/// button & popover
pub fn render(ui: &mut egui::Ui, pipe: &mut UiRenderPipe<UserData>) {
    let search_width = if ui.available_width() < 350.0 {
        150.0
    } else {
        250.0
    };
    let extra_space = 0.0;
    StripBuilder::new(ui)
        .size(Size::exact(extra_space))
        .size(Size::exact(30.0))
        .size(Size::remainder().at_least(search_width))
        .size(Size::exact(30.0))
        .size(Size::exact(extra_space))
        .horizontal(|mut strip| {
            strip.empty();
            strip.cell(|ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.button(icon_font_text_for_btn(ui, "\u{f0c9}"));
                });
            });
            strip.cell(|ui| {
                StripBuilder::new(ui)
                    .size(Size::remainder())
                    .size(Size::exact(search_width))
                    .size(Size::remainder())
                    .horizontal(|mut strip| {
                        strip.empty();
                        strip.cell(|ui| {
                            ui.with_layout(
                                Layout::left_to_right(Align::Center).with_main_justify(true),
                                |ui| {
                                    super::search::render(ui, pipe);
                                },
                            );
                        });
                        strip.empty();
                    });
            });
            strip.cell(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.button(icon_font_text_for_btn(ui, "\u{f0b0}"));
                });
            });
            strip.empty();
        });
}
