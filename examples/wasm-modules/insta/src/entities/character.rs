pub mod core;
pub mod player;
pub mod pos {
    pub use ::shared_game::entities::character::pos::*;
}
pub mod hook {
    pub use ::shared_game::entities::character::hook::*;
}

use api_macros::character_mod;

#[character_mod("../../../")]
pub mod character {
    impl Character {
        pub fn take_damage_from(
            self_char: &mut Character,
            self_char_id: &GameEntityId,
            killer_id: Option<GameEntityId>,
            force: &vec2,
            _source: &vec2,
            mut dmg_amount: u32,
            from: DamageTypes,
            by: DamageBy,
        ) -> CharacterDamageResult {
            if let DamageTypes::Character(id) = &from {
                if *id != self_char_id {
                    self_char.entity_events.push(CharacterEvent::Sound {
                        pos: *self_char.pos.pos() / 32.0,
                        ev: GameCharacterEventSound::Hit { strong: false },
                    });
                }
            }

            self_char.die(
                killer_id,
                match by {
                    DamageBy::Ninja => GameWorldActionKillWeapon::Ninja,
                    DamageBy::Weapon(weapon) => GameWorldActionKillWeapon::Weapon { weapon },
                },
            );

            CharacterDamageResult::Death
        }
    }
}
