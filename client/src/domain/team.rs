//! The client's team component and its bridges to `shared::map::Team`.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    Green,
    Blue,
}

impl From<shared::map::Team> for Team {
    fn from(team: shared::map::Team) -> Self {
        match team {
            shared::map::Team::Green => Team::Green,
            shared::map::Team::Blue => Team::Blue,
        }
    }
}

impl From<Team> for shared::map::Team {
    fn from(team: Team) -> Self {
        match team {
            Team::Green => shared::map::Team::Green,
            Team::Blue => shared::map::Team::Blue,
        }
    }
}

impl PartialEq<shared::map::Team> for Team {
    fn eq(&self, other: &shared::map::Team) -> bool {
        *self == Team::from(*other)
    }
}

impl PartialEq<Team> for shared::map::Team {
    fn eq(&self, other: &Team) -> bool {
        Team::from(*self) == *other
    }
}

impl Team {
    pub fn as_str(self) -> &'static str {
        match self {
            Team::Green => "Green",
            Team::Blue => "Blue",
        }
    }
}
