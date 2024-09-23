pub mod state {
    use std::fmt::Write;
    use std::num::{NonZero, NonZeroU16, NonZeroU64};
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Duration;

    use accounts_types::account_id::AccountId;
    use base::hash::Hash;
    use base_io::io_batcher::{IoBatcher, IoBatcherTask};
    use command_parser::parser::CommandType;
    use game_database::traits::DbInterface;
    use game_interface::chat_commands::ChatCommands;
    use game_interface::client_commands::ClientCommand;
    use game_interface::events::{
        EventClientInfo, EventId, EventIdGenerator, GameBuffEvent, GameBuffNinjaEvent,
        GameBuffNinjaEventSound, GameCharacterEvent, GameEvents, GameFlagEvent, GameGrenadeEvent,
        GameGrenadeEventSound, GameLaserEvent, GameLaserEventSound, GamePickupArmorEvent,
        GamePickupArmorEventSound, GamePickupEvent, GamePickupHeartEvent,
        GamePickupHeartEventSound, GameShotgunEvent, GameShotgunEventSound, GameWorldAction,
        GameWorldEntityEvent, GameWorldEvent, GameWorldEvents, GameWorldGlobalEvent,
        GameWorldPositionedEvent, GameWorldSystemMessage, KillFlags,
    };
    use game_interface::pooling::GamePooling;
    use game_interface::rcon_commands::{AuthLevel, RconCommands};
    use game_interface::types::character_info::{NetworkCharacterInfo, NetworkSkinInfo};
    use game_interface::types::emoticons::EmoticonType;
    use game_interface::types::game::{GameEntityId, GameTickCooldown, GameTickType};
    use game_interface::types::id_gen::IdGenerator;
    use game_interface::types::input::{
        CharacterInput, CharacterInputConsumableDiff, CharacterPredictionInput,
    };
    use game_interface::types::network_stats::PlayerNetworkStats;
    use game_interface::types::pickup::PickupType;
    use game_interface::types::player_info::{PlayerClientInfo, PlayerDropReason, PlayerUniqueId};
    use game_interface::types::render::game::game_match::{MatchSide, MatchStandings};
    use game_interface::types::render::game::GameRenderInfo;
    use game_interface::types::render::stage::StageRenderInfo;
    use game_interface::types::render::world::WorldRenderInfo;
    use game_interface::types::weapons::WeaponType;
    use hashlink::LinkedHashMap;
    use hiarc::hi_closure;
    use map::map::Map;
    use math::math::vector::{ubvec4, vec2};
    use pool::datatypes::PoolLinkedHashMap;
    use pool::mt_datatypes::{PoolCow as MtPoolCow, PoolLinkedHashMap as MtPoolLinkedHashMap};
    use pool::pool::Pool;

    use game_interface::interface::{
        GameStateCreate, GameStateCreateOptions, GameStateInterface, GameStateServerOptions,
        GameStateStaticInfo,
    };
    use game_interface::types::render::character::{
        CharacterBuff, CharacterBuffInfo, CharacterDebuff, CharacterDebuffInfo, CharacterInfo,
        CharacterPlayerInfo, CharacterRenderInfo, LocalCharacterRenderInfo, PlayerCameraMode,
        TeeEye,
    };
    use game_interface::types::render::flag::FlagRenderInfo;
    use game_interface::types::render::laser::LaserRenderInfo;
    use game_interface::types::render::pickup::PickupRenderInfo;
    use game_interface::types::render::projectiles::ProjectileRenderInfo;
    use game_interface::types::render::scoreboard::{
        Scoreboard, ScoreboardCharacterInfo, ScoreboardConnectionType, ScoreboardGameOptions,
        ScoreboardGameType, ScoreboardPlayerSpectatorInfo, ScoreboardStageInfo,
    };
    use game_interface::types::snapshot::{SnapshotClientInfo, SnapshotLocalPlayers};
    use pool::rc::PoolRc;
    use shared_base::mapdef_06::EEntityTiles;

    use crate::collision::collision::Tunings;
    use crate::config::{ConfigGameType, ConfigVanilla};
    use crate::entities::character::character::{self, CharacterPlayerTy};
    use crate::entities::character::player::player::{
        NoCharPlayer, NoCharPlayerType, NoCharPlayers, Player, PlayerInfo, Players,
    };
    use crate::entities::flag::flag::{Flag, Flags};
    use crate::entities::laser::laser::Laser;
    use crate::entities::pickup::pickup::Pickup;
    use crate::entities::projectile::projectile::{self};
    use crate::events::events::{
        CharacterEvent, FlagEvent, LaserEvent, PickupEvent, ProjectileEvent,
    };
    use crate::game_objects::game_objects::GameObjectDefinitions;
    use crate::match_state::match_state::{MatchState, MatchType};
    use crate::simulation_pipe::simulation_pipe::{
        SimulationEventWorldEntityType, SimulationEvents, SimulationWorldEvent,
        SimulationWorldEvents,
    };
    use crate::snapshot::snapshot::{Snapshot, SnapshotFor, SnapshotManager, SnapshotStage};
    use crate::sql::account_info::{AccountInfo, StatementResult};
    use crate::sql::setup_ddnet;
    use crate::stage::stage::Stages;
    use crate::types::types::{GameOptions, GameType};
    use crate::weapons::definitions::weapon_def::Weapon;

    use super::super::{
        collision::collision::Collision, entities::character::character::Character,
        simulation_pipe::simulation_pipe::SimulationPipeStage, spawns::GameSpawns,
        stage::stage::GameStage, world::world::WorldPool,
    };

    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum GameError {
        #[error("Stage ID was not found")]
        InvalidStage,
    }

    pub(crate) const TICKS_PER_SECOND: u64 = 50;

    pub struct Game {
        pub(crate) stages: Stages,

        pub players: Players,
        pub no_char_players: NoCharPlayers,

        pub timeout_players:
            LinkedHashMap<(PlayerUniqueId, usize), (GameEntityId, GameTickCooldown)>,
    }

    #[derive(Debug)]
    pub enum GameDbQueries {
        AccountInfo {
            player_id: GameEntityId,
            account_info: StatementResult,
        },
    }

    pub struct GameDb {
        pub(crate) io_batcher: IoBatcher,
        pub(crate) account_info: Option<AccountInfo>,

        pub(crate) cur_queries: Vec<IoBatcherTask<GameDbQueries>>,
        pub(crate) cur_queries_helper: Vec<IoBatcherTask<GameDbQueries>>,
    }

    /// A game state is a collection of game related attributes such as the world,
    /// which handles the entities,
    /// the current tick, the starting tick, if the game is paused,
    /// the stages of the game etc.
    pub struct GameState {
        pub(crate) game: Game,
        pub(crate) pred_game: Game,

        pub(crate) id_generator: IdGenerator,
        pub(crate) event_id_generator: EventIdGenerator,

        pub simulation_events: SimulationEvents,

        // only useful for server
        pub stage_0_id: GameEntityId,

        // physics
        pub(crate) collision: Collision,
        spawns: GameSpawns,
        pub(crate) game_objects_definitions: Rc<GameObjectDefinitions>,
        /// empty definitions for prediction
        pub(crate) pred_game_objects_definitions: Rc<GameObjectDefinitions>,

        // game
        pub(crate) game_options: GameOptions,
        config: ConfigVanilla,

        pub(crate) chat_commands: ChatCommands,
        pub(crate) rcon_commands: RconCommands,
        map_name: String,

        // db
        game_db: GameDb,

        // pooling
        pub(crate) world_pool: WorldPool,
        pub(crate) no_char_player_clone_pool: Pool<LinkedHashMap<GameEntityId, NoCharPlayer>>,
        player_clone_pool: Pool<Vec<(GameEntityId, Player)>>,
        game_pools: GamePooling,

        // snapshot
        pub(crate) snap_shot_manager: SnapshotManager,
    }

    impl GameStateCreate for GameState {
        fn new(
            map: Vec<u8>,
            map_name: String,
            options: GameStateCreateOptions,
            io_batcher: IoBatcher,
            db: Arc<dyn DbInterface>,
        ) -> (Self, GameStateStaticInfo)
        where
            Self: Sized,
        {
            let db_task = io_batcher.spawn(async move {
                setup_ddnet::setup(db.clone()).await?;

                let acc_info = AccountInfo::new(db).await;
                if let Err(err) = &acc_info {
                    log::warn!(
                        target: "sql", 
                        "failed to prepare account info sql: {}", err);
                }
                acc_info
            });

            let physics_group = Map::read_physics_group(&map).unwrap();

            let w = physics_group.attr.width.get() as u32;
            let h = physics_group.attr.height.get() as u32;

            let tiles = physics_group.get_game_layer_tiles();
            let tune_tiles = physics_group.get_tune_layer_tiles();
            let collision = Collision::new(
                w,
                h,
                tiles,
                tune_tiles.map(|tune_tiles| {
                    (
                        {
                            let mut tune_zones = vec![Tunings::default(); 256];
                            // TODO: actually implement, not just for fun
                            tune_zones[1] = Tunings {
                                grenade_curvature: -70.0,
                                ..Default::default()
                            };
                            tune_zones
                        },
                        tune_tiles.as_slice(),
                    )
                }),
            );
            let game_objects = GameObjectDefinitions::new(tiles, w, h);

            let mut spawns: Vec<vec2> = Default::default();
            let mut spawns_red: Vec<vec2> = Default::default();
            let mut spawns_blue: Vec<vec2> = Default::default();
            tiles.iter().enumerate().for_each(|(index, tile)| {
                let x = index % w as usize;
                let y = index / w as usize;
                let pos = vec2::new(x as f32 * 32.0 + 16.0, y as f32 * 32.0 + 16.0);
                if tile.index == EEntityTiles::Spawn as u8 {
                    spawns.push(pos);
                } else if tile.index == EEntityTiles::SpawnRed as u8 {
                    spawns_red.push(pos);
                } else if tile.index == EEntityTiles::SpawnBlue as u8 {
                    spawns_blue.push(pos);
                }
            });
            let id_generator = IdGenerator::new();

            let config: ConfigVanilla = options
                .config
                .and_then(|config| serde_json::from_slice(&config).ok())
                .unwrap_or_default();

            let game_type = match config.game_type {
                ConfigGameType::Ctf | ConfigGameType::Tdm => GameType::Team,
                ConfigGameType::Dm => GameType::Solo,
            };

            let account_info = db_task.get_storage().ok();

            let chat_commands = ChatCommands {
                cmds: vec![("account_info".to_string(), vec![])]
                    .into_iter()
                    .collect(),
                prefixes: vec!['/'],
            };
            let rcon_commands = RconCommands {
                cmds: vec![
                    ("info".to_string(), vec![]),
                    ("cheat.all_weapons".to_string(), vec![]),
                ]
                .into_iter()
                .collect(),
            };

            let has_accounts = account_info.is_some();

            let mut game = Self {
                game: Game {
                    stages: Default::default(),
                    players: Players::new(),
                    no_char_players: NoCharPlayers::new(),
                    timeout_players: Default::default(),
                },
                pred_game: Game {
                    stages: Default::default(),
                    players: Players::new(),
                    no_char_players: NoCharPlayers::new(),
                    timeout_players: Default::default(),
                },

                simulation_events: SimulationEvents::new(),

                // server
                stage_0_id: id_generator.next_id(), // TODO: few lines later the stage_id gets reassigned, but too lazy to improve it rn

                // physics
                collision,
                spawns: GameSpawns {
                    spawns,
                    spawns_red,
                    spawns_blue,
                },
                game_objects_definitions: Rc::new(game_objects),
                pred_game_objects_definitions: Rc::new(GameObjectDefinitions {
                    pickups: Default::default(),
                }),

                // game
                game_options: GameOptions::new(game_type, config.score_limit, matches!(config.game_type, ConfigGameType::Ctf)),
                config: config.clone(),
                chat_commands: chat_commands.clone(),
                rcon_commands: rcon_commands.clone(),
                map_name,

                // db
                game_db: GameDb {
                    io_batcher,
                    account_info,

                    cur_queries: Default::default(),
                    cur_queries_helper: Default::default(),
                },

                // pool
                world_pool: WorldPool::new(options.hint_max_characters.unwrap_or(64)),
                no_char_player_clone_pool: Pool::with_capacity(2),
                player_clone_pool: Pool::with_capacity(2),
                game_pools: GamePooling::new(options.hint_max_characters),

                id_generator,
                event_id_generator: Default::default(),

                // snapshot
                snap_shot_manager: SnapshotManager::new(&Default::default()),
            };
            game.stage_0_id = game.add_stage("".to_string(), ubvec4::new(0, 0, 0, 0));
            (
                game,
                GameStateStaticInfo {
                    ticks_in_a_second: NonZero::new(TICKS_PER_SECOND).unwrap(),
                    chat_commands,
                    rcon_commands,
                    config: serde_json::to_vec(&config).ok(),

                    mod_name: match config.game_type {
                        ConfigGameType::Dm => "dm".try_into().unwrap(),
                        ConfigGameType::Tdm => "tdm".try_into().unwrap(),
                        ConfigGameType::Ctf => "ctf".try_into().unwrap(),
                    },
                    version: "pre-alpha".to_string(),
                    options: GameStateServerOptions {
                        physics_group_name: "vanilla".try_into().unwrap(),
                        allow_stages: config.allow_stages,
                        use_vanilla_sides: matches!(config.game_type, ConfigGameType::Ctf | ConfigGameType::Tdm),
                        use_account_name: has_accounts,
                    },
                },
            )
        }
    }

    impl GameState {
        fn add_stage(&mut self, name: String, stage_color: ubvec4) -> GameEntityId {
            let stage_id = self.id_generator.next_id();
            self.game.stages.insert(
                stage_id,
                GameStage::new(
                    name,
                    stage_color,
                    stage_id,
                    &self.world_pool,
                    &self.game_objects_definitions,
                    NonZeroU16::new(self.collision.get_playfield_width() as u16).unwrap(),
                    NonZeroU16::new(self.collision.get_playfield_height() as u16).unwrap(),
                    Some(&self.id_generator),
                    self.game_options,
                ),
            );
            stage_id
        }

        pub fn add_char_to_stage<'a>(
            stages: &'a mut Stages,
            spawns: &GameSpawns,
            stage_id: &GameEntityId,
            character_id: &GameEntityId,
            player_info: PlayerInfo,
            player_input: CharacterInput,
            players: Players,
            no_char_players: NoCharPlayers,
            network_stats: PlayerNetworkStats,
            forced_side: Option<MatchSide>,
            initial_score: i64,
        ) -> &'a mut Character {
            Self::add_char_to_stage_checked(
                stages,
                spawns,
                stage_id,
                character_id,
                player_info,
                player_input,
                players,
                no_char_players,
                network_stats,
                forced_side,
                initial_score,
            )
            .unwrap()
        }

        pub(crate) fn add_char_to_stage_checked<'a>(
            stages: &'a mut Stages,
            spawns: &GameSpawns,
            stage_id: &GameEntityId,
            character_id: &GameEntityId,
            player_info: PlayerInfo,
            player_input: CharacterInput,
            players: Players,
            no_char_players: NoCharPlayers,
            network_stats: PlayerNetworkStats,
            forced_side: Option<MatchSide>,
            initial_score: i64,
        ) -> anyhow::Result<&'a mut Character> {
            let stage = stages.get_mut(stage_id).ok_or(GameError::InvalidStage)?;

            let side = match stage.match_manager.game_match.ty {
                MatchType::Solo => None,
                MatchType::Sided { .. } => forced_side.or_else(|| {
                    Some(
                        stage
                            .world
                            .evaluate_character_side(&no_char_players, stage_id),
                    )
                }),
            };

            // TODO: remove this log (move it somewhere)
            log::info!(target: "world", "added a character into side {:?}", side);

            let pos = stage.world.get_spawn_pos(spawns, side);

            let char = stage.world.add_character(
                *character_id,
                stage_id,
                player_info,
                player_input,
                side,
                CharacterPlayerTy::Player {
                    players,
                    no_char_players,
                    network_stats,
                    stage_id: *stage_id,
                },
                pos,
            );
            char.core.score = initial_score;
            Ok(char)
        }

        pub(crate) fn insert_new_stage(
            stages: &mut Stages,
            stage_id: GameEntityId,
            stage_name: String,
            stage_color: ubvec4,
            world_pool: &WorldPool,
            game_object_definitions: &Rc<GameObjectDefinitions>,
            width: NonZeroU16,
            height: NonZeroU16,
            id_gen: Option<&IdGenerator>,
            game_options: GameOptions,
        ) {
            stages.insert(
                stage_id,
                GameStage::new(
                    stage_name,
                    stage_color,
                    stage_id,
                    world_pool,
                    game_object_definitions,
                    width,
                    height,
                    id_gen,
                    game_options,
                ),
            );
        }

        fn tick_impl(&mut self, is_prediction: bool) {
            for stage in if !is_prediction {
                &mut self.game.stages
            } else {
                &mut self.pred_game.stages
            }
            .values_mut()
            {
                let stage_id = stage.game_element_id;
                let mut sim_pipe = SimulationPipeStage::new(
                    is_prediction,
                    &self.collision,
                    &stage_id,
                    &mut self.world_pool,
                );

                if !is_prediction {
                    self.simulation_events
                        .insert_world_evs(stage_id, stage.tick(&mut sim_pipe));
                } else {
                    // ignore prediction events
                    let _ = stage.tick(&mut sim_pipe);
                }
            }
        }

        fn on_character_spawn(&mut self, stage_id: &GameEntityId, character_id: &GameEntityId) {
            let world = &mut self.game.stages.get_mut(&stage_id).unwrap().world;
            let character = world.characters.get_mut(character_id).unwrap();
            let core = &mut character.core;

            core.active_weapon = WeaponType::Gun;

            let gun = Weapon {
                cur_ammo: Some(10),
                next_ammo_regeneration_tick: 0.into(),
            };

            let hammer = Weapon {
                cur_ammo: None,
                next_ammo_regeneration_tick: 0.into(),
            };

            let reusable_core = &mut character.reusable_core;
            reusable_core.weapons.insert(WeaponType::Hammer, hammer);
            reusable_core.weapons.insert(WeaponType::Gun, gun);
        }

        pub fn player_tick(&mut self) {
            let mut characters_to_spawn = self.no_char_player_clone_pool.new();
            let characters_to_spawn = &mut characters_to_spawn;
            self.game.no_char_players.retain_with_order(hi_closure!(
                [
                    characters_to_spawn: &mut PoolLinkedHashMap<GameEntityId, NoCharPlayer>
                ],
                |id: &GameEntityId, no_char_player: &mut NoCharPlayer| -> bool {
                    if let NoCharPlayerType::Dead {respawn_in_ticks, ..} = &mut no_char_player.no_char_type {
                        // try to respawn
                        if respawn_in_ticks.tick().unwrap_or_default() {
                            characters_to_spawn.insert(
                                no_char_player.id,
                                {
                                    NoCharPlayer::new(
                                        no_char_player.player_info.clone(),
                                        no_char_player.player_input,
                                        id,
                                        no_char_player.no_char_type,
                                        no_char_player.network_stats
                                    )
                                },
                            );
                            false
                        }
                        else {
                            true
                        }
                    } else {
                        true
                    }
                }
            ));

            for (_, no_char_player) in characters_to_spawn.drain() {
                let (forced_side, score, last_stage_id) = if let NoCharPlayerType::Dead {
                    side,
                    score,
                    stage_id,
                    ..
                } = no_char_player.no_char_type
                {
                    (side, score, Some(stage_id))
                } else {
                    (None, 0, None)
                };
                let player_id = no_char_player.id;
                let (char_id, stage_id) = match Self::add_char_to_stage_checked(
                    &mut self.game.stages,
                    &self.spawns,
                    &last_stage_id.unwrap_or(self.stage_0_id),
                    &player_id,
                    no_char_player.player_info.clone(),
                    no_char_player.player_input,
                    self.game.players.clone(),
                    self.game.no_char_players.clone(),
                    no_char_player.network_stats,
                    forced_side,
                    score,
                ) {
                    Err(_) => (
                        GameState::add_char_to_stage(
                            &mut self.game.stages,
                            &self.spawns,
                            &self.stage_0_id,
                            &player_id,
                            no_char_player.player_info,
                            no_char_player.player_input,
                            self.game.players.clone(),
                            self.game.no_char_players.clone(),
                            no_char_player.network_stats,
                            forced_side,
                            score,
                        )
                        .base
                        .game_element_id,
                        self.stage_0_id,
                    ),
                    Ok(char) => (
                        char.base.game_element_id,
                        last_stage_id.unwrap_or(self.stage_0_id),
                    ),
                };

                self.on_character_spawn(
                    &stage_id,
                    &char_id,
                );
            }

            let mut kick_players = Vec::new();
            self.game.timeout_players.retain(|_, player| {
                if player.1.tick().unwrap_or_default() {
                    kick_players.push(player.0);
                    false
                } else {
                    true
                }
            });
            for kick_player in kick_players {
                self.player_drop(&kick_player, PlayerDropReason::Disconnect);
            }
        }

        fn query_tick(&mut self) {
            self.game_db.cur_queries_helper.clear();
            for query in self.game_db.cur_queries.drain(..) {
                if query.is_finished() {
                    if let Ok(query) = query.get_storage() {
                        match query {
                            GameDbQueries::AccountInfo {
                                player_id,
                                account_info: info,
                            } => {
                                self.game
                                    .stages
                                    .get(&self.stage_0_id)
                                    .unwrap()
                                    .simulation_events
                                    .push(SimulationWorldEvent::Global(
                                        GameWorldGlobalEvent::System(
                                            GameWorldSystemMessage::Custom(
                                                self.game_pools.mt_string_pool.new_str(&format!(
                                                "user account information:\n\
                                                id: {}\n\
                                                name: {}\n\
                                                creation: {}",
                                                        info.id,
                                                        info.name,
                                                        <chrono::DateTime<chrono::Utc>>::from_timestamp(
                                                            info.create_time.secs as i64,
                                                            info.create_time.subsec_nanos
                                                        )
                                                        .unwrap()
                                                    )),
                                            ),
                                        ),
                                    ));
                            }
                        }
                    }
                }
            }
            std::mem::swap(
                &mut self.game_db.cur_queries_helper,
                &mut self.game_db.cur_queries,
            );
        }

        fn set_player_inp_impl(
            &mut self,
            player_id: &GameEntityId,
            inp: &CharacterInput,
            diff: CharacterInputConsumableDiff,
            intra_tick_ratio: Option<f64>,
            is_prediction: bool,
        ) {
            if let Some(player) = self.game.players.player(player_id) {
                let stages = if !is_prediction {
                    &mut self.game.stages
                } else {
                    &mut self.pred_game.stages
                };
                let character = stages
                    .get_mut(&player.stage_id())
                    .unwrap()
                    .world
                    .characters
                    .get_mut(player_id)
                    .unwrap();
                character.core.input = *inp;
                character.core.input_intra_tick_ratio = intra_tick_ratio.unwrap_or_default();
                let stage = stages.get_mut(&player.stage_id()).unwrap();
                if matches!(
                    stage.match_manager.game_match.state,
                    MatchState::Running { .. } | MatchState::Paused { .. }
                ) {
                    stage
                        .world
                        .handle_character_input_change(&self.collision, player_id, diff);
                }
            }
        }

        fn snapshot_for_impl(&self, snap_for: SnapshotFor) -> MtPoolCow<'static, [u8]> {
            let snapshot = self.snap_shot_manager.snapshot_for(self, snap_for);
            let mut res = self.game_pools.snapshot_pool.new();
            let writer: &mut Vec<_> = res.to_mut();
            bincode::serde::encode_into_std_write(&snapshot, writer, bincode::config::standard())
                .unwrap();
            res
        }

        fn cmd_account_info(game_db: &mut GameDb, player_id: &GameEntityId, character: &Character) {
            if let (Some(account_info), PlayerUniqueId::Account(account_id)) = (
                &game_db.account_info,
                &character.player_info.unique_identifier,
            ) {
                let account_info = account_info.clone();
                let account_id = *account_id;
                let player_id = *player_id;
                game_db
                    .cur_queries
                    .push(game_db.io_batcher.spawn(async move {
                        Ok(GameDbQueries::AccountInfo {
                            player_id,
                            account_info: account_info.fetch(account_id).await?,
                        })
                    }));
            }
        }

        fn handle_chat_commands(&mut self, player_id: &GameEntityId, cmds: Vec<CommandType>) {
            let Some(server_player) = self.game.players.player(player_id) else {
                return;
            };
            let Some(character) = self
                .game
                .stages
                .get(&server_player.stage_id())
                .and_then(|stage| stage.world.characters.get(player_id))
            else {
                return;
            };
            for cmd in cmds {
                match cmd {
                    CommandType::Full(cmd) => {
                        match cmd.ident.as_str() {
                            "account_info" => {
                                Self::cmd_account_info(&mut self.game_db, player_id, character);
                            }
                            _ => {
                                // TODO: send command not found text
                            }
                        }
                    }
                    CommandType::Partial(_) => {
                        // TODO: ignore for now
                        // send back feedback to user
                    }
                }
            }
        }

        fn handle_rcon_commands(
            &mut self,
            player_id: &GameEntityId,
            _auth: AuthLevel,
            cmds: Vec<CommandType>,
        ) {
            let Some(character_info) = self.game.players.player(player_id) else {
                return;
            };
            for cmd in cmds {
                match cmd {
                    CommandType::Full(cmd) => {
                        match cmd.ident.as_str() {
                            "info" => {
                                self.game
                                    .stages
                                    .get(&self.stage_0_id)
                                    .unwrap()
                                    .simulation_events
                                    .push(SimulationWorldEvent::Global(
                                        GameWorldGlobalEvent::System(
                                            GameWorldSystemMessage::Custom(
                                                self.game_pools
                                                    .mt_string_pool
                                                    .new_str("You are playing vanilla."),
                                            ),
                                        ),
                                    ));
                            }
                            "cheat.all_weapons" => {
                                if let Some(character) = self
                                    .game
                                    .stages
                                    .get_mut(&character_info.stage_id())
                                    .and_then(|stage| stage.world.characters.get_mut(player_id))
                                {
                                    let reusable_core = &mut character.reusable_core;
                                    let gun = Weapon {
                                        cur_ammo: Some(10),
                                        next_ammo_regeneration_tick: 0.into(),
                                    };
                                    reusable_core.weapons.insert(WeaponType::Gun, gun);
                                    reusable_core.weapons.insert(WeaponType::Shotgun, gun);
                                    reusable_core.weapons.insert(WeaponType::Grenade, gun);
                                    reusable_core.weapons.insert(WeaponType::Laser, gun);
                                }
                            }
                            _ => {
                                // TODO: send command not found text
                            }
                        }
                    }
                    CommandType::Partial(_) => {
                        // TODO: ignore for now
                        // send back feedback to user
                    }
                }
            }
        }

        fn build_pred_from_stages(
            &mut self,
            snap_stages: PoolLinkedHashMap<GameEntityId, SnapshotStage>,
        ) {
            SnapshotManager::convert_to_game_stages(
                snap_stages,
                &mut self.pred_game.stages,
                &self.world_pool,
                &self.pred_game_objects_definitions,
                None,
                &self.game_options,
                &self.pred_game.players,
                &self.pred_game.no_char_players,
                NonZeroU16::new(self.collision.get_playfield_width() as u16).unwrap(),
                NonZeroU16::new(self.collision.get_playfield_height() as u16).unwrap(),
            );
        }

        // rendering related
        fn stage_projectiles(
            &self,
            stage: &GameStage,
            pred_stage: Option<&GameStage>,
            ratio: f64,
        ) -> PoolLinkedHashMap<GameEntityId, ProjectileRenderInfo> {
            let mut res = self.game_pools.projectile_render_info_pool.new();
            let Some(pred_stage) = pred_stage else {
                return res;
            };
            res.extend(stage.world.projectiles.iter().filter_map(|(&id, proj)| {
                let pred_proj = pred_stage.world.projectiles.get(&id)?;
                Some((
                    id,
                    ProjectileRenderInfo {
                        ty: proj.projectile.core.ty,
                        pos: projectile::lerped_pos(&proj.projectile, &pred_proj.projectile, ratio)
                            / 32.0,
                        vel: projectile::estimated_fly_direction(
                            &proj.projectile,
                            &pred_proj.projectile,
                            ratio,
                        ) / 32.0,
                        owner_id: Some(pred_proj.character_id),
                    },
                ))
            }));
            res
        }

        fn stage_ctf_flags(
            &self,
            stage: &GameStage,
            pred_stage: Option<&GameStage>,
            ratio: f64,
        ) -> PoolLinkedHashMap<GameEntityId, FlagRenderInfo> {
            let mut res = self.game_pools.flag_render_info_pool.new();
            let Some(pred_stage) = pred_stage else {
                return res;
            };
            let mut collect_flags = |flags: &Flags, pred_flags: &Flags| {
                res.extend(flags.iter().filter_map(|(&id, flag)| {
                    let pred_flag = pred_flags.get(&id)?;
                    Some((
                        id,
                        FlagRenderInfo {
                            pos: Flag::lerped_pos(flag, pred_flag, ratio) / 32.0,
                            ty: flag.core.ty,
                            owner_id: pred_flag.core.carrier,
                        },
                    ))
                }));
            };
            collect_flags(&stage.world.red_flags, &pred_stage.world.red_flags);
            collect_flags(&stage.world.blue_flags, &pred_stage.world.blue_flags);
            res
        }

        fn stage_lasers(&self, ratio: f64) -> PoolLinkedHashMap<GameEntityId, LaserRenderInfo> {
            let mut res = self.game_pools.laser_render_info_pool.new();
            self.game.stages.iter().for_each(|(stage_id, stage)| {
                let Some(pred_stage) = self.pred_game.stages.get(stage_id) else {
                    return;
                };
                res.extend(stage.world.lasers.iter().filter_map(|(&id, laser)| {
                    let pred_laser = pred_stage.world.lasers.get(&id)?;
                    if pred_laser.laser.core.next_eval_in.is_none() {
                        return None;
                    }
                    Some((
                        id,
                        LaserRenderInfo {
                            ty: laser.laser.core.ty,
                            pos: Laser::lerped_pos(&laser.laser, &pred_laser.laser, ratio) / 32.0,
                            from: Laser::lerped_from(&laser.laser, &pred_laser.laser, ratio) / 32.0,
                            eval_tick_ratio: laser.laser.eval_tick_ratio(),
                            owner_id: Some(laser.character_id),
                        },
                    ))
                }))
            });
            res
        }

        fn stage_pickups(
            &self,
            stage: &GameStage,
            pred_stage: Option<&GameStage>,
            ratio: f64,
        ) -> PoolLinkedHashMap<GameEntityId, PickupRenderInfo> {
            let mut res = self.game_pools.pickup_render_info_pool.new();
            let Some(pred_stage) = pred_stage else {
                return res;
            };
            res.extend(stage.world.pickups.iter().filter_map(|(&id, pickup)| {
                let pred_pickup = pred_stage.world.pickups.get(&id)?;
                Some((
                    id,
                    PickupRenderInfo {
                        ty: pickup.core.ty,
                        pos: Pickup::lerped_pos(pickup, pred_pickup, ratio) / 32.0,
                        owner_id: None,
                    },
                ))
            }));
            res
        }

        fn stage_characters_render_info(
            &self,
            stage: &GameStage,
            pred_stage: Option<&GameStage>,
            intra_tick_ratio: f64,
        ) -> PoolLinkedHashMap<GameEntityId, CharacterRenderInfo> {
            let mut render_infos = self.game_pools.character_render_info_pool.new();

            let pred_stage = pred_stage.unwrap_or(stage);
            render_infos.extend(stage.world.characters.iter().map(|(id, character)| {
                let pred_character = pred_stage.world.characters.get(id).unwrap_or(character);
                (
                    *id,
                    CharacterRenderInfo {
                        lerped_pos: character::lerp_core_pos(
                            character,
                            pred_character,
                            intra_tick_ratio,
                        ) / 32.0,
                        lerped_vel: character::lerp_core_vel(
                            character,
                            pred_character,
                            intra_tick_ratio,
                        ) / 32.0,
                        lerped_hook_pos: {
                            // try special logic for when a character is hooked first.
                            let hooked_char = character.hook.hooked_char();
                            hooked_char
                                .and_then(|hooked_char_id| {
                                    let hooked_char = stage.world.characters.get(&hooked_char_id);
                                    let pred_hooked_char = pred_stage
                                        .world
                                        .characters
                                        .get(&hooked_char_id)
                                        .or(hooked_char);
                                    hooked_char.zip(pred_hooked_char).map(
                                        |(character, pred_character)| {
                                            character::lerp_core_pos(
                                                character,
                                                pred_character,
                                                intra_tick_ratio,
                                            )
                                        },
                                    )
                                })
                                // else fall back to the latest known hook pos
                                .or_else(|| {
                                    character::lerp_core_hook_pos(
                                        character,
                                        pred_character,
                                        intra_tick_ratio,
                                    )
                                })
                        }
                        .map(|pos| pos / 32.0),
                        has_air_jump: character.core.core.jumped <= 1,
                        cursor_pos: character.core.input.cursor.to_vec2(),
                        move_dir: *character.core.input.state.dir,
                        cur_weapon: character.core.active_weapon,
                        recoil_ticks_passed: character.core.attack_recoil.action_ticks(),
                        right_eye: character.core.eye,
                        left_eye: character.core.eye,
                        buffs: {
                            let mut buffs = self.game_pools.character_buffs.new();
                            buffs.extend(character.reusable_core.buffs.iter().map(|(buff, _)| {
                                match buff {
                                    CharacterBuff::Ninja => (
                                        CharacterBuff::Ninja,
                                        CharacterBuffInfo {
                                            remaining_time: None,
                                        },
                                    ),
                                    CharacterBuff::Ghost => (
                                        CharacterBuff::Ghost,
                                        CharacterBuffInfo {
                                            remaining_time: None,
                                        },
                                    ),
                                }
                            }));
                            buffs
                        },
                        debuffs: {
                            let mut debuffs = self.game_pools.character_debuffs.new();
                            debuffs.extend(character.reusable_core.debuffs.iter().map(
                                |(debuff, _)| match debuff {
                                    CharacterDebuff::Freeze => (
                                        CharacterDebuff::Freeze,
                                        CharacterDebuffInfo {
                                            remaining_time: None,
                                        },
                                    ),
                                },
                            ));
                            debuffs
                        },

                        animation_ticks_passed: stage.match_manager.game_match.state.passed_ticks(),
                        game_ticks_passed: stage.match_manager.game_match.state.passed_ticks(),
                        game_round_ticks: None,

                        emoticon: character.core.cur_emoticon.and_then(|emoticon| {
                            character
                                .core
                                .emoticon_tick
                                .action_ticks()
                                .map(|tick| (tick, emoticon))
                        }),
                    },
                )
            }));
            render_infos
        }
    }

    impl GameStateInterface for GameState {
        fn collect_characters_info(&self) -> PoolLinkedHashMap<GameEntityId, CharacterInfo> {
            let mut character_infos = self.game_pools.character_info_pool.new();

            let mut players = self.no_char_player_clone_pool.new();
            self.game.no_char_players.pooled_clone_into(&mut players);
            let no_char_players = players.iter().map(|(_, player)| {
                (
                    None::<GameEntityId>,
                    (
                        &player.id,
                        if let NoCharPlayerType::Dead { side, .. } = player.no_char_type {
                            Some(side)
                        } else {
                            None
                        },
                        &player.player_info,
                    ),
                    Some(match player.no_char_type {
                        NoCharPlayerType::Spectator => CharacterPlayerInfo {
                            cam_mode: PlayerCameraMode::Free,
                        },
                        NoCharPlayerType::Dead { died_at_pos, .. } => CharacterPlayerInfo {
                            cam_mode: PlayerCameraMode::LockedTo(died_at_pos / 32.0),
                        },
                    }),
                    {
                        let mut str = self.game_pools.string_pool.new();
                        if let NoCharPlayerType::Dead { score, .. } = player.no_char_type {
                            let _ = str.write_fmt(format_args!("{}", score));
                        }
                        str
                    },
                )
            });
            // of all chars (even server-side ones)
            // + all non char players
            self.game
                .stages
                .iter()
                .flat_map(|(stage_id, stage)| {
                    stage.world.characters.iter().map(|(id, character)| {
                        (
                            Some(*stage_id),
                            (id, Some(character.core.side), &character.player_info),
                            self.game
                                .players
                                .player(id)
                                .is_some()
                                .then_some(CharacterPlayerInfo {
                                    cam_mode: PlayerCameraMode::Default,
                                }),
                            {
                                let mut str = self.game_pools.string_pool.new();
                                let _ = str.write_fmt(format_args!("{}", character.core.score));
                                str
                            },
                        )
                    })
                })
                .chain(no_char_players)
                .for_each(
                    |(stage_id, (id, character_game_info, info), is_player, score)| {
                        character_infos.insert(
                            *id,
                            CharacterInfo {
                                info: info.player_info.clone(),
                                skin_info: match character_game_info.and_then(|side| side) {
                                    Some(side) => match side {
                                        MatchSide::Red => NetworkSkinInfo::Custom {
                                            body_color: ubvec4::new(255, 0, 0, 255),
                                            feet_color: ubvec4::new(255, 0, 0, 255),
                                        },
                                        MatchSide::Blue => NetworkSkinInfo::Custom {
                                            body_color: ubvec4::new(0, 0, 255, 255),
                                            feet_color: ubvec4::new(0, 0, 255, 255),
                                        },
                                    },
                                    None => {
                                        if character_game_info.is_some() {
                                            info.player_info.skin_info
                                        } else {
                                            NetworkSkinInfo::Custom {
                                                body_color: ubvec4::new(255, 0, 255, 255),
                                                feet_color: ubvec4::new(255, 0, 255, 255),
                                            }
                                        }
                                    }
                                },
                                stage_id,
                                player_info: is_player,
                                browser_score: score,
                                browser_eye: TeeEye::Normal,
                            },
                        );
                    },
                );

            character_infos
        }

        fn collect_scoreboard_info(&self) -> Scoreboard {
            let mut spectator_scoreboard_infos =
                self.game_pools.player_spectator_scoreboard_pool.new();

            let mut no_char_players = self.no_char_player_clone_pool.new();
            self.game
                .no_char_players
                .pooled_clone_into(&mut no_char_players);

            let mut red_or_solo_stage_infos = self.game_pools.stage_scoreboard_pool.new();
            let mut blue_stage_infos = self.game_pools.stage_scoreboard_pool.new();
            for (&stage_id, stage) in self.game.stages.iter() {
                let mut red_or_solo_characters = self.game_pools.character_scoreboard_pool.new();
                let mut blue_characters = self.game_pools.character_scoreboard_pool.new();

                for (id, character) in stage.world.characters.iter() {
                    let info = ScoreboardCharacterInfo {
                        id: *id,

                        score: character.core.score,
                        ping: if let Some(stats) = character.is_player_character() {
                            ScoreboardConnectionType::Network(stats)
                        } else {
                            ScoreboardConnectionType::Bot
                        },
                    };

                    match character.core.side {
                        Some(side) => match side {
                            MatchSide::Red => red_or_solo_characters.push(info),
                            MatchSide::Blue => blue_characters.push(info),
                        },
                        None => red_or_solo_characters.push(info),
                    }
                }

                red_or_solo_stage_infos.insert(
                    stage_id,
                    ScoreboardStageInfo {
                        characters: red_or_solo_characters,
                        name: self.game_pools.string_pool.new_str(&stage.stage_name),
                        max_size: 0,
                        color: stage.stage_color,
                    },
                );
                blue_stage_infos.insert(
                    stage_id,
                    ScoreboardStageInfo {
                        characters: blue_characters,
                        name: self.game_pools.string_pool.new_str(&stage.stage_name),
                        max_size: 0,
                        color: stage.stage_color,
                    },
                );
            }

            for (id, p) in no_char_players.iter() {
                if let NoCharPlayerType::Dead {
                    side,
                    score,
                    stage_id,
                    ..
                } = &p.no_char_type
                {
                    let Some(stage) = self.game.stages.get(stage_id) else {
                        continue;
                    };

                    let info = ScoreboardCharacterInfo {
                        id: *id,

                        score: *score,
                        ping: ScoreboardConnectionType::Network(p.network_stats),
                    };
                    match side {
                        Some(MatchSide::Blue) => {
                            let entry = blue_stage_infos.entry(*stage_id).or_insert_with(|| {
                                ScoreboardStageInfo {
                                    characters: self.game_pools.character_scoreboard_pool.new(),
                                    name: self.game_pools.string_pool.new_str(&stage.stage_name),
                                    max_size: 0,
                                    color: stage.stage_color,
                                }
                            });
                            entry.characters.push(info);
                        }
                        Some(MatchSide::Red) | None => {
                            let entry =
                                red_or_solo_stage_infos.entry(*stage_id).or_insert_with(|| {
                                    ScoreboardStageInfo {
                                        characters: self.game_pools.character_scoreboard_pool.new(),
                                        name: self
                                            .game_pools
                                            .string_pool
                                            .new_str(&stage.stage_name),
                                        max_size: 0,
                                        color: stage.stage_color,
                                    }
                                });
                            entry.characters.push(info);
                        }
                    }
                } else {
                    // add to spectators instead
                    spectator_scoreboard_infos.push(ScoreboardPlayerSpectatorInfo {
                        id: p.id,

                        score: 0,
                        ping: ScoreboardConnectionType::Network(p.network_stats),
                    });
                }
            }

            for stage in red_or_solo_stage_infos.values_mut() {
                stage.characters.sort_by_key(|c| c.score);
            }
            for stage in blue_stage_infos.values_mut() {
                stage.characters.sort_by_key(|c| c.score);
            }

            let ty = self.game_options.ty;
            Scoreboard {
                game: match ty {
                    GameType::Solo => ScoreboardGameType::SoloPlay {
                        stages: red_or_solo_stage_infos,
                        ignore_stage: self.stage_0_id,
                        spectator_players: spectator_scoreboard_infos,
                    },
                    GameType::Team => ScoreboardGameType::SidedPlay {
                        red_stages: red_or_solo_stage_infos,
                        blue_stages: blue_stage_infos,
                        ignore_stage: self.stage_0_id,
                        spectator_players: spectator_scoreboard_infos,

                        red_side_name: self.game_pools.string_pool.new_str("Red Team"),
                        blue_side_name: self.game_pools.string_pool.new_str("Blue Team"),
                    },
                },
                options: ScoreboardGameOptions {
                    map_name: self.game_pools.string_pool.new_str(&self.map_name),
                    score_limit: self.game_options.score_limit,
                },
            }
        }

        fn all_stages(
            &self,
            intra_tick_ratio: f64,
        ) -> PoolLinkedHashMap<GameEntityId, StageRenderInfo> {
            let mut stages = self.game_pools.stage_render_info.new();

            for (id, stage) in self.game.stages.iter() {
                let pred_stage = self.pred_game.stages.get(id);

                stages.insert(
                    *id,
                    StageRenderInfo {
                        world: WorldRenderInfo {
                            projectiles: self.stage_projectiles(
                                stage,
                                pred_stage,
                                intra_tick_ratio,
                            ),
                            ctf_flags: self.stage_ctf_flags(stage, pred_stage, intra_tick_ratio),
                            lasers: self.stage_lasers(intra_tick_ratio),
                            pickups: self.stage_pickups(stage, pred_stage, intra_tick_ratio),
                            characters: self.stage_characters_render_info(
                                stage,
                                pred_stage,
                                intra_tick_ratio,
                            ),
                        },
                        game: GameRenderInfo::Match {
                            standings: match stage.match_manager.game_match.ty {
                                MatchType::Solo => MatchStandings::Solo {
                                    leading_players: Default::default(), // TODO:
                                },
                                MatchType::Sided { scores } => MatchStandings::Sided {
                                    score_red: scores[0],
                                    score_blue: scores[1],
                                },
                            },
                        },
                    },
                );
            }

            stages
        }

        fn collect_character_local_render_info(
            &self,
            player_id: &GameEntityId,
        ) -> LocalCharacterRenderInfo {
            if let Some(p) = self.game.players.player(player_id) {
                let player_char = self
                    .game
                    .stages
                    .get(&p.stage_id())
                    .unwrap()
                    .world
                    .characters
                    .get(player_id)
                    .unwrap();

                LocalCharacterRenderInfo {
                    health: player_char.core.health,
                    armor: player_char.core.armor,
                    ammo_of_weapon: player_char.reusable_core.weapons
                        [&player_char.core.active_weapon]
                        .cur_ammo,
                }
            } else {
                LocalCharacterRenderInfo {
                    health: 0,
                    armor: 0,
                    ammo_of_weapon: None,
                }
            }
        }

        fn get_client_camera_join_pos(&self) -> vec2 {
            // TODO:
            vec2::default()
        }

        fn player_join(&mut self, client_player_info: &PlayerClientInfo) -> GameEntityId {
            if let Some((timeout_player_id, character_info)) = self
                .game
                .timeout_players
                .remove(&(
                    client_player_info.unique_identifier,
                    client_player_info.player_index,
                ))
                .and_then(|(id, _)| self.game.players.player(&id).map(|char| (id, char)))
            {
                let char = self
                    .game
                    .stages
                    .get_mut(&character_info.stage_id())
                    .unwrap()
                    .world
                    .characters
                    .get_mut(&timeout_player_id)
                    .unwrap();
                char.core.is_timeout = false;
                return timeout_player_id;
            }

            let player_id = self.id_generator.next_id();
            let stage_0_id = self.stage_0_id;

            self.game
                .stages
                .get(&stage_0_id)
                .unwrap()
                .simulation_events
                .push(SimulationWorldEvent::Global(GameWorldGlobalEvent::System(
                    GameWorldSystemMessage::PlayerJoined {
                        id: player_id,
                        name: self
                            .game_pools
                            .mt_string_pool
                            .new_str(&client_player_info.info.name),
                    },
                )));

            // spawn and send character info
            let char_id = Self::add_char_to_stage(
                &mut self.game.stages,
                &self.spawns,
                &stage_0_id,
                &player_id,
                PlayerInfo {
                    player_info: PoolRc::from_item_without_pool(client_player_info.info.clone()),
                    version: 1,
                    unique_identifier: client_player_info.unique_identifier,
                    player_index: client_player_info.player_index,
                    is_dummy: client_player_info.is_dummy,
                },
                Default::default(),
                self.game.players.clone(),
                self.game.no_char_players.clone(),
                client_player_info.initial_network_stats,
                None,
                0,
            )
            .base
            .game_element_id;
            self.on_character_spawn(
                &self.stage_0_id.clone(),
                &char_id,
            );

            player_id
        }

        fn player_drop(&mut self, player_id: &GameEntityId, _reason: PlayerDropReason) {
            let name = if let Some(server_player) = self.game.players.player(player_id) {
                let stage = self.game.stages.get_mut(&server_player.stage_id()).unwrap();

                let character = stage.world.characters.get_mut(player_id).unwrap();

                let name = self
                    .game_pools
                    .mt_string_pool
                    .new_str(&character.player_info.player_info.name);

                character.despawn_completely_silent();
                stage.world.characters.remove(player_id);

                Some((name, server_player.stage_id()))
            } else if let Some(no_char_player) = self.game.no_char_players.remove(player_id) {
                let name = self
                    .game_pools
                    .mt_string_pool
                    .new_str(&no_char_player.player_info.player_info.name);
                Some((name, self.stage_0_id))
            } else {
                None
            };

            if let Some((name, stage_id)) = name {
                let stage = self.game.stages.get(&stage_id).unwrap();
                stage.simulation_events.push(SimulationWorldEvent::Global(
                    GameWorldGlobalEvent::System(GameWorldSystemMessage::PlayerLeft {
                        id: *player_id,
                        name,
                    }),
                ));
            }
        }

        fn try_overwrite_player_character_info(
            &mut self,
            id: &GameEntityId,
            info: &NetworkCharacterInfo,
            version: NonZeroU64,
        ) {
            if let Some(player) = self.game.players.player(id) {
                let player_info = &mut self
                    .game
                    .stages
                    .get_mut(&player.stage_id())
                    .unwrap()
                    .world
                    .characters
                    .get_mut(id)
                    .unwrap()
                    .player_info;
                if player_info.version < version.get() {
                    player_info.player_info = PoolRc::from_item_without_pool(info.clone());
                    player_info.version = version.get();
                }
            } else if !self.game.no_char_players.handle_mut(
                id,
                hi_closure!(
                [version: NonZeroU64, info: &NetworkCharacterInfo],
                |no_char_player: &mut NoCharPlayer| -> () {
                    if no_char_player.player_info.version < version.get() {
                        no_char_player.player_info.player_info = PoolRc::from_item_without_pool(info.clone());
                        no_char_player.player_info.version = version.get();
                    }
                }),
            ) {
                panic!("player did not exist, this should not happen");
            }
        }

        fn account_created(&mut self, account_id: AccountId, cert_fingerprint: Hash) {
            // TODO:
        }

        fn network_stats(
            &mut self,
            mut stats: PoolLinkedHashMap<GameEntityId, PlayerNetworkStats>,
        ) {
            let mut players = self.player_clone_pool.new();
            self.game.players.pooled_clone_into(&mut players);

            for (id, char_info) in players.iter() {
                if let Some(stats) = stats.remove(id) {
                    self.game
                        .stages
                        .get_mut(&char_info.stage_id())
                        .unwrap()
                        .world
                        .characters
                        .get_mut(id)
                        .unwrap()
                        .update_player_ty(
                            &char_info.stage_id(),
                            CharacterPlayerTy::Player {
                                players: self.game.players.clone(),
                                no_char_players: self.game.no_char_players.clone(),
                                network_stats: stats,
                                stage_id: char_info.stage_id(),
                            },
                        );
                }
            }
        }

        fn client_command(&mut self, player_id: &GameEntityId, cmd: ClientCommand) {
            match cmd {
                ClientCommand::Kill => {
                    if let Some(server_player) = self.game.players.player(player_id) {
                        self.game
                            .stages
                            .get_mut(&server_player.stage_id())
                            .unwrap()
                            .world
                            .characters
                            .get_mut(player_id)
                            .unwrap()
                            .despawn_to_respawn();
                        self.game
                            .stages
                            .get_mut(&server_player.stage_id())
                            .unwrap()
                            .world
                            .characters
                            .remove(player_id);
                    }
                }
                ClientCommand::Chat(cmd) => {
                    let cmds = command_parser::parser::parse(&cmd.raw, &self.chat_commands.cmds);
                    self.handle_chat_commands(player_id, cmds);
                }
                ClientCommand::Rcon(cmd) => {
                    if !matches!(cmd.auth_level, AuthLevel::None) {
                        let cmds =
                            command_parser::parser::parse(&cmd.raw, &self.rcon_commands.cmds);
                        self.handle_rcon_commands(player_id, cmd.auth_level, cmds);
                    }
                }
                ClientCommand::JoinStage { name, color } => {
                    if self.config.allow_stages {
                        if let Some(player) = self.game.players.player(player_id) {
                            let stage = &mut self.game.stages.get_mut(&player.stage_id()).unwrap();
                            if let Some(character) = stage.world.characters.remove(player_id) {
                                let player_info = character.player_info.clone();
                                let player_input = character.core.input;
                                let network_stats = character.is_player_character().unwrap();
                                drop(character);

                                if player.stage_id() != self.stage_0_id
                                    && !stage
                                        .world
                                        .characters
                                        .values()
                                        .any(|c| c.is_player_character().is_some())
                                    && !self.game.no_char_players.any_player_in(player.stage_id())
                                {
                                    self.game.stages.remove(&player.stage_id());
                                }

                                let stage_id = self.add_stage(
                                    name.to_string(),
                                    ubvec4::new(color[0], color[1], color[2], 20),
                                );

                                Self::add_char_to_stage(
                                    &mut self.game.stages,
                                    &self.spawns,
                                    &stage_id,
                                    player_id,
                                    player_info,
                                    player_input,
                                    self.game.players.clone(),
                                    self.game.no_char_players.clone(),
                                    network_stats,
                                    None,
                                    0,
                                );
                                self.on_character_spawn(
                                    &stage_id,
                                    player_id,
                                );
                            }
                        }
                    }
                }
                ClientCommand::JoinSide(side) => {
                    if matches!(self.config.game_type, ConfigGameType::Ctf) {
                        if let Some(player) = self.game.players.player(player_id) {
                            if let Some(character) = self
                                .game
                                .stages
                                .get_mut(&player.stage_id())
                                .unwrap()
                                .world
                                .characters
                                .get_mut(player_id)
                            {
                                character.core.side = Some(side)
                            }
                        }
                    }
                }
                ClientCommand::JoinSpectator => {
                    if let Some(player) = self.game.players.player(player_id) {
                        if let Some(mut character) = self
                            .game
                            .stages
                            .get_mut(&player.stage_id())
                            .unwrap()
                            .world
                            .characters
                            .remove(player_id)
                        {
                            character.despawn_to_join_spectators();
                        }
                    }
                }
            }
        }

        fn set_player_input(
            &mut self,
            player_id: &GameEntityId,
            inp: &CharacterInput,
            diff: CharacterInputConsumableDiff,
        ) {
            self.set_player_inp_impl(player_id, inp, diff, None, false)
        }

        fn set_player_emoticon(&mut self, player_id: &GameEntityId, emoticon: EmoticonType) {
            if let Some(player) = self.game.players.player(player_id) {
                let stages = &mut self.game.stages;
                let character = stages
                    .get_mut(&player.stage_id())
                    .unwrap()
                    .world
                    .characters
                    .get_mut(player_id)
                    .unwrap();

                character.core.emoticon_tick = (2 * TICKS_PER_SECOND).into();
                character.core.cur_emoticon = Some(emoticon);
            }
        }

        fn set_player_eye(&mut self, player_id: &GameEntityId, eye: TeeEye, duration: Duration) {
            if let Some(player) = self.game.players.player(player_id) {
                let stages = &mut self.game.stages;
                let character = stages
                    .get_mut(&player.stage_id())
                    .unwrap()
                    .world
                    .characters
                    .get_mut(player_id)
                    .unwrap();

                let normal_in = (duration.as_millis().clamp(0, GameTickType::MAX as u128)
                    as GameTickType
                    / TICKS_PER_SECOND)
                    .max(1);

                character.core.normal_eye_in = normal_in.into();
                character.core.eye = eye;
            }
        }

        fn tick(&mut self) {
            self.tick_impl(false);

            self.player_tick();
            self.query_tick();
        }

        fn pred_tick(
            &mut self,
            mut inps: PoolLinkedHashMap<GameEntityId, CharacterPredictionInput>,
        ) {
            let mut stages = self.snap_shot_manager.snapshot_pool.stages_pool.new();
            self.snap_shot_manager.build_stages(&mut stages, self);
            self.build_pred_from_stages(stages);
            for (
                ref id,
                CharacterPredictionInput {
                    inp,
                    diff,
                    intra_tick_ratio,
                },
            ) in inps.drain()
            {
                self.set_player_inp_impl(id, &inp, diff, Some(intra_tick_ratio), true);
            }
            self.tick_impl(true);
        }

        fn snapshot_for(&self, client: SnapshotClientInfo) -> MtPoolCow<'static, [u8]> {
            self.snapshot_for_impl(SnapshotFor::Client(client))
        }

        fn build_from_snapshot(
            &mut self,
            snapshot: &MtPoolCow<'static, [u8]>,
        ) -> SnapshotLocalPlayers {
            let (snapshot, _) =
                bincode::serde::decode_from_slice(snapshot, bincode::config::standard()).unwrap();

            SnapshotManager::build_from_snapshot(snapshot, self)
        }

        fn snapshot_for_hotreload(&self) -> Option<MtPoolCow<'static, [u8]>> {
            Some(self.snapshot_for_impl(SnapshotFor::Hotreload))
        }

        fn build_from_snapshot_by_hotreload(&mut self, snapshot: &MtPoolCow<'static, [u8]>) {
            let Ok((snapshot, _)) =
                bincode::serde::decode_from_slice(snapshot, bincode::config::standard())
            else {
                return;
            };

            let _ = SnapshotManager::build_from_snapshot(snapshot, self);

            let mut players = self.player_clone_pool.new();
            self.game.players.pooled_clone_into(&mut players);

            for (id, character_info) in players.iter() {
                if let Some(stage) = self.game.stages.get_mut(&character_info.stage_id()) {
                    if let Some(character) = stage.world.characters.get_mut(id) {
                        character.core.is_timeout = true;
                        self.game.timeout_players.insert(
                            (
                                character.player_info.unique_identifier,
                                character.player_info.player_index,
                            ),
                            (*id, (TICKS_PER_SECOND * 120).into()),
                        );
                    }
                }
            }
        }

        fn build_from_snapshot_for_pred(
            &mut self,
            old_snapshot: &MtPoolCow<'static, [u8]>,
            snapshot: &MtPoolCow<'static, [u8]>,
        ) {
            let (snapshot, _): (Snapshot, usize) =
                bincode::serde::decode_from_slice(snapshot, bincode::config::standard()).unwrap();

            self.build_pred_from_stages(snapshot.stages);
        }

        fn events_for(&self, client: EventClientInfo) -> GameEvents {
            // handle simulation events
            let mut worlds_events = self.game_pools.worlds_events_pool.new();
            let worlds_events_ref = &mut worlds_events;

            fn fill_pickup_ev(
                event_id_generator: &EventIdGenerator,
                world_events: &mut MtPoolLinkedHashMap<EventId, GameWorldEvent>,
                owner_id: Option<GameEntityId>,
                ev: PickupEvent,
            ) {
                match ev {
                    PickupEvent::Despawn { .. } => {
                        // ignore
                    }
                    PickupEvent::Pickup { pos, ty } => match ty {
                        PickupType::PowerupHealth => {
                            world_events.insert(
                                event_id_generator.next_id(),
                                GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                    pos,
                                    owner_id,
                                    ev: GameWorldEntityEvent::Pickup {
                                        ev: GamePickupEvent::Heart(GamePickupHeartEvent::Sound(
                                            GamePickupHeartEventSound::Collect,
                                        )),
                                    },
                                }),
                            );
                        }
                        PickupType::PowerupArmor => {
                            world_events.insert(
                                event_id_generator.next_id(),
                                GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                    pos,
                                    owner_id,
                                    ev: GameWorldEntityEvent::Pickup {
                                        ev: GamePickupEvent::Armor(GamePickupArmorEvent::Sound(
                                            GamePickupArmorEventSound::Collect,
                                        )),
                                    },
                                }),
                            );
                        }
                        PickupType::PowerupNinja => {
                            world_events.insert(
                                event_id_generator.next_id(),
                                GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                    pos,
                                    owner_id,
                                    ev: GameWorldEntityEvent::Character {
                                        ev: GameCharacterEvent::Buff(GameBuffEvent::Ninja(
                                            GameBuffNinjaEvent::Sound(
                                                GameBuffNinjaEventSound::Collect,
                                            ),
                                        )),
                                    },
                                }),
                            );
                        }
                        PickupType::PowerupWeapon(weapon) => match weapon {
                            WeaponType::Hammer | WeaponType::Gun => {
                                // nothing to do
                            }
                            WeaponType::Shotgun => {
                                world_events.insert(
                                    event_id_generator.next_id(),
                                    GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                        pos,
                                        owner_id,
                                        ev: GameWorldEntityEvent::Shotgun {
                                            ev: GameShotgunEvent::Sound(
                                                GameShotgunEventSound::Collect,
                                            ),
                                        },
                                    }),
                                );
                            }
                            WeaponType::Grenade => {
                                world_events.insert(
                                    event_id_generator.next_id(),
                                    GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                        pos,
                                        owner_id,
                                        ev: GameWorldEntityEvent::Grenade {
                                            ev: GameGrenadeEvent::Sound(
                                                GameGrenadeEventSound::Collect,
                                            ),
                                        },
                                    }),
                                );
                            }
                            WeaponType::Laser => {
                                world_events.insert(
                                    event_id_generator.next_id(),
                                    GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                        pos,
                                        owner_id,
                                        ev: GameWorldEntityEvent::Laser {
                                            ev: GameLaserEvent::Sound(GameLaserEventSound::Collect),
                                        },
                                    }),
                                );
                            }
                        },
                    },
                }
            }

            let game_pools = &self.game_pools;
            let event_id_generator = &self.event_id_generator;

            self.simulation_events.for_each(hi_closure!([
                game_pools: &GamePooling,
                event_id_generator: &EventIdGenerator,
                worlds_events_ref: &mut MtPoolLinkedHashMap<GameEntityId, GameWorldEvents>,
            ], |world_id: &GameEntityId, evs: &SimulationWorldEvents|
             -> () {
                let mut world_events = game_pools.world_events_pool.new();
                for simulation_event in evs.iter() {
                    match simulation_event {
                        SimulationWorldEvent::Entity(entity) => match entity.ev {
                            SimulationEventWorldEntityType::Character { ev } => match ev {
                                CharacterEvent::Projectile { .. }
                                | CharacterEvent::Laser { .. } => {
                                    // ignored
                                }
                                CharacterEvent::Despawn { killer_id, weapon } => {
                                    world_events.insert(
                                        event_id_generator.next_id(),
                                        GameWorldEvent::Global(GameWorldGlobalEvent::Action(
                                            GameWorldAction::Kill {
                                                killer: killer_id,
                                                assists: game_pools.entity_id_pool.new(),
                                                victims: {
                                                    let mut victims =
                                                        game_pools.entity_id_pool.new();
                                                    if let Some(owner_id) = entity.owner_id {
                                                        victims.push(owner_id);
                                                    }
                                                    victims
                                                },
                                                weapon,
                                                flags: KillFlags::empty(),
                                            },
                                        )),
                                    );
                                }
                                CharacterEvent::Sound { pos, ev } => {
                                    world_events.insert(
                                        event_id_generator.next_id(),
                                        GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                            pos,
                                            owner_id: entity.owner_id,
                                            ev: GameWorldEntityEvent::Character {
                                                ev: GameCharacterEvent::Sound(ev),
                                            },
                                        }),
                                    );
                                }
                                CharacterEvent::Effect { pos, ev } => {
                                    world_events.insert(
                                        event_id_generator.next_id(),
                                        GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                            pos,
                                            owner_id: entity.owner_id,
                                            ev: GameWorldEntityEvent::Character {
                                                ev: GameCharacterEvent::Effect(ev),
                                            },
                                        }),
                                    );
                                }
                                CharacterEvent::Buff { pos, ev } => {
                                    world_events.insert(
                                        event_id_generator.next_id(),
                                        GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                            pos,
                                            owner_id: entity.owner_id,
                                            ev: GameWorldEntityEvent::Character {
                                                ev: GameCharacterEvent::Buff(ev),
                                            },
                                        }),
                                    );
                                }
                                CharacterEvent::Debuff { pos, ev } => {
                                    world_events.insert(
                                        event_id_generator.next_id(),
                                        GameWorldEvent::Positioned(GameWorldPositionedEvent {
                                            pos,
                                            owner_id: entity.owner_id,
                                            ev: GameWorldEntityEvent::Character {
                                                ev: GameCharacterEvent::Debuff(ev),
                                            },
                                        }),
                                    );
                                }
                            },
                            SimulationEventWorldEntityType::Projectile { ev, .. } => {
                                match ev {
                                    ProjectileEvent::Despawn { .. } => {
                                        // nothing to do
                                    }
                                    ProjectileEvent::GrenadeSound { pos, ev } => {
                                        world_events.insert(
                                            event_id_generator.next_id(),
                                            GameWorldEvent::Positioned(
                                                GameWorldPositionedEvent {
                                                    pos,
                                                    owner_id: entity.owner_id,
                                                    ev: GameWorldEntityEvent::Grenade {
                                                        ev: GameGrenadeEvent::Sound(ev),
                                                    },
                                                },
                                            ),
                                        );
                                    }
                                    ProjectileEvent::GrenadeEffect { pos, ev } => {
                                        world_events.insert(
                                            event_id_generator.next_id(),
                                            GameWorldEvent::Positioned(
                                                GameWorldPositionedEvent {
                                                    pos,
                                                    owner_id: entity.owner_id,
                                                    ev: GameWorldEntityEvent::Grenade {
                                                        ev: GameGrenadeEvent::Effect(ev),
                                                    },
                                                },
                                            ),
                                        );
                                    }
                                }
                            }
                            SimulationEventWorldEntityType::Pickup { ev, .. } => {
                                fill_pickup_ev(
                                    event_id_generator,
                                    &mut world_events,
                                    entity.owner_id,
                                    ev,
                                );
                            }
                            SimulationEventWorldEntityType::Flag { ev, .. } => {
                                match ev {
                                    FlagEvent::Despawn { .. } => {
                                        // do nothing
                                    }
                                    FlagEvent::Sound { pos, ev } => {
                                        world_events.insert(
                                            event_id_generator.next_id(),
                                            GameWorldEvent::Positioned(
                                                GameWorldPositionedEvent {
                                                    pos,
                                                    owner_id: entity.owner_id,
                                                    ev: GameWorldEntityEvent::Flag {
                                                        ev: GameFlagEvent::Sound(ev),
                                                    },
                                                },
                                            ),
                                        );
                                    }
                                    FlagEvent::Effect { ev, .. } => match ev {},
                                    FlagEvent::Grab
                                    | FlagEvent::Capture { .. } => {
                                        // ignore, not sent to client
                                    }
                                }
                            }
                            SimulationEventWorldEntityType::Laser { ev, .. } => {
                                match ev {
                                    LaserEvent::Despawn { .. } => {
                                        // do nothing
                                    }
                                    LaserEvent::Sound { pos, ev } => {
                                        world_events.insert(
                                            event_id_generator.next_id(),
                                            GameWorldEvent::Positioned(
                                                GameWorldPositionedEvent {
                                                    pos,
                                                    owner_id: entity.owner_id,
                                                    ev: GameWorldEntityEvent::Laser {
                                                        ev: GameLaserEvent::Sound(ev),
                                                    },
                                                },
                                            ),
                                        );
                                    }
                                }
                            }
                        },
                        SimulationWorldEvent::Global(ev) => {
                            world_events.insert(
                                event_id_generator.next_id(),
                                GameWorldEvent::Global(ev.clone()),
                            );
                        }
                    }
                }
                if !world_events.is_empty() {
                    worlds_events_ref.insert(
                        *world_id,
                        GameWorldEvents {
                            events: world_events,
                        },
                    );
                }
            }));

            GameEvents {
                worlds: worlds_events,
                event_id: self.event_id_generator.peek_next_id(),
            }
        }

        fn clear_events(&mut self) {
            self.simulation_events.take();
        }

        fn sync_event_id(&self, event_id: EventId) {
            self.event_id_generator.reset_id_for_client(event_id);
        }
    }
}
