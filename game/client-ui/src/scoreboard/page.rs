use ui_base::types::{UiRenderPipe, UiState};
use ui_traits::traits::UiPageInterface;

use super::{main_frame, user_data::UserData};

pub struct ScoreboardUi {}

impl Default for ScoreboardUi {
    fn default() -> Self {
        Self::new()
    }
}

impl ScoreboardUi {
    pub fn new() -> Self {
        Self {}
    }
}

impl<'a> UiPageInterface<UserData<'a>> for ScoreboardUi {
    fn has_blur(&self) -> bool {
        true
    }

    fn render_main_frame(
        &mut self,
        ui: &mut egui::Ui,
        pipe: &mut UiRenderPipe<UserData>,
        ui_state: &mut UiState,
    ) {
        main_frame::render(ui, pipe, ui_state, true);
    }

    fn render(
        &mut self,
        ui: &mut egui::Ui,
        pipe: &mut UiRenderPipe<UserData>,
        ui_state: &mut UiState,
    ) {
        main_frame::render(ui, pipe, ui_state, false)
    }
}
