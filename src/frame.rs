use bevy::prelude::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct NetworkedEntities {
    pub entities: Vec<Entity>,
    pub translations: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct WithRotation {
    pub entities: Vec<Entity>,
    pub translations: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub rotations: Vec<Quat>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct NetworkFrame {
    pub tick: u32,
    pub last_player_input: u32,
    pub entities: NetworkedEntities,
    pub with_rotation: WithRotation,
}
