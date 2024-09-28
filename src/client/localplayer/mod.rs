use std::{collections::BTreeMap, time::Duration};

use binds::binds::BindActions;
use client_ui::emote_wheel::user_data::EmoteWheelEvent;
use game_interface::types::{
    game::{GameEntityId, GameTickType},
    render::character::PlayerCameraMode,
};
use hashlink::LinkedHashMap;
use math::math::vector::dvec2;
use native::input::binds::Binds;
use shared_base::{network::messages::PlayerInputChainable, player_input::PlayerInput};

#[derive(Debug)]
pub struct ServerInputForDiff {
    pub id: u64,
    pub inp: PlayerInputChainable,
}

#[derive(Debug, Default)]
pub struct ClientPlayer {
    pub input: PlayerInput,
    pub sent_input: PlayerInput,
    pub sent_input_time: Option<Duration>,
    /// The game tick the input was sent in
    pub sent_inp_tick: GameTickType,

    pub binds: Binds<Vec<BindActions>>,

    pub chat_input_active: bool,
    pub chat_msg: String,

    /// show a longer chat history
    pub show_chat_all: bool,
    pub show_scoreboard: bool,

    pub emote_wheel_active: bool,
    pub last_emote_wheel_selection: Option<EmoteWheelEvent>,

    // dummy controls
    pub dummy_copy_moves: bool,
    pub dummy_hammer: bool,

    /// For updating the player info on the server.
    pub player_info_version: u64,

    /// last input the server knows about
    pub server_input: Option<ServerInputForDiff>,
    /// inputs the client still knows about,
    /// [`PlayerInputChainable`] here is always the last of a chain that is send.
    pub server_input_storage: BTreeMap<u64, PlayerInputChainable>,

    pub is_dummy: bool,

    pub zoom: f32,

    pub input_cam_mode: PlayerCameraMode,
    pub free_cam_pos: dvec2,
    pub cursor_pos: dvec2,
}

pub type LocalPlayers = LinkedHashMap<GameEntityId, ClientPlayer>;
