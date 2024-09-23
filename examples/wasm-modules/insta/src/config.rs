use config::{config_default, ConfigInterface};
use game_interface::types::weapons::WeaponType;
use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    ConfigInterface,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
pub enum ConfigGameType {
    #[default]
    Dm,
    Tdm,
    Ctf,
}

#[config_default]
#[derive(Debug, Clone, Serialize, Deserialize, ConfigInterface)]
pub struct ConfigVanilla {
    pub game_type: ConfigGameType,
    #[default = 100]
    pub score_limit: u64,
    pub allow_stages: bool,
    pub weapon: WeaponType
}
