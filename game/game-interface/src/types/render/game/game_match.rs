use hiarc::Hiarc;
use serde::{Deserialize, Serialize};

use crate::types::game::GameEntityId;

/// Current results for the match.
#[derive(Debug, Hiarc, Serialize, Deserialize, Clone, Copy)]
pub enum MatchStandings {
    Solo {
        /// The top players in the current match
        leading_players: [Option<GameEntityId>; 2],
    },
    Sided {
        /// Score for the red side
        score_red: i64,
        /// Score for the blue side
        score_blue: i64,
    },
}

/// The side in the current match
#[derive(Debug, Hiarc, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchSide {
    Red,
    Blue,
}
