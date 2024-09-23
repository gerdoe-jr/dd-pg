use api_macros::world_mod;

#[world_mod("../../../")]
pub mod world {
    impl GameWorld {
        pub fn new(
            world_pool: &WorldPool,
            game_object_definitions: &Rc<GameObjectDefinitions>,
            width: NonZeroU16,
            height: NonZeroU16,
            id_gen: Option<&IdGenerator>,
        ) -> Self {
            let simulation_events = SimulationEntityEvents::new();
            let mut inactive_game_objects = GameObjectsWorld {
                pickups: Default::default(),
            };

            let mut red_flags = world_pool.flag_pool.flag_pool.new();
            let mut blue_flags = world_pool.flag_pool.flag_pool.new();
            let mut pickups = world_pool.pickup_pool.pickup_pool.new();

            if let Some(id_gen) = id_gen {
                let mut add_pick = |pickup_pos: &ivec2, ty: PickupType| {
                    let id = id_gen.next_id();
                    pickups.insert(
                        id,
                        Pickup::new(
                            &id,
                            &(vec2::new(pickup_pos.x as f32, pickup_pos.y as f32) * 32.0
                                + vec2::new(16.0, 16.0)),
                            ty,
                            &world_pool.pickup_pool,
                            &simulation_events,
                        ),
                    );
                };

                let add_flag = |flags: &mut Flags, pos: &ivec2, ty: FlagType| {
                    let id = id_gen.next_id();
                    flags.insert(
                        id,
                        Flag::new(
                            &id,
                            &(vec2::new(pos.x as f32, pos.y as f32) * 32.0 + vec2::new(16.0, 16.0)),
                            ty,
                            &world_pool.flag_pool,
                            &simulation_events,
                        ),
                    );
                };
                for flag in &game_object_definitions.pickups.red_flags {
                    add_flag(&mut red_flags, flag, FlagType::Red)
                }
                for flag in &game_object_definitions.pickups.blue_flags {
                    add_flag(&mut blue_flags, flag, FlagType::Blue)
                }
            }

            Self {
                removed_characters_helper: world_pool.removed_characters_helper_pool.new(),

                projectiles: world_pool.projectile_pool.projectile_pool.new(),
                red_flags,
                blue_flags,
                pickups,
                lasers: world_pool.laser_pool.laser_pool.new(),
                characters: world_pool.character_pool.character_pool.new(),

                inactive_game_objects,

                world_pool: world_pool.clone(),

                id_generator: id_gen.cloned(),

                simulation_events,
                play_field: CharacterPositionPlayfield::new(width, height),
                hooks: Default::default(),
            }
        }
    }
}
