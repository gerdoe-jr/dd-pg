use egui::{Color32, Pos2, Response, Shape, Stroke};

use crate::utils::icon_font_text_for_btn;

pub struct MenuTopButtonProps {
    active: bool,
    text: String,
}

impl MenuTopButtonProps {
    pub fn new(text: &str, current_active: &Option<String>) -> Self {
        Self {
            active: Some(text).eq(&current_active.as_ref().map(|s| s.as_str())),
            text: text.to_string(),
        }
    }
}

#[must_use]
pub fn menu_top_button(ui: &mut egui::Ui, props: MenuTopButtonProps) -> Response {
    let res = ui.button(props.text);
    if props.active {
        ui.painter().add(Shape::line_segment(
            [
                Pos2::new(res.rect.left() + 8.0, res.rect.top() + 18.0),
                Pos2::new(
                    res.rect.left() + res.rect.width() - 8.0,
                    res.rect.top() + 18.0,
                ),
            ],
            Stroke::new(1.0, Color32::LIGHT_BLUE),
        ));
    }
    res
}

#[must_use]
pub fn menu_top_button_icon(ui: &mut egui::Ui, props: MenuTopButtonProps) -> Response {
    let res = ui.button(icon_font_text_for_btn(ui, &props.text));
    if props.active {
        ui.painter().add(Shape::line_segment(
            [
                Pos2::new(res.rect.left() + 8.0, 18.0),
                Pos2::new(res.rect.left() + res.rect.width() - 8.0, 18.0),
            ],
            Stroke::new(1.0, Color32::LIGHT_BLUE),
        ));
    }
    res
}
