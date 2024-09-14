use std::{collections::BTreeMap, time::Duration};

use game_interface::types::resource_key::NetworkResourceKey;
use ui_base::types::{UiRenderPipe, UiState};

use crate::{main_menu::user_data::UserData, utils::render_entities_for_ui};

pub fn entities_list(
    ui: &mut egui::Ui,
    pipe: &mut UiRenderPipe<UserData>,
    ui_state: &mut UiState,
    profile_index: usize,
) {
    let entries = pipe.user_data.entities_container.entries_index();
    let entries_sorted = entries.into_iter().collect::<BTreeMap<_, _>>();
    let player = &mut pipe.user_data.config.game.players[profile_index];
    let search_str = pipe
        .user_data
        .config
        .engine
        .ui
        .path
        .query
        .entry("entites-search".to_string())
        .or_default();
    let mut next_name = None;
    super::super::super::list::list::render(
        ui,
        entries_sorted.iter().map(|(name, &ty)| (name.as_str(), ty)),
        200.0,
        |_, name| {
            let valid: Result<NetworkResourceKey<24>, _> = name.try_into();
            valid.map(|_| ()).map_err(|err| err.into())
        },
        |_, name| player.entities == name,
        |ui, _, name, pos, asset_size| {
            render_entities_for_ui(
                pipe.user_data.canvas_handle,
                pipe.user_data.entities_container,
                pipe.user_data.map_render,
                pipe.user_data
                    .tile_set_preview
                    .buffer_object
                    .clone()
                    .unwrap(),
                ui,
                ui_state,
                pipe.user_data.full_rect,
                Some(ui.clip_rect()),
                &name.try_into().unwrap_or_default(),
                pos,
                asset_size,
            );
        },
        |_, name| {
            next_name = Some(name.to_string());
        },
        search_str,
        |_| {},
    );
    if let Some(next_name) = next_name.take() {
        player.entities = next_name;
        pipe.user_data
            .player_settings_sync
            .set_player_info_changed();
    }

    pipe.user_data.entities_container.update(
        &pipe.cur_time,
        &Duration::from_secs(10),
        &Duration::from_secs(1),
        [].iter(),
    );
}
