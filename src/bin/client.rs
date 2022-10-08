#![feature(map_first_last)]

use std::{collections::HashMap, net::UdpSocket, time::SystemTime};

use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_egui::{EguiContext, EguiPlugin};
use bevy_rapier3d::prelude::*;
use bevy_renet::{
    renet::{ClientAuthentication, RenetClient, RenetError},
    run_if_client_connected, RenetClientPlugin,
};
use renet_test::{
    client_connection_config,
    controller::{self, FpsControllerPhysicsBundle},
    exit_on_esc_system,
    frame::NetworkFrame,
    predict::VelocityExtrapolate,
    setup_level, ClientChannel, ObjectType, PlayerCommand, ServerChannel, ServerMessages,
    PROTOCOL_ID,
};
use renet_visualizer::{RenetClientVisualizer, RenetVisualizerStyle};
use smooth_bevy_cameras::LookTransformPlugin;

#[derive(Default)]
struct NetworkMapping(HashMap<Entity, Entity>);

#[derive(Debug)]
struct PlayerInfo {
    client_entity: Entity,
    server_entity: Entity,
}

#[derive(Debug, Default)]
struct ClientLobby {
    players: HashMap<u64, PlayerInfo>,
}

#[derive(Debug)]
struct MostRecentTick {
    from_server: u32,
    predicted: u32,
}

#[derive(Component, Default, Debug)]
struct TransformFromServer(Transform);

fn new_renet_client() -> RenetClient {
    let server_addr = "127.0.0.1:5000".parse().unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let connection_config = client_connection_config();
    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let client_id = current_time.as_millis() as u64;
    info!("client id 1: {}", client_id);
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr,
        user_data: None,
    };

    RenetClient::new(
        current_time,
        socket,
        client_id,
        connection_config,
        authentication,
    )
    .unwrap()
}

fn main() {
    // tracing_subscriber::fmt().compact().init();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_plugin(RenetClientPlugin);
    app.add_plugin(LookTransformPlugin);
    app.add_plugin(FrameTimeDiagnosticsPlugin::default());
    // app.add_plugin(LogDiagnosticsPlugin::default());
    app.add_plugin(EguiPlugin);
    // app.add_plugin(controller::FpsControllerPlugin);
    app.add_plugin(RapierPhysicsPlugin::<NoUserData>::default())
        .add_plugin(RapierDebugRenderPlugin::default());
    app.add_event::<PlayerCommand>();
    app.add_event::<controller::FpsControllerInput>();

    app.insert_resource(ClientLobby::default());
    app.init_resource::<controller::FpsControllerConfig>();
    app.init_resource::<controller::FpsControllerSerial>();

    app.insert_resource(new_renet_client());
    app.insert_resource(NetworkMapping::default());

    app.add_system(controller::fps_controller_input);
    app.add_system(controller::fps_controller_move.after(controller::fps_controller_input));

    app.add_system(player_input);
    app.add_system(renet_test::camera::camera_follow);
    app.add_system(renet_test::camera::update_target_system);
    app.add_system(client_send_input.with_run_criteria(run_if_client_connected));
    app.add_system(client_send_player_commands.with_run_criteria(run_if_client_connected));
    app.add_system(client_sync_players.with_run_criteria(run_if_client_connected));
    app.add_system(
        predict_entities
            .with_run_criteria(run_if_client_connected)
            .after(client_sync_players),
    );

    app.add_system(exit_on_esc_system);

    app.insert_resource(RenetClientVisualizer::<200>::new(
        RenetVisualizerStyle::default(),
    ));
    app.add_system(update_visulizer_system);

    app.add_startup_system(setup_level);
    app.add_startup_system(renet_test::camera::setup_camera);
    app.add_startup_system(renet_test::camera::setup_target);
    app.add_system(panic_on_error_system);

    app.run();
}

// If any error is found we just panic
fn panic_on_error_system(mut renet_error: EventReader<RenetError>) {
    for e in renet_error.iter() {
        panic!("{}", e);
    }
}

fn update_visulizer_system(
    mut egui_context: ResMut<EguiContext>,
    mut visualizer: ResMut<RenetClientVisualizer<200>>,
    client: Res<RenetClient>,
    mut show_visualizer: Local<bool>,
    keyboard_input: Res<Input<KeyCode>>,
) {
    visualizer.add_network_info(client.network_info());
    if keyboard_input.just_pressed(KeyCode::F1) {
        *show_visualizer = !*show_visualizer;
    }
    if *show_visualizer {
        visualizer.show_window(egui_context.ctx_mut());
    }
}

/// read input for event based user input (enqueue PlayerCommand::BasicAttack)
fn player_input(
    mouse_button_input: Res<Input<MouseButton>>,
    target_query: Query<&Transform, With<renet_test::WorldSpacePointer>>,
    mut player_commands: EventWriter<PlayerCommand>,
) {
    if mouse_button_input.just_pressed(MouseButton::Left) {
        let target_transform = target_query.single();
        player_commands.send(PlayerCommand::BasicAttack {
            cast_at: target_transform.translation,
        });
    }
    // info!("most recent tick: {:?}", most_recent_tick);
}

/// serialize and send FpsControllerInput to server on ClientChannel::Input
fn client_send_input(
    mut client: ResMut<RenetClient>,
    mut event_reader: EventReader<controller::FpsControllerInput>,
) {
    for input in event_reader.iter() {
        let input_message = bincode::serialize(input).unwrap();
        client.send_message(ClientChannel::Input.id(), input_message);
    }
}

/// serialize and send PlayerCommand to server on ClientChannel::Command
fn client_send_player_commands(
    mut player_commands: EventReader<PlayerCommand>,
    mut client: ResMut<RenetClient>,
) {
    for command in player_commands.iter() {
        let command_message = bincode::serialize(command).unwrap();
        client.send_message(ClientChannel::Command.id(), command_message);
    }
}

/// receive ServerChannel::ServerMessage:
/// - PlayerCreate
/// - PlayerRemove
/// - SpawnProjectile (directly spawn entity)
/// - DespawnProjectile (directly de-spawn entity)
///
/// receive ServerChannel::NetworkFrame
/// - update most_recent_tick
/// - deserialize & apply transformation updates to entities
///

#[allow(clippy::too_many_arguments)]
fn client_sync_players(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut client: ResMut<RenetClient>,
    mut lobby: ResMut<ClientLobby>,
    mut network_mapping: ResMut<NetworkMapping>,
    mut most_recent_tick: Option<ResMut<MostRecentTick>>,
    mut transform_query: Query<&mut Transform, Without<renet_test::ControlledPlayer>>,
    mut controlled_player: Query<
        (
            &mut controller::FpsController,
            &mut controller::FpsControllerLog,
            &mut Transform,
            &mut Velocity,
        ),
        With<renet_test::ControlledPlayer>,
    >,
    mut extrapolate: Query<
        (&mut TransformFromServer, &mut VelocityExtrapolate),
        Without<renet_test::ControlledPlayer>,
    >,
) {
    let client_id = client.client_id();
    while let Some(message) = client.receive_message(ServerChannel::ServerMessages.id()) {
        let server_message = bincode::deserialize(&message).unwrap();
        match server_message {
            ServerMessages::PlayerCreate {
                id,
                translation,
                entity,
            } => {
                info!("Player {} connected. {}", id, client_id);
                let mut client_entity = commands.spawn_bundle(PbrBundle {
                    mesh: meshes.add(Mesh::from(shape::Capsule::default())),
                    material: materials.add(Color::rgb(0.8, 0.7, 0.6).into()),
                    transform: Transform::from_xyz(translation[0], translation[1], translation[2]),
                    ..Default::default()
                });

                if client_id == id {
                    info!("controlled player");
                    client_entity
                        .insert(renet_test::ControlledPlayer)
                        // .insert(PlayerInputQueue::default());
                        .insert_bundle(FpsControllerPhysicsBundle::default())
                        .insert(
                            controller::FpsControllerInputQueue::default(), //  {
                                                                            //     pitch: -TAU / 12.0,
                                                                            //     yaw: TAU * 5.0 / 8.0,
                                                                            //     ..default()
                                                                            // }
                        )
                        .insert(controller::FpsController {
                            log_name: Some("client"),
                            ..default()
                        })
                        // .insert(Transform::from_xyz(0.0, 3.0, 0.0))
                        ;
                } else {
                    client_entity.insert(VelocityExtrapolate::default());
                }

                client_entity.insert(TransformFromServer::default());
                let player_info = PlayerInfo {
                    server_entity: entity,
                    client_entity: client_entity.id(),
                };
                lobby.players.insert(id, player_info);
                network_mapping.0.insert(entity, client_entity.id());
            }
            ServerMessages::PlayerRemove { id } => {
                println!("Player {} disconnected.", id);
                if let Some(PlayerInfo {
                    server_entity,
                    client_entity,
                }) = lobby.players.remove(&id)
                {
                    commands.entity(client_entity).despawn();
                    network_mapping.0.remove(&server_entity);
                }
            }
            ServerMessages::SpawnProjectile {
                entity,
                translation,
                object_type: ObjectType::Projectile,
            } => {
                let mut projectile_entity = commands.spawn_bundle(PbrBundle {
                    mesh: meshes.add(Mesh::from(shape::Icosphere {
                        radius: 0.1,
                        subdivisions: 5,
                    })),
                    material: materials.add(Color::rgb(1.0, 0.0, 0.0).into()),
                    transform: Transform::from_translation(translation),
                    ..Default::default()
                });
                projectile_entity
                    .insert(TransformFromServer::default())
                    .insert(VelocityExtrapolate::default());
                network_mapping.0.insert(entity, projectile_entity.id());
            }
            ServerMessages::SpawnProjectile {
                entity,
                translation,
                object_type: ObjectType::Box,
            } => {
                info!("spawn box");
                let mut bundle = ObjectType::Box.representation_bundle(&mut meshes, &mut materials);
                bundle.transform = Transform::from_translation(translation);

                let mut projectile_entity = commands.spawn_bundle(bundle);
                projectile_entity
                    .insert(TransformFromServer::default())
                    .insert(VelocityExtrapolate::default());
                network_mapping.0.insert(entity, projectile_entity.id());
            }
            ServerMessages::DespawnProjectile { entity } => {
                if let Some(entity) = network_mapping.0.remove(&entity) {
                    commands.entity(entity).despawn();
                }
            }
        }
    }

    while let Some(message) = client.receive_message(ServerChannel::NetworkFrame.id()) {
        let frame: NetworkFrame = bincode::deserialize(&message).unwrap();
        // info!("network frame");
        match most_recent_tick {
            None => {
                commands.insert_resource(MostRecentTick {
                    from_server: frame.tick,
                    predicted: frame.tick,
                });
            }
            Some(ref mut tick) if tick.from_server < frame.tick => {
                tick.from_server = frame.tick;
                tick.predicted = frame.tick;
                //  = Some(MostRecentTick {
                //     from_server: frame.tick,
                //     predicted: frame.tick,
                // })
            }
            _ => continue,
        }

        for i in 0..frame.entities.entities.len() {
            debug!(
                "entity {} {:?} -> {:?}",
                i,
                frame.entities.entities[i],
                network_mapping.0.get(&frame.entities.entities[i])
            );

            if let Some(entity) = network_mapping.0.get(&frame.entities.entities[i]) {
                let translation = frame.entities.translations[i];
                // let rotation = frame.entities.rotations[i];
                let transform = Transform {
                    translation,
                    // rotation,
                    ..Default::default()
                };

                if let Ok((
                    mut fps_controller,
                    mut controller_log,
                    mut ent_transform,
                    mut velocity,
                )) = controlled_player.get_mut(*entity)
                {
                    // *player_transform = transform;
                    velocity.linvel = frame.entities.velocities[i];

                    fps_controller.last_applied_serial = frame.last_player_input;
                    if let Some(log_pos) = controller_log
                        .pos
                        .get(&(fps_controller.last_applied_serial))
                    {
                        let delta = *log_pos - transform.translation;
                        let delta_len = delta.length();
                        let velocity_len = velocity.linvel.length();

                        let age = match controller_log.pos.last_key_value() {
                            Some((last, _)) if *last >= frame.last_player_input => {
                                Some(last - frame.last_player_input)
                            }
                            _ => None,
                        };

                        while let Some(e) = controller_log.pos.first_entry() {
                            if *e.key() >= frame.last_player_input {
                                break;
                            }
                            debug!("discard: {}", e.key());
                            e.remove();
                        }

                        info!(
                            "delta: {} {} {} age {:?}",
                            velocity_len,
                            delta_len,
                            delta_len / velocity_len,
                            age,
                        );

                        if delta_len > 0.1 {
                            if velocity.linvel.length() < 0.1 {
                                info!("correction.");
                                ent_transform.translation = transform.translation;
                            }
                        }
                    }
                    // info!("player transform update: {:?} {:?}", transform, velocity);
                }
                if let Ok(mut ent_transform) = transform_query.get_mut(*entity) {
                    info!(
                        "apply transform {} {:?} -> {:?} {:?}",
                        frame.last_player_input,
                        entity,
                        transform.translation,
                        ent_transform.translation
                    );
                    *ent_transform = transform;
                }
                if let Ok((mut transform_from_server, mut extrapolate)) =
                    extrapolate.get_mut(*entity)
                {
                    *transform_from_server = TransformFromServer(transform);
                    extrapolate.base_tick = frame.tick;
                    extrapolate.velocity = frame.entities.velocities[i];
                }
            }
        }
        for i in 0..frame.with_rotation.entities.len() {
            debug!(
                "entity {} {:?} -> {:?}",
                i,
                frame.with_rotation.entities[i],
                network_mapping.0.get(&frame.with_rotation.entities[i])
            );

            if let Some(entity) = network_mapping.0.get(&frame.with_rotation.entities[i]) {
                let translation = frame.with_rotation.translations[i];
                let rotation = frame.with_rotation.rotations[i];
                let transform = Transform {
                    translation,
                    rotation,
                    ..Default::default()
                };

                if let Ok(old_transform) = transform_query.get(*entity) {
                    debug!(
                        "apply transform {} {:?} -> {:?} {:?}",
                        frame.last_player_input,
                        entity,
                        transform.translation,
                        old_transform.translation
                    );
                }

                if let Ok(mut ent_transform) = transform_query.get_mut(*entity) {
                    *ent_transform = transform;
                }
                if let Ok((mut transform_from_server, mut extrapolate)) =
                    extrapolate.get_mut(*entity)
                {
                    *transform_from_server = TransformFromServer(transform);
                    extrapolate.base_tick = frame.tick;
                    extrapolate.velocity = frame.with_rotation.velocities[i];
                }
            }
        }
    }
}

fn predict_entities(
    most_recent_tick: Option<ResMut<MostRecentTick>>,
    mut transform_query: Query<(&mut Transform, &TransformFromServer, &VelocityExtrapolate)>,
) {
    if let Some(mut tick) = most_recent_tick {
        for (mut transform, transform_from_server, extrapolate) in &mut transform_query {
            transform.translation =
                extrapolate.apply(tick.predicted, transform_from_server.0.translation);
            debug!(
                "predict: {:?} {:?} {:?}",
                transform.translation, transform_from_server, extrapolate
            );
        }

        tick.predicted += 1;
    }
}
