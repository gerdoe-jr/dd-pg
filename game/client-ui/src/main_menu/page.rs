use std::{path::Path, sync::Arc};

use base_io::{io::Io, io_batcher::IoBatcherTask};
use base_io_traits::fs_traits::FileSystemEntryTy;
use client_containers::utils::{load_containers, RenderGameContainers};
use client_render_base::{
    map::{
        map_buffered::{ClientMapBuffered, TileLayerVisuals},
        map_pipeline::MapGraphics,
    },
    render::{tee::RenderTee, toolkit::ToolkitRender},
};
use shared_base::server_browser::{ServerBrowserData, ServerBrowserInfo, ServerBrowserServer};

use game_config::config::Config;
use graphics::{
    graphics::graphics::Graphics,
    handles::{
        backend::backend::GraphicsBackendHandle, canvas::canvas::GraphicsCanvasHandle,
        stream::stream::GraphicsStreamHandle,
    },
};
use master_server_types::{addr::Protocol, servers::BrowserServers};
use shared_base::network::server_info::ServerInfo;
use sound::sound::SoundManager;
use ui_base::types::{UiRenderPipe, UiState};
use ui_traits::traits::UiPageInterface;

use crate::{client_info::ClientInfo, events::UiEvents, main_menu::user_data::MainMenuInterface};

use super::{
    demo_list::{DemoList, DemoListEntry},
    main_frame,
    monitors::UiMonitors,
    player_settings_ntfy::PlayerSettingsSync,
    profiles_interface::ProfilesInterface,
    spatial_chat::SpatialChat,
    theme_container::{ThemeContainer, THEME_CONTAINER_PATH},
    user_data::{ProfileTasks, RenderOptions, UserData},
};

pub struct MainMenuIo {
    pub(crate) io: Io,
    cur_servers_task: Option<IoBatcherTask<String>>,
    cur_demos_task: Option<IoBatcherTask<DemoList>>,
}

impl MainMenuInterface for MainMenuIo {
    fn refresh(&mut self) {
        self.cur_servers_task = Some(MainMenuUi::req_server_list(&self.io));
    }

    fn refresh_demo_list(&mut self, path: &Path) {
        self.cur_demos_task = Some(MainMenuUi::req_demo_list(&self.io, path));
    }
}

pub struct MainMenuUi {
    pub(crate) server_info: Arc<ServerInfo>,
    pub(crate) client_info: ClientInfo,
    pub(crate) browser_data: ServerBrowserData,

    pub(crate) demos: DemoList,

    menu_io: MainMenuIo,
    io: Io,
    events: UiEvents,

    pub backend_handle: GraphicsBackendHandle,
    pub stream_handle: GraphicsStreamHandle,
    pub canvas_handle: GraphicsCanvasHandle,

    pub containers: RenderGameContainers,
    pub theme_container: ThemeContainer,

    pub render_tee: RenderTee,
    pub toolkit_render: ToolkitRender,
    pub map_render: MapGraphics,
    pub tile_layer_visuals: TileLayerVisuals,

    pub profiles: Arc<dyn ProfilesInterface>,
    pub profile_tasks: ProfileTasks,

    pub monitors: UiMonitors,
    spatial_chat: SpatialChat,
    player_settings_sync: PlayerSettingsSync,
}

impl MainMenuUi {
    fn req_demo_list(io: &Io, path: &Path) -> IoBatcherTask<DemoList> {
        let fs = io.fs.clone();
        let path = path.to_path_buf();
        io.io_batcher
            .spawn(async move {
                Ok(fs
                    .entries_in_dir(&path)
                    .await?
                    .into_iter()
                    .map(|(f, ty)| match ty {
                        FileSystemEntryTy::File { date } => DemoListEntry::File { name: f, date },
                        FileSystemEntryTy::Directory => DemoListEntry::Directory { name: f },
                    })
                    .collect())
            })
            .cancelable()
    }

    fn req_server_list(io: &Io) -> IoBatcherTask<String> {
        let http = io.http.clone();
        io.io_batcher
            .spawn(async move {
                Ok(http
                    .download_text(
                        "https://pg.ddnet.org:4444/ddnet/15/servers.json"
                            .try_into()
                            .unwrap(),
                    )
                    .await?)
            })
            .cancelable()
    }

    pub fn new(
        graphics: &Graphics,
        sound: &SoundManager,
        server_info: Arc<ServerInfo>,
        client_info: ClientInfo,
        events: UiEvents,
        io: Io,
        tp: Arc<rayon::ThreadPool>,
        profiles: Arc<dyn ProfilesInterface>,
        monitors: UiMonitors,
        spatial_chat: SpatialChat,
        player_settings_sync: PlayerSettingsSync,
    ) -> Self {
        let cur_servers_task = Self::req_server_list(&io);

        let mut profile_tasks: ProfileTasks = Default::default();
        let profiles_task = profiles.clone();
        profile_tasks.user_interactions.push(
            io.io_batcher
                .spawn(async move { profiles_task.user_interaction().await })
                .cancelable(),
        );

        let scene = sound.scene_handle.create(Default::default());

        Self {
            server_info,
            client_info,

            browser_data: ServerBrowserData {
                servers: Vec::new(),
            },
            demos: DemoList::default(),

            menu_io: MainMenuIo {
                io: io.clone(),
                cur_servers_task: Some(cur_servers_task),
                cur_demos_task: None,
            },
            io: io.clone(),
            events,

            backend_handle: graphics.backend_handle.clone(),
            stream_handle: graphics.stream_handle.clone(),
            canvas_handle: graphics.canvas_handle.clone(),
            render_tee: RenderTee::new(graphics),
            toolkit_render: ToolkitRender::new(graphics),
            containers: load_containers(&io, &tp, None, None, graphics, sound, &scene),
            theme_container: {
                let default_theme: IoBatcherTask<
                    client_containers::container::ContainerLoadedItem,
                > = ThemeContainer::load_default(&io, THEME_CONTAINER_PATH.as_ref());
                ThemeContainer::new(
                    io.clone(),
                    tp.clone(),
                    default_theme,
                    None,
                    None,
                    "theme-container",
                    graphics,
                    sound,
                    &scene,
                    THEME_CONTAINER_PATH.as_ref(),
                )
            },
            map_render: MapGraphics::new(&graphics.backend_handle),
            tile_layer_visuals: ClientMapBuffered::tile_set_preview(
                &graphics.get_graphics_mt(),
                &graphics.buffer_object_handle,
                &graphics.backend_handle,
            ),

            profiles,
            profile_tasks,
            monitors,
            spatial_chat,
            player_settings_sync,
        }
    }

    pub(crate) fn get_user_data<'a>(
        &'a mut self,
        config: &'a mut Config,
        hide_buttons_right: bool,
        ui: &egui::Ui,
    ) -> UserData<'a> {
        UserData {
            server_info: &self.server_info,
            client_info: &self.client_info,

            browser_data: &mut self.browser_data,
            demos: &self.demos,

            render_options: RenderOptions {
                hide_buttons_icons: hide_buttons_right,
            },

            main_menu: &mut self.menu_io,
            config,
            events: &self.events,
            backend_handle: &self.backend_handle,
            stream_handle: &self.stream_handle,
            canvas_handle: &self.canvas_handle,
            render_tee: &self.render_tee,
            skin_container: &mut self.containers.skin_container,
            flags_container: &mut self.containers.flags_container,

            toolkit_render: &self.toolkit_render,
            weapons_container: &mut self.containers.weapon_container,
            hook_container: &mut self.containers.hook_container,
            entities_container: &mut self.containers.entities_container,
            freeze_container: &mut self.containers.freeze_container,
            emoticons_container: &mut self.containers.emoticons_container,
            particles_container: &mut self.containers.particles_container,
            ninja_container: &mut self.containers.ninja_container,
            game_container: &mut self.containers.game_container,
            hud_container: &mut self.containers.hud_container,
            ctf_container: &mut self.containers.ctf_container,
            theme_container: &mut self.theme_container,

            map_render: &self.map_render,
            tile_set_preview: &self.tile_layer_visuals,

            spatial_chat: &self.spatial_chat,
            player_settings_sync: &self.player_settings_sync,

            full_rect: ui.available_rect_before_wrap(),
            profiles: &self.profiles,
            profile_tasks: &mut self.profile_tasks,
            io: &self.io,
            monitors: &self.monitors,
        }
    }

    pub fn json_to_server_browser(servers_raw: &str) -> Vec<ServerBrowserServer> {
        let servers: BrowserServers = match serde_json::from_str(servers_raw) {
            Ok(servers) => servers,
            Err(err) => {
                log::error!("could not parse servers json: {err}");
                return Default::default();
            }
        };

        let parsed_servers: Vec<ServerBrowserServer> = servers
            .servers
            .into_iter()
            .filter_map(|server| {
                if let Some(addr) = server
                    .addresses
                    .iter()
                    .find(|addr| addr.protocol == Protocol::VPg)
                {
                    let info: serde_json::Result<ServerBrowserInfo> =
                        serde_json::from_str(server.info.get());
                    match info {
                        Ok(info) => Some(ServerBrowserServer {
                            address: addr.ip.to_string() + ":" + &addr.port.to_string(),
                            info,
                        }),
                        Err(err) => {
                            log::error!("ServerBrowserInfo could not be parsed: {err}");
                            None
                        }
                    }
                } else {
                    None
                }
            })
            .collect();
        parsed_servers
    }

    pub fn check_tasks(&mut self) {
        if let Some(server_task) = &self.menu_io.cur_servers_task {
            if server_task.is_finished() {
                match self.menu_io.cur_servers_task.take().unwrap().get_storage() {
                    Ok(servers_raw) => {
                        self.browser_data.servers = Self::json_to_server_browser(&servers_raw);
                    }
                    Err(err) => {
                        log::error!("failed to download master server list: {err}");
                    }
                }
            }
        }
        if let Some(task) = &self.menu_io.cur_demos_task {
            if task.is_finished() {
                match self.menu_io.cur_demos_task.take().unwrap().get_storage() {
                    Ok(demos) => {
                        self.demos = demos;
                    }
                    Err(err) => {
                        log::error!("failed to get demo list: {err}");
                    }
                }
            }
        }
    }
}

impl UiPageInterface<Config> for MainMenuUi {
    fn has_blur(&self) -> bool {
        true
    }

    fn render_main_frame(
        &mut self,
        ui: &mut egui::Ui,
        pipe: &mut UiRenderPipe<Config>,
        ui_state: &mut UiState,
    ) {
        main_frame::render(
            ui,
            &mut UiRenderPipe {
                cur_time: pipe.cur_time,
                user_data: &mut self.get_user_data(pipe.user_data, false, ui),
            },
            ui_state,
            true,
        );
    }

    fn render(
        &mut self,
        ui: &mut egui::Ui,
        pipe: &mut UiRenderPipe<Config>,
        ui_state: &mut UiState,
    ) {
        self.check_tasks();

        main_frame::render(
            ui,
            &mut UiRenderPipe {
                cur_time: pipe.cur_time,
                user_data: &mut self.get_user_data(pipe.user_data, false, ui),
            },
            ui_state,
            false,
        )
    }

    fn unmount(&mut self) {
        self.containers.clear_except_default();
        self.profile_tasks = Default::default();
        self.menu_io.cur_servers_task = None;
    }
}
