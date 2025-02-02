use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    rc::Rc,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use base::{
    hash::Hash,
    reduced_ascii_str::ReducedAsciiString,
    system::{System, SystemTimeInterface},
};
use base_io::{io::Io, io_batcher::IoBatcherTask};
use binds::binds::{
    gen_local_player_action_hash_map, syn_to_bind, BindActions, BindActionsLocalPlayer,
};
use client_accounts::accounts::Accounts;
use client_console::console::remote_console::{RemoteConsole, RemoteConsoleBuilder};
use client_map::client_map::{ClientMapFile, ClientMapLoading, GameMap};
use client_render_game::render_game::{
    ObservedPlayer, RenderGameCreateOptions, RenderGameForPlayer,
};
use client_types::console::{entries_to_parser, ConsoleEntry};
use client_ui::{
    client_info::ClientInfo,
    connect::user_data::{ConnectMode, ConnectModes},
    ingame_menu::{
        account_info::AccountInfo,
        server_info::{GameInfo, GameServerInfo},
        votes::Votes,
    },
    main_menu::{player_settings_ntfy::PlayerSettingsSync, spatial_chat},
};
use command_parser::parser::{self, CommandType};
use config::config::ConfigEngine;
use demo::recorder::{DemoRecorder, DemoRecorderCreateProps};
use game_config::config::{ConfigDummyProfile, ConfigGame, ConfigPlayer};
use game_interface::{
    events::GameEvents,
    interface::GameStateCreateOptions,
    types::{
        character_info::{NetworkCharacterInfo, NetworkSkinInfo},
        game::{GameEntityId, GameTickType},
        input::CharacterPredictionInput,
        network_string::NetworkString,
        resource_key::NetworkResourceKey,
        snapshot::SnapshotLocalPlayers,
        weapons::WeaponType,
    },
    votes::{MapVote, VoteState, Voted},
};
use graphics::graphics::graphics::Graphics;
use graphics_backend::backend::GraphicsBackend;
use hashlink::{LinkedHashMap, LinkedHashSet};
use log::info;
use math::math::vector::{luffixed, vec2};
use native::{
    input::binds::{BindKey, MouseExtra},
    native::{KeyCode, MouseButton, PhysicalKey},
};
use network::network::{
    network::{NetworkClientCertCheckMode, NetworkClientCertMode, NetworkClientInitOptions},
    packet_compressor::DefaultNetworkPacketCompressor,
    plugins::{NetworkPluginPacket, NetworkPlugins},
    quinn_network::QuinnNetwork,
};
use pool::{
    datatypes::{PoolBTreeMap, PoolVec, PoolVecDeque, StringPool},
    mt_pool::Pool as MtPool,
    pool::Pool,
    rc::PoolRc,
};
use prediction_timer::prediction_timing::PredictionTimer;
use shared_base::{
    network::{
        messages::{
            GameModification, MsgClInputPlayerChain, MsgClReady, MsgClSnapshotAck,
            PlayerInputChainable,
        },
        server_info::ServerInfo,
        types::chat::NetChatMsg,
    },
    player_input::PlayerInput,
};
use shared_network::{
    game_event_generator::GameEventGenerator,
    messages::{
        ClientToServerMessage, ClientToServerPlayerMessage, GameMessage, ServerToClientMessage,
    },
};
use sound::{scene_object::SceneObject, sound::SoundManager};
use ui_base::{font_data::UiFontData, types::UiState, ui::UiCreator};
use url::Url;

use crate::localplayer::{ClientPlayer, LocalPlayers};

use super::{
    client::ClientPlayerInputPerTick,
    component::GameMsgPipeline,
    components::network_logic::NetworkLogic,
    input::input_handling::DeviceToLocalPlayerIndex,
    spatial_chat::spatial_chat::{SpatialChat, SpatialChatGameWorldTy},
};

#[derive(Debug, Default)]
pub struct NetworkByteStats {
    pub last_timestamp: Duration,
    pub last_bytes_sent: u64,
    pub last_bytes_recv: u64,
    pub bytes_per_sec_sent: luffixed,
    pub bytes_per_sec_recv: luffixed,
}

#[derive(Debug)]
pub struct SnapshotStorageItem {
    pub snapshot: Vec<u8>,
    pub monotonic_tick: u64,
}

#[derive(Debug, Clone)]
pub enum ServerCertMode {
    Cert(Vec<u8>),
    Hash(Hash),
}

/// Automatically reset some state if the client dropped.
///
/// Mostly some Ui stuff
#[derive(Debug)]
pub struct DisconnectAutoCleanup {
    pub spatial_chat: spatial_chat::SpatialChat,
    pub client_info: ClientInfo,
    pub account_info: AccountInfo,
    pub player_settings_sync: PlayerSettingsSync,
    pub votes: Votes,
}

impl Drop for DisconnectAutoCleanup {
    fn drop(&mut self) {
        self.spatial_chat.support(false);
        self.client_info.set_local_player_count(0);
        self.account_info.fill_account_info(None);
        self.player_settings_sync.did_player_info_change();
        self.player_settings_sync.did_controls_change();
        self.player_settings_sync.did_team_settings_change();
        self.votes.needs_map_votes();
        self.votes.fill_map_votes(Default::default());
    }
}

pub struct GameData {
    pub local_players: LocalPlayers,

    /// Snapshot that still has to be acknowledged.
    pub snap_acks: Vec<MsgClSnapshotAck>,

    pub device_to_local_player_index: DeviceToLocalPlayerIndex, // TODO: keyboard and mouse are different devices
    pub input_per_tick: ClientPlayerInputPerTick,

    /// This is only used to make sure old snapshots are not handled.
    pub handled_snap_id: Option<u64>,

    /// Ever increasing id for sending input packages.
    pub input_id: u64,

    /// last (few) snapshot diffs & id client used
    pub snap_storage: BTreeMap<u64, SnapshotStorageItem>,

    /// A tracker of sent inputs and their time
    /// used to evaluate the estimated RTT/ping.
    pub sent_input_ids: BTreeMap<u64, Duration>,

    pub prediction_timer: PredictionTimer,
    pub net_byte_stats: NetworkByteStats,

    pub last_game_tick: Duration,
    pub last_frame_time: Duration,
    pub intra_tick_time: Duration,

    pub chat_msgs_pool: Pool<VecDeque<NetChatMsg>>,
    pub chat_msgs: PoolVecDeque<NetChatMsg>,
    pub player_inp_pool: Pool<LinkedHashMap<GameEntityId, PlayerInput>>,
    pub player_snap_pool: Pool<Vec<u8>>,

    /// current vote in the game and the network timestamp when it arrived
    pub vote: Option<(PoolRc<VoteState>, Option<Voted>, Duration)>,

    pub map_votes: Vec<MapVote>,
}

impl GameData {
    fn new(cur_time: Duration, prediction_timer: PredictionTimer) -> Self {
        let chat_and_system_msgs_pool = Pool::with_capacity(2);
        Self {
            local_players: LocalPlayers::new(),

            snap_acks: Vec::with_capacity(16),

            input_id: 0,

            snap_storage: Default::default(),

            device_to_local_player_index: Default::default(),
            input_per_tick: Default::default(),

            sent_input_ids: Default::default(),

            handled_snap_id: None,
            prediction_timer,
            net_byte_stats: Default::default(),

            last_game_tick: cur_time,
            intra_tick_time: Duration::ZERO,
            last_frame_time: cur_time,

            chat_msgs: chat_and_system_msgs_pool.new(),
            chat_msgs_pool: chat_and_system_msgs_pool,
            player_inp_pool: Pool::with_capacity(64),
            player_snap_pool: Pool::with_capacity(2),

            vote: None,
            map_votes: Default::default(),
        }
    }
}

impl GameData {
    pub fn handle_local_players_from_snapshot(
        &mut self,
        config: &ConfigGame,
        console_entries: &[ConsoleEntry],
        local_players: SnapshotLocalPlayers,
    ) {
        self.local_players
            .retain_with_order(|player_id, _| local_players.contains_key(player_id));
        local_players.iter().for_each(|(id, snap_player)| {
            if !self.local_players.contains_key(id) {
                let mut local_player: ClientPlayer = ClientPlayer {
                    is_dummy: snap_player.is_dummy,
                    zoom: 1.0,
                    ..Default::default()
                };
                let binds = &mut local_player.binds;
                let map = gen_local_player_action_hash_map();

                if snap_player.is_dummy {
                    if let Some((player, dummy)) = config
                        .players
                        .get(config.profiles.main as usize)
                        .zip(config.players.get(config.profiles.dummy.index as usize))
                    {
                        let bind_player = if config.profiles.dummy.copy_binds_from_main {
                            player
                        } else {
                            dummy
                        };
                        for bind in &bind_player.binds {
                            let cmds = parser::parse(bind, &entries_to_parser(console_entries));
                            for cmd in &cmds {
                                if let CommandType::Full(cmd) = cmd {
                                    let (keys, actions) = syn_to_bind(&cmd.args, &map).unwrap();

                                    binds.register_bind(&keys, actions);
                                }
                            }
                        }
                    }
                } else if let Some(player) = config.players.get(config.profiles.main as usize) {
                    for bind in &player.binds {
                        let cmds = parser::parse(bind, &entries_to_parser(console_entries));
                        for cmd in &cmds {
                            if let CommandType::Full(cmd) = cmd {
                                let (keys, actions) = syn_to_bind(&cmd.args, &map).unwrap();

                                binds.register_bind(&keys, actions);
                            }
                        }
                    }
                }

                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::KeyA))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::MoveLeft)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::KeyD))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::MoveRight)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Space))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Jump)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Escape))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::OpenMenu)],
                );
                binds.register_bind(
                    &[BindKey::Mouse(MouseButton::Left)],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Fire)],
                );
                binds.register_bind(
                    &[BindKey::Mouse(MouseButton::Right)],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Hook)],
                );
                binds.register_bind(
                    &[BindKey::Extra(MouseExtra::WheelDown)],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::PrevWeapon)],
                );
                binds.register_bind(
                    &[BindKey::Extra(MouseExtra::WheelUp)],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::NextWeapon)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Digit1))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Weapon(
                        WeaponType::Hammer,
                    ))],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Digit2))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Weapon(
                        WeaponType::Gun,
                    ))],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Digit3))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Weapon(
                        WeaponType::Shotgun,
                    ))],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Digit4))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Weapon(
                        WeaponType::Grenade,
                    ))],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Digit5))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Weapon(
                        WeaponType::Laser,
                    ))],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::KeyG))],
                    vec![BindActions::LocalPlayer(
                        BindActionsLocalPlayer::ToggleDummyCopyMoves,
                    )],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Enter))],
                    vec![BindActions::LocalPlayer(
                        BindActionsLocalPlayer::ActivateChatInput,
                    )],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::KeyT))],
                    vec![BindActions::LocalPlayer(
                        BindActionsLocalPlayer::ActivateChatInput,
                    )],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::Tab))],
                    vec![BindActions::LocalPlayer(
                        BindActionsLocalPlayer::ShowScoreboard,
                    )],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::KeyU))],
                    vec![BindActions::LocalPlayer(
                        BindActionsLocalPlayer::ShowChatHistory,
                    )],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::ShiftLeft))],
                    vec![BindActions::LocalPlayer(
                        BindActionsLocalPlayer::ShowEmoteWheel,
                    )],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::KeyQ))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::Kill)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::F3))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::VoteYes)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::F4))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::VoteNo)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::NumpadSubtract))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::ZoomOut)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::NumpadAdd))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::ZoomIn)],
                );
                binds.register_bind(
                    &[BindKey::Key(PhysicalKey::Code(KeyCode::NumpadMultiply))],
                    vec![BindActions::LocalPlayer(BindActionsLocalPlayer::ZoomReset)],
                );
                self.local_players.insert(*id, local_player);
            }
            // sort
            if let Some(local_player) = self.local_players.to_back(id) {
                local_player.input_cam_mode = snap_player.input_cam_mode;
            }
        });
    }
}

pub struct ActiveGame {
    pub network_logic: NetworkLogic,
    pub network: QuinnNetwork,
    pub game_event_generator_client: Arc<GameEventGenerator>,
    pub has_new_events_client: Arc<AtomicBool>,

    pub map: GameMap,
    pub demo_recorder: Option<DemoRecorder>,

    pub demo_recorder_props: DemoRecorderCreateProps,

    pub game_data: GameData,

    pub events: PoolBTreeMap<GameTickType, (GameEvents, bool)>,

    pub map_votes_loaded: bool,

    pub render_players_pool: Pool<LinkedHashMap<GameEntityId, RenderGameForPlayer>>,
    pub pred_player_inputs_pool: Pool<LinkedHashMap<GameEntityId, CharacterPredictionInput>>,
    pub render_observers_pool: Pool<Vec<ObservedPlayer>>,

    pub player_inputs_pool: Pool<LinkedHashMap<GameEntityId, PoolVec<PlayerInputChainable>>>,
    pub player_inputs_chainable_pool: Pool<Vec<PlayerInputChainable>>,
    pub player_inputs_chain_pool: MtPool<LinkedHashMap<GameEntityId, MsgClInputPlayerChain>>,
    pub player_inputs_chain_data_pool: MtPool<Vec<u8>>,
    pub player_inputs_ser_helper_pool: Pool<Vec<u8>>,
    pub events_pool: Pool<BTreeMap<GameTickType, (GameEvents, bool)>>,
    pub player_ids_pool: Pool<LinkedHashSet<GameEntityId>>,

    addr: SocketAddr,

    pub remote_console: RemoteConsole,
    rcon_secret: Option<[u8; 32]>,

    pub requested_account_details: bool,

    pub spatial_world: SpatialChatGameWorldTy,
    auto_cleanup: DisconnectAutoCleanup,
    pub connect_info: ConnectMode,
}

pub struct PrepareConnectGame {
    connect_info: ConnectMode,
    cert: ServerCertMode,
    addr: SocketAddr,
    task: Option<IoBatcherTask<NetworkClientCertMode>>,
    dicts_task: IoBatcherTask<(Vec<u8>, Vec<u8>)>,
    rcon_secret: Option<[u8; 32]>,
    auto_cleanup: DisconnectAutoCleanup,
}

pub struct ConnectingGame {
    pub network: QuinnNetwork,
    pub game_event_generator_client: Arc<GameEventGenerator>,
    pub has_new_events_client: Arc<AtomicBool>,
    pub connect_info: ConnectMode,
    server_connect_time: Duration,
    addr: SocketAddr,
    rcon_secret: Option<[u8; 32]>,
    auto_cleanup: DisconnectAutoCleanup,
}

pub struct LoadingGame {
    pub network: QuinnNetwork,
    pub game_event_generator_client: Arc<GameEventGenerator>,
    pub has_new_events_client: Arc<AtomicBool>,
    map: ClientMapLoading,
    ping: Duration,
    prediction_timer: PredictionTimer,
    hint_start_camera_pos: vec2,
    addr: SocketAddr,
    pub demo_recorder_props: DemoRecorderCreateProps,
    rcon_secret: Option<[u8; 32]>,
    spatial_world: SpatialChatGameWorldTy,
    auto_cleanup: DisconnectAutoCleanup,
    pub connect_info: ConnectMode,
}

pub enum Game {
    /// the game is currently inactive, e.g. if the client
    /// is still in the main menu
    None,
    /// prepare to connect to a server
    /// e.g. load private key or whatever
    PrepareConnect(PrepareConnectGame),
    /// the game is connecting
    Connecting(ConnectingGame),
    /// the game is loading
    Loading(LoadingGame),
    WaitingForFirstSnapshot(Box<ActiveGame>),
    Active(Box<ActiveGame>),
}

impl Game {
    pub fn new(
        io: &Io,
        connect_info: &ConnectMode,
        cert: ServerCertMode,
        addr: SocketAddr,
        accounts: &Arc<Accounts>,
        rcon_secret: Option<[u8; 32]>,
        auto_cleanup: DisconnectAutoCleanup,
    ) -> anyhow::Result<Self> {
        let accounts = accounts.clone();
        let task = io.io_batcher.spawn(async move {
            let (game_key, cert, _) = accounts.connect_to_game_server().await;
            Ok(NetworkClientCertMode::FromCertAndPrivateKey {
                cert,
                private_key: game_key.private_key,
            })
        });

        let fs = io.fs.clone();
        let zstd_dicts = io.io_batcher.spawn(async move {
            let client_send = fs.read_file("dict/client_send".as_ref()).await;
            let server_send = fs.read_file("dict/server_send".as_ref()).await;

            Ok(client_send.and_then(|c| server_send.map(|s| (c, s)))?)
        });

        Ok(Self::PrepareConnect(PrepareConnectGame {
            connect_info: connect_info.clone(),
            cert,
            addr,
            task: Some(task),
            dicts_task: zstd_dicts,
            rcon_secret,
            auto_cleanup,
        }))
    }

    fn connect(
        connect_info: &ConnectMode,
        sys: &System,
        server_cert: &ServerCertMode,
        config: &ConfigEngine,
        addr: SocketAddr,
        cert: NetworkClientCertMode,
        dicts: Option<(Vec<u8>, Vec<u8>)>,
        rcon_secret: Option<[u8; 32]>,
        auto_cleanup: DisconnectAutoCleanup,
    ) -> Self {
        let has_new_events_client = Arc::new(AtomicBool::new(false));
        let game_event_generator_client = Arc::new(GameEventGenerator::new(
            has_new_events_client.clone(),
            sys.time.clone(),
        ));

        let mut packet_plugins: Vec<Arc<dyn NetworkPluginPacket>> = vec![];

        if let Some((client_send, server_send)) = dicts {
            packet_plugins.push(Arc::new(DefaultNetworkPacketCompressor::new_with_dict(
                client_send,
                server_send,
            )));
        } else {
            packet_plugins.push(Arc::new(DefaultNetworkPacketCompressor::new()));
        }

        let (network_client, _game_event_notifier) = QuinnNetwork::init_client(
            "0.0.0.0:0",
            game_event_generator_client.clone(),
            sys,
            NetworkClientInitOptions::new(
                if config.dbg.untrusted_cert {
                    NetworkClientCertCheckMode::DisableCheck
                } else {
                    match server_cert {
                        ServerCertMode::Cert(cert) => {
                            NetworkClientCertCheckMode::CheckByCert { cert: cert.into() }
                        }
                        ServerCertMode::Hash(hash) => {
                            NetworkClientCertCheckMode::CheckByPubKeyHash { hash }
                        }
                    }
                },
                cert,
            )
            //.with_ack_config(5, Duration::from_millis(50), 5 - 1)
            // since there are many packets, increase loss detection thresholds
            //.with_loss_detection_cfg(25, 2.0)
            .with_timeout(config.net.timeout),
            NetworkPlugins {
                packet_plugins: Arc::new(packet_plugins),
                connection_plugins: Default::default(),
            },
            &addr.to_string(),
        );

        Self::Connecting(ConnectingGame {
            network: network_client,
            game_event_generator_client,
            has_new_events_client,
            connect_info: connect_info.clone(),
            server_connect_time: sys.time_get_nanoseconds(),
            addr,
            rcon_secret,
            auto_cleanup,
        })
    }

    fn load(
        network: QuinnNetwork,
        game_event_generator_client: Arc<GameEventGenerator>,
        has_new_events_client: Arc<AtomicBool>,
        tp: &Arc<rayon::ThreadPool>,
        io: &Io,
        map: &ReducedAsciiString,
        map_blake3_hash: &Hash,
        game_mod: GameModification,
        timestamp: Duration,
        server_connect_time: Duration,
        hint_start_camera_pos: vec2,
        ui: &mut UiState,
        config: &mut ConfigEngine,
        addr: SocketAddr,
        game_options: GameStateCreateOptions,
        rcon_secret: Option<[u8; 32]>,
        props: RenderGameCreateOptions,
        spatial_world: SpatialChatGameWorldTy,
        auto_cleanup: DisconnectAutoCleanup,
        connect_info: ConnectMode,
    ) -> Self {
        info!("loading map: {}", map.as_str());
        let ping = timestamp.saturating_sub(server_connect_time);

        ui.is_ui_open = false;
        config.ui.path.route("ingame");

        let demo_recorder_props = DemoRecorderCreateProps {
            map: map.clone(),
            map_hash: *map_blake3_hash,
            game_options: game_options.clone(),
            required_resources: Default::default(), /* TODO: */
            physics_module: game_mod.clone(),
            render_module: GameModification::Native,
            io: io.clone(),
            physics_group_name: props.physics_group_name.clone(),
        };
        Self::Loading(LoadingGame {
            network,
            game_event_generator_client,
            has_new_events_client,
            map: ClientMapLoading::new(
                "map/maps".as_ref(),
                map,
                Some(*map_blake3_hash),
                io,
                tp,
                game_mod,
                false,
                game_options,
                props,
            ),
            ping,
            prediction_timer: PredictionTimer::new(ping, timestamp),
            hint_start_camera_pos,
            addr,
            demo_recorder_props,
            rcon_secret,
            spatial_world,
            auto_cleanup,
            connect_info,
        })
    }

    /// This
    pub fn network_char_info_from_config_for_dummy(
        player: &ConfigPlayer,
        copy_player: &ConfigPlayer,
        dummy_profile: &ConfigDummyProfile,
    ) -> NetworkCharacterInfo {
        let assets_player = if dummy_profile.copy_assets_from_main {
            copy_player
        } else {
            player
        };
        NetworkCharacterInfo {
            name: NetworkString::new(&player.name).unwrap(),
            clan: NetworkString::new(&player.clan).unwrap(),
            flag: NetworkString::new(player.flag.to_lowercase().replace("-", "_")).unwrap(),
            skin: NetworkResourceKey::from_str_lossy(&player.skin.name),

            skin_info: if player.skin.custom_colors {
                NetworkSkinInfo::Custom {
                    body_color: player.skin.body_color.into(),
                    feet_color: player.skin.feet_color.into(),
                }
            } else {
                NetworkSkinInfo::Original
            },

            weapon: NetworkResourceKey::from_str_lossy(&assets_player.weapon),
            freeze: NetworkResourceKey::from_str_lossy(&assets_player.freeze),
            ninja: NetworkResourceKey::from_str_lossy(&assets_player.ninja),
            game: NetworkResourceKey::from_str_lossy(&assets_player.game),
            ctf: NetworkResourceKey::from_str_lossy(&assets_player.ctf),
            hud: NetworkResourceKey::from_str_lossy(&assets_player.hud),
            entities: NetworkResourceKey::from_str_lossy(&assets_player.entities),
            emoticons: NetworkResourceKey::from_str_lossy(&assets_player.emoticons),
            particles: NetworkResourceKey::from_str_lossy(&assets_player.particles),
            hook: NetworkResourceKey::from_str_lossy(&assets_player.hook),
        }
    }

    pub fn network_char_info_from_config(p: &ConfigPlayer) -> NetworkCharacterInfo {
        Self::network_char_info_from_config_for_dummy(
            p,
            p,
            &ConfigDummyProfile {
                index: 0,
                copy_assets_from_main: false,
                copy_binds_from_main: false,
            },
        )
    }

    pub fn update(
        &mut self,
        graphics: &Graphics,
        graphics_backend: &Rc<GraphicsBackend>,
        sound: &SoundManager,
        config: &ConfigEngine,
        config_game: &ConfigGame,
        sys: &System,
        ui_creator: &UiCreator,
    ) {
        let mut selfi = Self::None;
        std::mem::swap(&mut selfi, self);
        *self = match selfi {
            Game::Active(mut game) => {
                // check msgs from ui
                if game
                    .auto_cleanup
                    .player_settings_sync
                    .did_player_info_change()
                {
                    for (local_player_id, local_player) in game.game_data.local_players.iter_mut() {
                        let character_info = if let Some((info, copy_info)) = local_player
                            .is_dummy
                            .then(|| {
                                config_game
                                    .players
                                    .get(config_game.profiles.dummy.index as usize)
                                    .zip(
                                        config_game.players.get(config_game.profiles.main as usize),
                                    )
                            })
                            .flatten()
                        {
                            Game::network_char_info_from_config_for_dummy(
                                info,
                                copy_info,
                                &config_game.profiles.dummy,
                            )
                        } else if let Some(p) =
                            config_game.players.get(config_game.profiles.main as usize)
                        {
                            // TODO: splitscreen support
                            Game::network_char_info_from_config(p)
                        } else {
                            NetworkCharacterInfo::explicit_default()
                        };
                        local_player.player_info_version += 1;
                        let version = local_player.player_info_version.try_into().unwrap();
                        game.network
                            .send_unordered_to_server(&GameMessage::ClientToServer(
                                ClientToServerMessage::PlayerMsg((
                                    *local_player_id,
                                    ClientToServerPlayerMessage::UpdateCharacterInfo {
                                        info: Box::new(character_info),
                                        version,
                                    },
                                )),
                            ))
                    }
                }

                Game::Active(game)
            }
            Game::None | Game::WaitingForFirstSnapshot(_) => {
                // nothing to do
                selfi
            }
            Game::Connecting(game) => Self::Connecting(game),
            Game::PrepareConnect(PrepareConnectGame {
                connect_info,
                cert,
                addr,
                task,
                dicts_task,
                rcon_secret,
                auto_cleanup,
            }) => {
                if !task.as_ref().is_some_and(|task| !task.is_finished())
                    && dicts_task.is_finished()
                {
                    Self::connect(
                        &connect_info,
                        sys,
                        &cert,
                        config,
                        addr,
                        task.map(|task| task.get_storage().unwrap()).unwrap(),
                        dicts_task.get_storage().ok(),
                        rcon_secret,
                        auto_cleanup,
                    )
                } else {
                    Game::PrepareConnect(PrepareConnectGame {
                        connect_info,
                        cert,
                        addr,
                        task,
                        dicts_task,
                        rcon_secret,
                        auto_cleanup,
                    })
                }
            }
            Game::Loading(LoadingGame {
                network,
                game_event_generator_client,
                has_new_events_client,
                mut map,
                ping,
                prediction_timer,
                hint_start_camera_pos,
                addr,
                demo_recorder_props,
                rcon_secret,
                spatial_world,
                auto_cleanup,
                connect_info,
            }) => {
                if map.is_fully_loaded() {
                    let player_info = if let Some(p) =
                        config_game.players.get(config_game.profiles.main as usize)
                    {
                        Self::network_char_info_from_config(p)
                    } else {
                        NetworkCharacterInfo::explicit_default()
                    };
                    network.send_unordered_to_server(&GameMessage::ClientToServer(
                        ClientToServerMessage::Ready(MsgClReady {
                            player_info,
                            rcon_secret,
                        }),
                    ));
                    let ClientMapLoading::Map(ClientMapFile::Game(map)) = map else {
                        panic!("remove this in future.")
                    };

                    let demo_recorder = DemoRecorder::new(
                        demo_recorder_props.clone(),
                        map.game.game_tick_speed(),
                        None,
                    );

                    let mut remote_console = RemoteConsoleBuilder::build(ui_creator);
                    remote_console.ui.ui_state.is_ui_open = false;

                    let events_pool = Pool::with_capacity(4);

                    Self::WaitingForFirstSnapshot(Box::new(ActiveGame {
                        network_logic: NetworkLogic::new(),
                        network,
                        game_event_generator_client,
                        has_new_events_client,
                        map,
                        demo_recorder: Some(demo_recorder),
                        demo_recorder_props,
                        game_data: GameData::new(sys.time_get_nanoseconds(), prediction_timer),

                        events: events_pool.new(),
                        map_votes_loaded: Default::default(),

                        render_players_pool: Pool::with_capacity(64),
                        pred_player_inputs_pool: Pool::with_capacity(2),
                        render_observers_pool: Pool::with_capacity(2),

                        player_inputs_pool: Pool::with_capacity(4),
                        player_inputs_chainable_pool: Pool::with_capacity(4),
                        player_inputs_chain_pool: MtPool::with_capacity(4),
                        player_inputs_chain_data_pool: MtPool::with_capacity(4),
                        player_inputs_ser_helper_pool: Pool::with_capacity(4),
                        events_pool,
                        player_ids_pool: Pool::with_capacity(4),

                        addr,

                        remote_console,
                        rcon_secret,

                        requested_account_details: false,

                        spatial_world,
                        auto_cleanup,
                        connect_info,
                    }))
                } else {
                    map.continue_loading(sound, graphics, graphics_backend, config, sys);
                    Self::Loading(LoadingGame {
                        network,
                        game_event_generator_client,
                        has_new_events_client,
                        map,
                        ping,
                        prediction_timer,
                        hint_start_camera_pos,
                        addr,
                        demo_recorder_props,
                        rcon_secret,
                        spatial_world,
                        auto_cleanup,
                        connect_info,
                    })
                }
            }
        }
    }

    pub fn on_msg(
        &mut self,
        timestamp: Duration,
        msg: ServerToClientMessage<'static>,
        sys: &System,
        tp: &Arc<rayon::ThreadPool>,
        io: &Io,
        ui: &mut UiState,
        config: &mut ConfigEngine,
        config_game: &mut ConfigGame,
        shared_info: &Arc<ServerInfo>,
        string_pool: &StringPool,
        console_entries: &Vec<ConsoleEntry>,
        game_server_info: &GameServerInfo,
        fonts: &Arc<UiFontData>,
        account_info: &AccountInfo,
        spatial_chat: &mut SpatialChat,
        spatial_chat_scene: &SceneObject,
    ) {
        let mut selfi = Self::None;
        std::mem::swap(&mut selfi, self);
        let mut is_waiting = matches!(&selfi, Game::WaitingForFirstSnapshot(_));
        match selfi {
            Game::None => {}
            Game::PrepareConnect(game) => {
                *self = Self::PrepareConnect(game);
            }
            Game::Connecting(connecting) => match msg {
                ServerToClientMessage::ServerInfo { info, overhead } => {
                    game_server_info.fill_game_info(GameInfo {
                        map_name: info.map.to_string(),
                    });
                    game_server_info.fill_server_options(info.server_options.clone());
                    spatial_chat.spatial_chat.support(info.spatial_chat);
                    *self = Self::load(
                        connecting.network,
                        connecting.game_event_generator_client,
                        connecting.has_new_events_client,
                        tp,
                        io,
                        &info.map,
                        &info.map_blake3_hash,
                        info.game_mod,
                        timestamp.saturating_sub(overhead),
                        connecting.server_connect_time,
                        info.hint_start_camera_pos,
                        ui,
                        config,
                        connecting.addr,
                        GameStateCreateOptions {
                            hint_max_characters: None, // TODO: get from server
                            config: info.mod_config,
                        },
                        connecting.rcon_secret,
                        RenderGameCreateOptions {
                            physics_group_name: info.server_options.physics_group_name,
                            resource_download_server: info.resource_server_fallback.map(|port| {
                                Url::try_from(
                                    format!("http://{}:{}", connecting.addr.ip(), port).as_str(),
                                )
                                .unwrap()
                            }),
                            fonts: fonts.clone(),
                            sound_props: Default::default(),
                        },
                        info.spatial_chat
                            .then(|| spatial_chat.create_world(spatial_chat_scene, config_game))
                            .unwrap_or(SpatialChatGameWorldTy::None),
                        connecting.auto_cleanup,
                        connecting.connect_info,
                    );
                }
                ServerToClientMessage::QueueInfo(info) => {
                    connecting
                        .connect_info
                        .set(ConnectModes::Queue { msg: info });
                    config.ui.path.route("connect");
                    *self = Self::Connecting(connecting);
                }
                _ => {
                    // collect msgs
                    *self = Self::Connecting(connecting);
                }
            },
            Game::Loading(loading) => {
                *self = Self::Loading(loading);
            }
            Game::WaitingForFirstSnapshot(mut game) | Game::Active(mut game) => {
                if let ServerToClientMessage::Load(info) = msg {
                    game_server_info.fill_game_info(GameInfo {
                        map_name: info.map.to_string(),
                    });
                    game_server_info.fill_server_options(info.server_options.clone());
                    spatial_chat.spatial_chat.support(info.spatial_chat);
                    *self = Self::load(
                        game.network,
                        game.game_event_generator_client,
                        game.has_new_events_client,
                        tp,
                        io,
                        &info.map,
                        &info.map_blake3_hash,
                        info.game_mod,
                        timestamp,
                        timestamp.saturating_sub(game.game_data.prediction_timer.ping_max()),
                        info.hint_start_camera_pos,
                        ui,
                        config,
                        game.addr,
                        GameStateCreateOptions {
                            hint_max_characters: None, // TODO: get from server
                            config: info.mod_config,
                        },
                        game.rcon_secret,
                        RenderGameCreateOptions {
                            physics_group_name: info.server_options.physics_group_name,
                            resource_download_server: info.resource_server_fallback.map(|port| {
                                format!("http://{}:{}", game.addr.ip(), port)
                                    .as_str()
                                    .try_into()
                                    .unwrap()
                            }),
                            fonts: fonts.clone(),
                            sound_props: Default::default(),
                        },
                        info.spatial_chat
                            .then(|| spatial_chat.create_world(spatial_chat_scene, config_game))
                            .unwrap_or(SpatialChatGameWorldTy::None),
                        game.auto_cleanup,
                        game.connect_info,
                    );
                } else {
                    if let ServerToClientMessage::Snapshot {
                        overhead_time,
                        game_monotonic_tick_diff,
                        diff_id,
                        ..
                    } = &msg
                    {
                        if is_waiting {
                            // set the first ping based on the intial packets,
                            // later prefer the network stats
                            let last_game_tick = sys.time_get_nanoseconds()
                                - *overhead_time
                                - game.game_data.prediction_timer.pred_max_smooth(
                                    Duration::from_nanos(
                                        (Duration::from_secs(1).as_nanos()
                                            / game.map.game.game_tick_speed().get() as u128)
                                            as u64,
                                    ),
                                );
                            game.game_data.last_game_tick = last_game_tick;

                            // set initial predicted game monotonic tick based on this first snapshot
                            game.map.game.predicted_game_monotonic_tick = diff_id
                                .and_then(|diff_id| {
                                    game.game_data
                                        .snap_storage
                                        .get(&diff_id)
                                        .map(|old| *game_monotonic_tick_diff + old.monotonic_tick)
                                })
                                .unwrap_or(*game_monotonic_tick_diff);

                            is_waiting = false;
                        }
                    }
                    game.network_logic.on_msg(
                        &timestamp,
                        msg,
                        &mut GameMsgPipeline {
                            demo_recorder: &mut game.demo_recorder,
                            network: &mut game.network,
                            runtime_thread_pool: tp,
                            io,
                            map: &mut game.map,
                            game_data: &mut game.game_data,
                            events: &mut game.events,
                            config,
                            config_game,
                            shared_info,
                            ui,
                            sys,
                            string_pool,
                            console_entries,
                            remote_console: &mut game.remote_console,
                            account_info,
                            spatial_world: game.spatial_world.as_mut(),
                            spatial_chat,
                        },
                    );

                    if is_waiting {
                        *self = Self::WaitingForFirstSnapshot(game);
                    } else {
                        *self = Self::Active(game);
                    }
                }
            }
        }
    }

    pub fn get_remote_console(&self) -> Option<&RemoteConsole> {
        if let Game::Active(game) = self {
            Some(&game.remote_console)
        } else {
            None
        }
    }
    pub fn get_remote_console_mut(&mut self) -> Option<&mut RemoteConsole> {
        if let Game::Active(game) = self {
            Some(&mut game.remote_console)
        } else {
            None
        }
    }
    pub fn remote_console_open(&self) -> bool {
        self.get_remote_console()
            .is_some_and(|c| c.ui.ui_state.is_ui_open)
    }
}
