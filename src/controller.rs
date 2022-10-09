// adapted from https://github.com/qhdwight/bevy_fps_controller

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::f32::consts::*;
use std::time::Duration;

use bevy::input::mouse::MouseMotion;
use bevy::utils::Instant;
use bevy::{math::Vec3Swizzles, prelude::*};
use bevy_rapier3d::prelude::*;
use serde::{Deserialize, Serialize};

pub struct FpsControllerPlugin;

impl Plugin for FpsControllerPlugin {
    fn build(&self, app: &mut App) {
        // TODO: these need to be sequential (exclusive system set)
        app.add_system(fps_controller_input)
            // .add_system(fps_controller_look)
            .add_system(fps_controller_move)
            .add_system(fps_controller_render);
    }
}

pub enum MoveMode {
    Noclip,
    Ground,
}

#[derive(Component)]
pub struct LogicalPlayer(pub u8);

#[derive(Component)]
pub struct RenderPlayer(pub u8);

#[derive(Default)]
pub struct FpsControllerSerial(u32);

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct FpsControllerInput {
    pub serial: u32,
    pub fly: bool,
    pub sprint: bool,
    pub jump: bool,
    pub crouch: bool,
    pub pitch: f32,
    pub yaw: f32,
    pub movement: Vec3,
}

#[derive(Component, Default)]
pub struct FpsControllerInputQueue {
    pub queue: VecDeque<FpsControllerInput>,
}

// #[derive(Component)]
pub struct FpsControllerConfig {
    pub sensitivity: f32,
    pub enable_input: bool,
    pub key_forward: KeyCode,
    pub key_back: KeyCode,
    pub key_left: KeyCode,
    pub key_right: KeyCode,
    pub key_up: KeyCode,
    pub key_down: KeyCode,
    pub key_sprint: KeyCode,
    pub key_jump: KeyCode,
    pub key_fly: KeyCode,
    pub key_crouch: KeyCode,
}

impl Default for FpsControllerConfig {
    fn default() -> Self {
        Self {
            enable_input: true,
            key_forward: KeyCode::W,
            key_back: KeyCode::S,
            key_left: KeyCode::A,
            key_right: KeyCode::D,
            key_up: KeyCode::Q,
            key_down: KeyCode::E,
            key_sprint: KeyCode::LShift,
            key_jump: KeyCode::Space,
            key_fly: KeyCode::F,
            key_crouch: KeyCode::LControl,
            sensitivity: 0.001,
        }
    }
}

#[derive(Component)]
pub struct FpsController {
    pub last_applied_serial: u32,
    pub move_mode: MoveMode,
    pub gravity: f32,
    pub walk_speed: f32,
    pub run_speed: f32,
    pub forward_speed: f32,
    pub side_speed: f32,
    pub air_speed_cap: f32,
    pub air_acceleration: f32,
    pub max_air_speed: f32,
    pub accel: f32,
    pub friction: f32,
    pub friction_cutoff: f32,
    pub jump_speed: f32,
    pub fly_speed: f32,
    pub fast_fly_speed: f32,
    pub fly_friction: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub velocity: Vec3,
    pub ground_tick: u8,
    pub stop_speed: f32,
    pub log_name: Option<&'static str>,
    pub apply_single: bool,
}

impl Default for FpsController {
    fn default() -> Self {
        Self {
            last_applied_serial: 0,
            move_mode: MoveMode::Ground,
            fly_speed: 10.0,
            fast_fly_speed: 30.0,
            gravity: 23.0,
            walk_speed: 10.0,
            run_speed: 30.0,
            forward_speed: 30.0,
            side_speed: 30.0,
            air_speed_cap: 2.0,
            air_acceleration: 20.0,
            max_air_speed: 8.0,
            accel: 10.0,
            friction: 10.0,
            friction_cutoff: 0.1,
            fly_friction: 0.5,
            pitch: 0.0,
            yaw: 0.0,
            velocity: Vec3::ZERO,
            ground_tick: 0,
            stop_speed: 1.0,
            jump_speed: 8.5,
            log_name: None,
            apply_single: false,
        }
    }
}

#[derive(Default)]
pub struct FrameTime(Duration);

impl FrameTime {
    pub fn new(time: Duration) -> FrameTime {
        FrameTime(time)
    }
}

impl std::fmt::Display for FrameTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let micros_per_frame = 1000000 / 60;
        let a = self.0.as_micros() / micros_per_frame;
        let b = (self.0.as_micros() % micros_per_frame) * 1000 / micros_per_frame;

        write!(f, "{}:{:03}", a, b)
    }
}

#[derive(Default, Component)]
pub struct FpsControllerLog {
    pos: BTreeMap<u32, Vec3>,
    log: Option<(&'static str, std::fs::File)>,
    last_put_time: Option<Instant>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExternalLogRecord {
    pub serial: u32,
    pub log_name: String,
    pub pos: Vec3,
    pub dt: Duration,
}

impl FpsControllerLog {
    pub fn put(&mut self, serial: u32, pos: &Vec3) {
        match self.pos.entry(serial) {
            std::collections::btree_map::Entry::Occupied(_) => (),
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(*pos);

                let now = Instant::now();
                // let d = self
                //     .last_put_time
                //     .map(|last| FrameTime::new(now.duration_since(last)))
                // .unwrap_or_default();
                let d = self
                    .last_put_time
                    .map(|last| now.duration_since(last))
                    .unwrap_or_default();

                if let Some((log_name, log_file)) = &mut self.log {
                    use std::io::Write;
                    // let _ = writeln!(log_file, "\"{:05}\",\"{}\",\"{}\"", serial, log_name, pos);
                    // let _ = writeln!(log_file, "{:05},{},{},{}", serial, log_name, pos.x, d);

                    let s = serde_json::to_string(&ExternalLogRecord {
                        serial,
                        log_name: log_name.to_owned(),
                        pos: *pos,
                        dt: d,
                    })
                    .unwrap();
                    writeln!(log_file, "{}", s);
                }

                self.last_put_time = Some(now);
            }
        };
    }

    pub fn discard(&mut self, serial: u32) {
        while let Some(e) = self.pos.first_entry() {
            if *e.key() >= serial {
                break;
            }
            debug!("discard: {}", e.key());
            e.remove();
        }
    }
    pub fn get_delta(&self, pos: &Vec3, serial: u32) -> Option<(f32, Vec3)> {
        if let Some(log_pos) = self.pos.get(&serial) {
            let delta = *log_pos - *pos;
            let delta_len = delta.length();

            let age = match self.pos.last_key_value() {
                Some((last, _)) if *last >= serial => Some(last - serial),
                _ => None,
            };

            // let velocity_len = velocity.linvel.length();
            info!(
                "delta: {} age {:?}",
                // velocity_len,
                delta_len,
                // delta_len / velocity_len,
                age,
            );
            Some((delta_len, delta))
            // if delta_len > 0.1 {
            //     if velocity.linvel.length() < 0.1 {
            //         info!("correction.");
            //         ent_transform.translation = transform.translation;
            //     }
            // }
        } else {
            None
        }
        // info!("player transform update: {:?} {:?}", transform, velocity);
    }
}

// fn format_frame_time(time: Duration) -> String {
//     format!("")
// }

// ██╗      ██████╗  ██████╗ ██╗ ██████╗
// ██║     ██╔═══██╗██╔════╝ ██║██╔════╝
// ██║     ██║   ██║██║  ███╗██║██║
// ██║     ██║   ██║██║   ██║██║██║
// ███████╗╚██████╔╝╚██████╔╝██║╚██████╗
// ╚══════╝ ╚═════╝  ╚═════╝ ╚═╝ ╚═════╝

const ANGLE_EPSILON: f32 = 0.001953125;

pub fn fps_controller_input(
    key_input: Res<Input<KeyCode>>,
    controller: Res<FpsControllerConfig>,
    mut serial: ResMut<FpsControllerSerial>,
    mut windows: ResMut<Windows>,
    mut mouse_events: EventReader<MouseMotion>,
    mut query: Query<&mut FpsControllerInputQueue>,
    mut event_writer: EventWriter<FpsControllerInput>,
) {
    if !controller.enable_input {
        return;
    }

    let mut input = FpsControllerInput::default();
    let window = windows.get_primary_mut().unwrap();
    if window.is_focused() {
        let mut mouse_delta = Vec2::ZERO;
        for mouse_event in mouse_events.iter() {
            mouse_delta += mouse_event.delta;
        }
        mouse_delta *= controller.sensitivity;

        input.pitch = (input.pitch - mouse_delta.y)
            .clamp(-FRAC_PI_2 + ANGLE_EPSILON, FRAC_PI_2 - ANGLE_EPSILON);
        input.yaw -= mouse_delta.x;
    }

    input.movement = Vec3::new(
        get_axis(&key_input, controller.key_right, controller.key_left),
        get_axis(&key_input, controller.key_up, controller.key_down),
        get_axis(&key_input, controller.key_forward, controller.key_back),
    );
    input.sprint = key_input.pressed(controller.key_sprint);
    input.jump = key_input.pressed(controller.key_jump);
    input.fly = key_input.just_pressed(controller.key_fly);
    input.crouch = key_input.pressed(controller.key_crouch);
    input.serial = serial.0;
    serial.0 += 1;

    for mut input_queue in query.iter_mut() {
        input_queue.queue.push_back(input.clone());
    }
    // info!("send: {}", input.serial);
    event_writer.send(input);
}

// pub fn fps_controller_look(mut query: Query<(&mut FpsController, &FpsControllerInput)>) {
//     for (mut controller, input) in query.iter_mut() {
//         controller.pitch = input.pitch;
//         controller.yaw = input.yaw;
//     }
// }

pub fn fps_controller_move(
    time: Res<Time>,
    physics_context: Res<RapierContext>,
    mut query: Query<(
        Entity,
        &mut FpsControllerInputQueue,
        &mut FpsController,
        &Collider,
        &mut Transform,
        &mut Velocity,
        &mut FpsControllerLog,
    )>,
) {
    let dt = time.delta_seconds();

    for (
        entity,
        mut input_queue,
        mut controller,
        collider,
        transform,
        mut velocity,
        mut controller_log,
    ) in query.iter_mut()
    {
        while let Some(input) = input_queue.queue.pop_front() {
            // HACK: store transform for last applied serial right before applying a new one, to get more realistic
            // estimate of position after the input has taken effect (after physics has run).
            // controller_log.put(controller.last_applied_serial, &transform.translation);

            controller_log.put(input.serial, &transform.translation);

            if input.fly {
                controller.move_mode = match controller.move_mode {
                    MoveMode::Noclip => MoveMode::Ground,
                    MoveMode::Ground => MoveMode::Noclip,
                }
            }

            let orientation = look_quat(input.pitch, input.yaw);
            let right = orientation * Vec3::X;
            let forward = orientation * -Vec3::Z;
            let position = transform.translation;

            match controller.move_mode {
                MoveMode::Noclip => {
                    if input.movement == Vec3::ZERO {
                        let friction = controller.fly_friction.clamp(0.0, 1.0);
                        controller.velocity *= 1.0 - friction;
                        if controller.velocity.length_squared() < 1e-6 {
                            controller.velocity = Vec3::ZERO;
                        }
                    } else {
                        let fly_speed = if input.sprint {
                            controller.fast_fly_speed
                        } else {
                            controller.fly_speed
                        };
                        controller.velocity = input.movement.normalize() * fly_speed;
                    }
                    velocity.linvel = controller.velocity.x * right
                        + controller.velocity.y * Vec3::Y
                        + controller.velocity.z * forward;
                }

                MoveMode::Ground => {
                    if let Some(capsule) = collider.as_capsule() {
                        let capsule = capsule.raw;
                        let mut start_velocity = controller.velocity;
                        let mut end_velocity = start_velocity;
                        let lateral_speed = start_velocity.xz().length();

                        // Capsule cast downwards to find ground
                        // Better than single raycast as it handles when you are near the edge of a surface
                        let mut ground_hit = None;
                        let cast_capsule = Collider::capsule(
                            capsule.segment.a.into(),
                            capsule.segment.b.into(),
                            capsule.radius * 1.0625,
                        );
                        let cast_velocity = Vec3::Y * -1.0;
                        let max_distance = 0.125;
                        // Avoid self collisions
                        let groups = QueryFilter::default().exclude_rigid_body(entity);

                        if let Some((_handle, hit)) = physics_context.cast_shape(
                            position,
                            orientation,
                            cast_velocity,
                            &cast_capsule,
                            max_distance,
                            groups,
                        ) {
                            ground_hit = Some(hit);
                        }

                        let mut wish_direction =
                            input.movement.z * controller.forward_speed * forward
                                + input.movement.x * controller.side_speed * right;
                        let mut wish_speed = wish_direction.length();
                        if wish_speed > 1e-6 {
                            // Avoid division by zero
                            wish_direction /= wish_speed; // Effectively normalize, avoid length computation twice
                        }

                        let max_speed = if input.sprint {
                            controller.run_speed
                        } else {
                            controller.walk_speed
                        };

                        wish_speed = f32::min(wish_speed, max_speed);

                        if let Some(_ground_hit) = ground_hit {
                            // Only apply friction after at least one tick, allows b-hopping without losing speed
                            if controller.ground_tick >= 1 {
                                if lateral_speed > controller.friction_cutoff {
                                    friction(
                                        lateral_speed,
                                        controller.friction,
                                        controller.stop_speed,
                                        dt,
                                        &mut end_velocity,
                                    );
                                } else {
                                    end_velocity.x = 0.0;
                                    end_velocity.z = 0.0;
                                }
                                end_velocity.y = 0.0;
                            }
                            accelerate(
                                wish_direction,
                                wish_speed,
                                controller.accel,
                                dt,
                                &mut end_velocity,
                            );
                            if input.jump {
                                // Simulate one update ahead, since this is an instant velocity change
                                start_velocity.y = controller.jump_speed;
                                end_velocity.y = start_velocity.y - controller.gravity * dt;
                            }
                            // Increment ground tick but cap at max value
                            controller.ground_tick = controller.ground_tick.saturating_add(1);
                        } else {
                            controller.ground_tick = 0;
                            wish_speed = f32::min(wish_speed, controller.air_speed_cap);
                            accelerate(
                                wish_direction,
                                wish_speed,
                                controller.air_acceleration,
                                dt,
                                &mut end_velocity,
                            );
                            end_velocity.y -= controller.gravity * dt;
                            let air_speed = end_velocity.xz().length();
                            if air_speed > controller.max_air_speed {
                                let ratio = controller.max_air_speed / air_speed;
                                end_velocity.x *= ratio;
                                end_velocity.z *= ratio;
                            }
                        }

                        // At this point our collider may be intersecting with the ground
                        // Fix up our collider by offsetting it to be flush with the ground
                        // if end_vel.y < -1e6 {
                        //     if let Some(ground_hit) = ground_hit {
                        //         let normal = Vec3::from(*ground_hit.normal2);
                        //         next_translation += normal * ground_hit.toi;
                        //     }
                        // }

                        controller.velocity = end_velocity;
                        velocity.linvel = (start_velocity + end_velocity) * 0.5;
                    }
                }
            }

            if let Some(log_name) = controller.log_name {
                debug!(
                    "applied: {}: {}: {} {:?}",
                    FrameTime::new(time.time_since_startup()),
                    log_name,
                    input.serial,
                    transform.translation
                );
            }
            controller.last_applied_serial = input.serial;
            if controller.apply_single {
                break;
            }
        }
        if input_queue.queue.len() > 2 {
            debug!(
                "queue size: {:?}: {}",
                controller.log_name,
                input_queue.queue.len()
            )
        }
    }
}

fn look_quat(pitch: f32, yaw: f32) -> Quat {
    Quat::from_euler(EulerRot::ZYX, 0.0, yaw, pitch)
}

fn friction(lateral_speed: f32, friction: f32, stop_speed: f32, dt: f32, velocity: &mut Vec3) {
    let control = f32::max(lateral_speed, stop_speed);
    let drop = control * friction * dt;
    let new_speed = f32::max((lateral_speed - drop) / lateral_speed, 0.0);
    velocity.x *= new_speed;
    velocity.z *= new_speed;
}

fn accelerate(wish_dir: Vec3, wish_speed: f32, accel: f32, dt: f32, velocity: &mut Vec3) {
    let velocity_projection = Vec3::dot(*velocity, wish_dir);
    let add_speed = wish_speed - velocity_projection;
    if add_speed <= 0.0 {
        return;
    }

    let accel_speed = f32::min(accel * wish_speed * dt, add_speed);
    let wish_direction = wish_dir * accel_speed;
    velocity.x += wish_direction.x;
    velocity.z += wish_direction.z;
}

fn get_pressed(key_input: &Res<Input<KeyCode>>, key: KeyCode) -> f32 {
    if key_input.pressed(key) {
        1.0
    } else {
        0.0
    }
}

fn get_axis(key_input: &Res<Input<KeyCode>>, key_pos: KeyCode, key_neg: KeyCode) -> f32 {
    get_pressed(key_input, key_pos) - get_pressed(key_input, key_neg)
}

// ██████╗ ███████╗███╗   ██╗██████╗ ███████╗██████╗
// ██╔══██╗██╔════╝████╗  ██║██╔══██╗██╔════╝██╔══██╗
// ██████╔╝█████╗  ██╔██╗ ██║██║  ██║█████╗  ██████╔╝
// ██╔══██╗██╔══╝  ██║╚██╗██║██║  ██║██╔══╝  ██╔══██╗
// ██║  ██║███████╗██║ ╚████║██████╔╝███████╗██║  ██║
// ╚═╝  ╚═╝╚══════╝╚═╝  ╚═══╝╚═════╝ ╚══════╝╚═╝  ╚═╝

pub fn fps_controller_render(
    logical_query: Query<
        (&Transform, &Collider, &FpsController, &LogicalPlayer),
        With<LogicalPlayer>,
    >,
    mut render_query: Query<(&mut Transform, &RenderPlayer), Without<LogicalPlayer>>,
) {
    // TODO: inefficient O(N^2) loop, use hash map?
    for (logical_transform, collider, controller, logical_player_id) in logical_query.iter() {
        if let Some(capsule) = collider.as_capsule() {
            for (mut render_transform, render_player_id) in render_query.iter_mut() {
                if logical_player_id.0 != render_player_id.0 {
                    continue;
                }
                // TODO: let this be more configurable
                let camera_height = capsule.segment().b().y + capsule.radius() * 0.75;
                render_transform.translation =
                    logical_transform.translation + Vec3::Y * camera_height;
                render_transform.rotation = look_quat(controller.pitch, controller.yaw);
            }
        }
    }
}

#[derive(Bundle)]
pub struct FpsControllerPhysicsBundle {
    collider: Collider,
    active_evnets: ActiveEvents,
    velocity: Velocity,
    rigid_body: RigidBody,
    sleeping: Sleeping,
    locked_axes: LockedAxes,
    additional_mass_properties: AdditionalMassProperties,
    gravity_scale: GravityScale,
    ccd: Ccd,
    // transform: Transform,
}
impl Default for FpsControllerPhysicsBundle {
    fn default() -> Self {
        Self {
            // collider: Collider::capsule(Vec3::Y * 0.5, Vec3::Y * 1.5, 0.5),
            collider: Collider::capsule_y(0.5, 0.5),
            active_evnets: ActiveEvents::COLLISION_EVENTS,
            velocity: Velocity::zero(),
            rigid_body: RigidBody::Dynamic,
            sleeping: Sleeping::disabled(),
            locked_axes: LockedAxes::ROTATION_LOCKED,
            additional_mass_properties: AdditionalMassProperties::Mass(1.0),
            gravity_scale: GravityScale(0.0),
            ccd: Ccd { enabled: true }, // Prevent clipping when going fas,
                                        // transform: Transform::from_xyz(0.0, 3.0, 0.0),
                                        // controller_log: default(),
        }
    }
}

#[derive(Bundle, Default)]
pub struct FpsControllerLocgicBundle {
    controller_log: FpsControllerLog,
    input_queue: FpsControllerInputQueue,
    controller: FpsController,
}

impl FpsControllerLocgicBundle {
    pub fn with_log_name(name: &'static str) -> Self {
        Self {
            controller_log: FpsControllerLog {
                log: std::fs::File::create(format!("{}.log", name))
                    .ok()
                    .map(|f| (name, f)),
                ..default()
            },
            input_queue: default(),
            controller: FpsController {
                log_name: Some(name),
                ..default()
            },
        }
    }
}
