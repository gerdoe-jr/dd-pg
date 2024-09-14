use std::{path::Path, sync::Arc};

use base_io::{io::Io, io_batcher::IoBatcherTask};
use client_containers::{
    ctf::CtfContainer, emoticons::EmoticonsContainer, entities::EntitiesContainer,
    flags::FlagsContainer, freezes::FreezeContainer, game::GameContainer, hooks::HookContainer,
    hud::HudContainer, ninja::NinjaContainer, particles::ParticlesContainer, skins::SkinContainer,
    weapons::WeaponContainer,
};
use client_render_base::{
    map::{map_buffered::TileLayerVisuals, map_pipeline::MapGraphics},
    render::{tee::RenderTee, toolkit::ToolkitRender},
};
use game_config::config::Config;
use graphics::handles::{
    backend::backend::GraphicsBackendHandle, canvas::canvas::GraphicsCanvasHandle,
    stream::stream::GraphicsStreamHandle,
};
use shared_base::network::server_info::ServerInfo;
use shared_base::server_browser::ServerBrowserData;
use url::Url;

use crate::{client_info::ClientInfo, events::UiEvents};

use super::{
    demo_list::DemoList,
    monitors::UiMonitors,
    player_settings_ntfy::PlayerSettingsSync,
    profiles_interface::{LoginTokenError, ProfilesInterface},
    spatial_chat::SpatialChat,
    theme_container::ThemeContainer,
};

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub hide_buttons_icons: bool,
}

pub trait MainMenuInterface {
    fn refresh(&mut self);

    fn refresh_demo_list(&mut self, path: &Path);
}

#[derive(Debug, Default)]
pub struct ProfileTasks {
    pub login_tokens: Vec<IoBatcherTask<Result<(), LoginTokenError>>>,
    pub logins: Vec<IoBatcherTask<()>>,
    pub user_interactions: Vec<IoBatcherTask<()>>,
    pub errors: Vec<String>,

    pub web_validations: Vec<Url>,
}

impl ProfileTasks {
    pub fn update(&mut self) {
        fn handle_task<T>(errors: &mut Vec<String>, tasks: &mut Vec<IoBatcherTask<T>>) -> Vec<T> {
            let login = std::mem::take(tasks);
            let mut res = Vec::new();
            for login in login.into_iter() {
                if login.is_finished() {
                    match login.get_storage() {
                        Ok(t) => {
                            res.push(t);
                        }
                        Err(err) => {
                            errors.push(err.to_string());
                        }
                    }
                } else {
                    tasks.push(login);
                }
            }
            res
        }
        let token_results = handle_task(&mut self.errors, &mut self.login_tokens);
        for token_result in token_results {
            match token_result {
                Ok(_) => {
                    // ignore
                }
                Err(err) => match err {
                    LoginTokenError::WebValidationProcessNeeded { url } => {
                        self.web_validations.push(url)
                    }
                    LoginTokenError::Other(err) => {
                        self.errors.push(err.to_string());
                    }
                },
            }
        }
        handle_task(&mut self.errors, &mut self.logins);
        handle_task(&mut self.errors, &mut self.user_interactions);
    }
}

pub struct UserData<'a> {
    pub browser_data: &'a mut ServerBrowserData,
    pub server_info: &'a Arc<ServerInfo>,

    pub demos: &'a DemoList,

    pub render_options: RenderOptions,

    pub main_menu: &'a mut dyn MainMenuInterface,

    pub config: &'a mut Config,

    pub events: &'a UiEvents,
    pub client_info: &'a ClientInfo,

    pub spatial_chat: &'a SpatialChat,
    pub player_settings_sync: &'a PlayerSettingsSync,

    pub backend_handle: &'a GraphicsBackendHandle,
    pub stream_handle: &'a GraphicsStreamHandle,
    pub canvas_handle: &'a GraphicsCanvasHandle,
    pub skin_container: &'a mut SkinContainer,
    pub render_tee: &'a RenderTee,
    pub flags_container: &'a mut FlagsContainer,
    pub toolkit_render: &'a ToolkitRender,
    pub weapons_container: &'a mut WeaponContainer,
    pub hook_container: &'a mut HookContainer,
    pub entities_container: &'a mut EntitiesContainer,
    pub freeze_container: &'a mut FreezeContainer,
    pub emoticons_container: &'a mut EmoticonsContainer,
    pub particles_container: &'a mut ParticlesContainer,
    pub ninja_container: &'a mut NinjaContainer,
    pub game_container: &'a mut GameContainer,
    pub hud_container: &'a mut HudContainer,
    pub ctf_container: &'a mut CtfContainer,
    pub theme_container: &'a mut ThemeContainer,

    pub map_render: &'a MapGraphics,
    pub tile_set_preview: &'a TileLayerVisuals,

    pub profiles: &'a Arc<dyn ProfilesInterface>,
    pub profile_tasks: &'a mut ProfileTasks,
    pub io: &'a Io,

    pub full_rect: egui::Rect,

    pub monitors: &'a UiMonitors,
}
