use api_macros::state_mod;

#[state_mod("../../../")]
pub mod state {
    impl GameState {
        fn on_character_spawn(&mut self, stage_id: &GameEntityId, character_id: &GameEntityId) {
            let world = &mut self.game.stages.get_mut(&stage_id).unwrap().world;
            let character = world.characters.get_mut(character_id).unwrap();
            let core = &mut character.core;

            let insta_weapon = self.config.weapon;

            core.active_weapon = insta_weapon;

            let laser = Weapon {
                cur_ammo: None,
                next_ammo_regeneration_tick: 0.into(),
            };

            let reusable_core = &mut character.reusable_core;
            reusable_core.weapons.insert(insta_weapon, laser);
        }
    }
}
