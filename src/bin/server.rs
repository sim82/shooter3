use std::{
    collections::{HashMap, VecDeque},
    net::UdpSocket,
    time::SystemTime,
};

use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, math::Vec3Swizzles, prelude::*};
use bevy_egui::{EguiContext, EguiPlugin};
use bevy_rapier3d::prelude::*;
use bevy_renet::{
    renet::{RenetServer, ServerAuthentication, ServerConfig, ServerEvent},
    RenetServerPlugin,
};
use renet_test::{
    exit_on_esc_system, frame::NetworkFrame, server_connection_config, setup_level, spawn_fireball,
    ClientChannel, ObjectType, Player, PlayerCommand, PlayerInput, Projectile, ServerChannel,
    ServerMessages, PLAYER_MOVE_SPEED, PROTOCOL_ID,
};
use renet_visualizer::RenetServerVisualizer;

#[derive(Debug, Default)]
pub struct ServerLobby {
    pub players: HashMap<u64, Entity>,
}

#[derive(Debug, Default)]
struct NetworkTick(u32);

// Clients last received ticks
#[derive(Debug, Default)]
struct ClientTicks(HashMap<u64, Option<u32>>);

fn new_renet_server() -> RenetServer {
    let server_addr = "127.0.0.1:5000".parse().unwrap();
    let socket = UdpSocket::bind(server_addr).unwrap();
    let connection_config = server_connection_config();
    let server_config =
        ServerConfig::new(64, PROTOCOL_ID, server_addr, ServerAuthentication::Unsecure);
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    RenetServer::new(current_time, server_config, connection_config, socket).unwrap()
}

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);

    app.add_plugin(RenetServerPlugin)
        .add_plugin(RapierPhysicsPlugin::<NoUserData>::default())
        .add_plugin(RapierDebugRenderPlugin::default())
        .add_plugin(FrameTimeDiagnosticsPlugin::default())
        .add_plugin(EguiPlugin);

    app.insert_resource(ServerLobby::default())
        .insert_resource(NetworkTick(0))
        .insert_resource(ClientTicks::default())
        .insert_resource(new_renet_server())
        .insert_resource(RenetServerVisualizer::<200>::default())
        .insert_resource(SendTickTimer(Timer::from_seconds(5.0 / 60.0, true)))
        .insert_resource(AddCubeTimer(Timer::from_seconds(1.0, true)));

    app.add_system(server_update_system)
        .add_system(server_network_sync)
        .add_system(move_players_system)
        .add_system(update_projectiles_system)
        .add_system(update_visulizer_system)
        .add_system(despawn_projectile_system)
        .add_system(exit_on_esc_system)
        // .add_system(add_cube_system)
        ;

    app.add_system_to_stage(CoreStage::PostUpdate, projectile_on_removal_system);

    app.add_startup_system(setup_level)
        .add_startup_system(setup_simple_camera);

    app.run();
}

#[derive(Component, Default)]
struct PlayerInputQueue {
    queue: VecDeque<PlayerInput>,
    last_applied_serial: u32,
}

#[derive(Component, Default)]
struct PlayerVelocity {
    velocity: Vec3,
}

///
/// recive ServerEvent
/// - ClientConnected
/// - ClientDisconnected
///
/// receive ClientChannel::Command
/// - PlayerCommand
/// - PlayerInput: put nnto player entity as component
#[allow(clippy::too_many_arguments)]
fn server_update_system(
    mut server_events: EventReader<ServerEvent>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lobby: ResMut<ServerLobby>,
    mut server: ResMut<RenetServer>,
    mut visualizer: ResMut<RenetServerVisualizer<200>>,
    mut client_ticks: ResMut<ClientTicks>,
    mut players: Query<(Entity, &Player, &Transform, &mut PlayerInputQueue)>,
) {
    for event in server_events.iter() {
        match event {
            ServerEvent::ClientConnected(id, _) => {
                info!("Player {} connected.", id);
                visualizer.add_client(*id);

                // Initialize other players for this new client
                for (entity, player, transform, _) in players.iter() {
                    // let translation: [f32; 3] = transform.translation.into();
                    let message = bincode::serialize(&ServerMessages::PlayerCreate {
                        id: player.id,
                        entity,
                        translation: transform.translation,
                    })
                    .unwrap();
                    server.send_message(*id, ServerChannel::ServerMessages.id(), message);
                }

                // Spawn new player
                let transform = Transform::from_xyz(0.0, 0.51, 0.0);
                let player_entity = commands
                    .spawn_bundle(PbrBundle {
                        mesh: meshes.add(Mesh::from(shape::Capsule::default())),
                        material: materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
                        transform,
                        ..Default::default()
                    })
                    .insert(RigidBody::Dynamic)
                    .insert(
                        LockedAxes::ROTATION_LOCKED, /*| LockedAxes::TRANSLATION_LOCKED_Y*/
                    )
                    .insert(Collider::capsule_y(0.5, 0.5))
                    .insert(PlayerInput::default())
                    // .insert(Velocity::default())
                    .insert(PlayerInputQueue::default())
                    .insert(PlayerVelocity::default())
                    .insert(Player { id: *id })
                    .insert(ExternalImpulse::default())
                    .id();

                lobby.players.insert(*id, player_entity);

                // let translation: [f32; 3] = transform.translation.into();
                let message = bincode::serialize(&ServerMessages::PlayerCreate {
                    id: *id,
                    entity: player_entity,
                    translation: transform.translation,
                })
                .unwrap();
                server.broadcast_message(ServerChannel::ServerMessages.id(), message);
            }
            ServerEvent::ClientDisconnected(id) => {
                println!("Player {} disconnected.", id);
                visualizer.remove_client(*id);
                client_ticks.0.remove(id);
                if let Some(player_entity) = lobby.players.remove(id) {
                    commands.entity(player_entity).despawn();
                }

                let message =
                    bincode::serialize(&ServerMessages::PlayerRemove { id: *id }).unwrap();
                server.broadcast_message(ServerChannel::ServerMessages.id(), message);
            }
        }
    }

    for client_id in server.clients_id().into_iter() {
        while let Some(message) = server.receive_message(client_id, ClientChannel::Command.id()) {
            let command: PlayerCommand = bincode::deserialize(&message).unwrap();
            match command {
                PlayerCommand::BasicAttack { mut cast_at } => {
                    println!(
                        "Received basic attack from client {}: {:?}",
                        client_id, cast_at
                    );

                    if let Some(player_entity) = lobby.players.get(&client_id) {
                        if let Ok((_, _, player_transform, _)) = players.get(*player_entity) {
                            cast_at[1] = player_transform.translation[1];

                            let direction =
                                (cast_at - player_transform.translation).normalize_or_zero();
                            let mut translation = player_transform.translation + (direction * 0.7);
                            translation[1] = 1.0;

                            let fireball_entity = spawn_fireball(
                                &mut commands,
                                &mut meshes,
                                &mut materials,
                                translation,
                                direction,
                            );
                            let message = ServerMessages::SpawnProjectile {
                                entity: fireball_entity,
                                translation,
                                object_type: ObjectType::Projectile,
                            };
                            let message = bincode::serialize(&message).unwrap();
                            // info!("spawn projectile: {}", message.len());
                            server.broadcast_message(ServerChannel::ServerMessages.id(), message);
                        }
                    }
                }
            }
        }
        while let Some(message) = server.receive_message(client_id, ClientChannel::Input.id()) {
            let input: PlayerInput = bincode::deserialize(&message).unwrap();
            client_ticks.0.insert(client_id, input.most_recent_tick);
            if let Some(player_entity) = lobby.players.get(&client_id) {
                if let Ok((_, _, _, mut player_input_queue)) = players.get_mut(*player_entity) {
                    // commands.entity(*player_entity).insert(input);
                    player_input_queue.queue.push_back(input)
                }
            }
        }
    }
}

fn update_projectiles_system(
    mut commands: Commands,
    mut projectiles: Query<(Entity, &mut Projectile)>,
    time: Res<Time>,
) {
    for (entity, mut projectile) in projectiles.iter_mut() {
        projectile.duration.tick(time.delta());
        if projectile.duration.finished() {
            commands.entity(entity).despawn();
        }
    }
}

fn update_visulizer_system(
    mut egui_context: ResMut<EguiContext>,
    mut visualizer: ResMut<RenetServerVisualizer<200>>,
    server: Res<RenetServer>,
) {
    visualizer.update(&server);
    visualizer.show_window(egui_context.ctx_mut());
}

struct SendTickTimer(Timer);

/// send out NetworkFrame messages to clients
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn server_network_sync(
    mut tick: ResMut<NetworkTick>,
    mut server: ResMut<RenetServer>,
    time: Res<Time>,
    mut timer: ResMut<SendTickTimer>,
    players: Query<
        (Entity, &Transform, &PlayerVelocity),
        (Without<Projectile>, With<Player>, Without<CubeMarker>),
    >,
    projectiles: Query<
        (Entity, &Transform, &Velocity),
        (With<Projectile>, Without<Player>, Without<CubeMarker>),
    >,
    cubes: Query<
        (Entity, &Transform, &Velocity),
        (Without<Projectile>, Without<Player>, With<CubeMarker>),
    >,
    player_query: Query<(&PlayerInputQueue, &Player)>,
) {
    let mut frame = NetworkFrame::default();

    for (entity, transform, velocity) in players.iter() {
        frame.entities.entities.push(entity);
        frame.entities.translations.push(transform.translation);
        frame.entities.velocities.push(velocity.velocity);
        // frame.entities.rotations.push(default());
    }

    for (entity, transform, velocity) in projectiles.iter() {
        frame.entities.entities.push(entity);
        frame.entities.translations.push(transform.translation);
        frame.entities.velocities.push(velocity.linvel);
        // frame.entities.rotations.push(default());
    }

    for (entity, transform, velocity) in cubes.iter() {
        frame.with_rotation.entities.push(entity);
        frame.with_rotation.translations.push(transform.translation);
        frame.with_rotation.velocities.push(velocity.linvel);
        frame.with_rotation.rotations.push(transform.rotation);
        // info!("rot: {:?}", velocity.angvel);
    }

    frame.tick = tick.0;
    tick.0 += 1;
    // info!("tick: {}", tick.0);
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        for (player_input_queue, player) in &player_query {
            frame.last_player_input = player_input_queue.last_applied_serial;
            let sync_message = bincode::serialize(&frame).unwrap();
            // server.broadcast_message(ServerChannel::NetworkFrame.id(), sync_message);
            server.send_message(player.id, ServerChannel::NetworkFrame.id(), sync_message);
        }
    }
}

// apply PlayerInput to client entities
fn move_players_system(
    mut query: Query<(
        &mut Transform,
        &mut PlayerInputQueue,
        &mut PlayerVelocity,
        &mut ExternalImpulse,
    )>,
) {
    for (mut transform, mut input_queue, mut player_velocity, mut impulse) in query.iter_mut() {
        while let Some(input) = input_queue.queue.pop_front() {
            debug!("apply player input: {}", input.serial);
            let x = (input.right as i8 - input.left as i8) as f32;
            let y = (input.down as i8 - input.up as i8) as f32;
            let direction = Vec2::new(x, y).normalize_or_zero();
            let offs = direction * PLAYER_MOVE_SPEED; // * (1.0 / 60.0);
                                                      // transform.translation.x += offs.x;
                                                      // transform.translation.z += offs.y;
            impulse.impulse.x = offs.x;
            impulse.impulse.z = offs.y;

            player_velocity.velocity = (direction * PLAYER_MOVE_SPEED).extend(0.0).xzy();
            input_queue.last_applied_serial = input.serial;
            // velocity.linvel.x = direction.x * PLAYER_MOVE_SPEED;
            // velocity.linvel.z = direction.y * PLAYER_MOVE_SPEED;
        }
    }
}

pub fn setup_simple_camera(mut commands: Commands) {
    // camera
    commands.spawn_bundle(Camera3dBundle {
        transform: Transform::from_xyz(-5.5, 5.0, 5.5).looking_at(Vec3::ZERO, Vec3::Y),
        ..Default::default()
    });
}

fn despawn_projectile_system(
    mut commands: Commands,
    mut collision_events: EventReader<CollisionEvent>,
    projectile_query: Query<Option<&Projectile>>,
) {
    for collision_event in collision_events.iter() {
        if let CollisionEvent::Started(entity1, entity2, _) = collision_event {
            if let Ok(Some(_)) = projectile_query.get(*entity1) {
                commands.entity(*entity1).despawn();
            }
            if let Ok(Some(_)) = projectile_query.get(*entity2) {
                commands.entity(*entity2).despawn();
            }
        }
    }
}

fn projectile_on_removal_system(
    mut server: ResMut<RenetServer>,
    removed_projectiles: RemovedComponents<Projectile>,
) {
    for entity in removed_projectiles.iter() {
        let message = ServerMessages::DespawnProjectile { entity };
        info!("message {:?}", message);

        let message = bincode::serialize(&message).unwrap();
        info!("message {:?}", message);
        server.broadcast_message(ServerChannel::ServerMessages.id(), message);
    }
}

struct AddCubeTimer(Timer);
#[derive(Component)]
struct CubeMarker;

fn add_cube_system(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<AddCubeTimer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut server: ResMut<RenetServer>,
) {
    timer.0.tick(time.delta());

    if timer.0.just_finished() {
        let bundle = ObjectType::Box.representation_bundle(&mut meshes, &mut materials);
        let translation = bundle.transform.translation;
        let cube_entity = commands
            .spawn_bundle(bundle)
            .insert(RigidBody::Dynamic)
            .insert(Collider::cuboid(0.1, 0.1, 0.1))
            .insert(CubeMarker)
            .insert(Velocity::default())
            .id();

        let message = ServerMessages::SpawnProjectile {
            entity: cube_entity,
            translation,
            object_type: ObjectType::Box,
        };
        let message = bincode::serialize(&message).unwrap();
        // info!("spawn projectile: {}", message.len());
        server.broadcast_message(ServerChannel::ServerMessages.id(), message);
    }
}
