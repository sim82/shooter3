use bevy::prelude::*;

#[derive(Component, Default, Debug)]
pub struct VelocityExtrapolate {
    pub velocity: Vec3,
    pub base_tick: u32,
}

impl VelocityExtrapolate {
    pub fn apply(&self, tick: u32, base_translation: Vec3) -> Vec3 {
        if tick <= self.base_tick {
            return base_translation;
        }
        let ticks = tick - self.base_tick;
        let f = (ticks as f32) / 60.0;

        base_translation + self.velocity * f
    }
}
