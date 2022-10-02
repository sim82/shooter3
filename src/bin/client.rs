use std::{
    collections::{HashMap, VecDeque},
    net::UdpSocket,
    time::SystemTime,
};

use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_egui::{EguiContext, EguiPlugin};
use bevy_renet::{
    renet::{ClientAuthentication, RenetClient, RenetError},
    run_if_client_connected, RenetClientPlugin,
};
use renet_test::{
    client_connection_config, exit_on_esc_system, predict::VelocityExtrapolate, setup_level,
    ClientChannel, NetworkFrame, PlayerCommand, PlayerInput, Ray3d, ServerChannel, ServerMessages,
    PLAYER_MOVE_SPEED, PROTOCOL_ID,
};
use renet_visualizer::{RenetClientVisualizer, RenetVisualizerStyle};
use smooth_bevy_cameras::{LookTransform, LookTransformBundle, LookTransformPlugin, Smoother};

#[derive(Component)]
struct ControlledPlayer;

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

#[derive(Component, Default)]
struct PlayerInputQueue {
    queue: VecDeque<PlayerInput>,
    last_server_serial: u32,
}

#[derive(Component, Default, Debug)]
struct TransformFromServer(Transform);

#[derive(Component)]
struct Target;

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
    // ran
    // let client_id = rand::thread_rng().gen();
    // info!("client id 2: {}", client_id);

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
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_plugin(RenetClientPlugin);
    app.add_plugin(LookTransformPlugin);
    app.add_plugin(FrameTimeDiagnosticsPlugin::default());
    // app.add_plugin(LogDiagnosticsPlugin::default());
    app.add_plugin(EguiPlugin);

    app.add_event::<PlayerCommand>();

    app.insert_resource(ClientLobby::default());
    app.insert_resource(PlayerInput::default());

    app.insert_resource(new_renet_client());
    app.insert_resource(NetworkMapping::default());
    // app.insert_resource(PlayerInputQueue::default());

    app.add_system(player_input);
    app.add_system(camera_follow);
    app.add_system(update_target_system);
    app.add_system(client_send_input.with_run_criteria(run_if_client_connected));
    app.add_system(client_send_player_commands.with_run_criteria(run_if_client_connected));
    app.add_system(client_sync_players.with_run_criteria(run_if_client_connected));
    app.add_system(
        client_predict_input
            .with_run_criteria(run_if_client_connected)
            .after(player_input)
            .after(client_sync_players),
    )
    .add_system(
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
    app.add_startup_system(setup_camera);
    app.add_startup_system(setup_target);
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

/// read input into PlayerInput resource and enqueue PlayerCommand::BasicAttack
// #[allow(clippy::too_many_arguments)]
fn player_input(
    keyboard_input: Res<Input<KeyCode>>,
    mut player_input: ResMut<PlayerInput>,
    mouse_button_input: Res<Input<MouseButton>>,
    target_query: Query<&Transform, With<Target>>,
    mut player_commands: EventWriter<PlayerCommand>,
    most_recent_tick: Option<Res<MostRecentTick>>,
) {
    player_input.serial += 1;
    player_input.left = keyboard_input.pressed(KeyCode::A) || keyboard_input.pressed(KeyCode::Left);
    player_input.right =
        keyboard_input.pressed(KeyCode::D) || keyboard_input.pressed(KeyCode::Right);
    player_input.up = keyboard_input.pressed(KeyCode::W) || keyboard_input.pressed(KeyCode::Up);
    player_input.down = keyboard_input.pressed(KeyCode::S) || keyboard_input.pressed(KeyCode::Down);
    player_input.most_recent_tick = most_recent_tick.as_ref().map(|tick| tick.from_server);

    if mouse_button_input.just_pressed(MouseButton::Left) {
        let target_transform = target_query.single();
        player_commands.send(PlayerCommand::BasicAttack {
            cast_at: target_transform.translation,
        });
    }
    // info!("most recent tick: {:?}", most_recent_tick);
}

/// serialize and send PlayerInput to server on ClientChannel::Input
fn client_send_input(
    player_input: Res<PlayerInput>,
    mut client: ResMut<RenetClient>,
    mut player_input_queue: Query<&mut PlayerInputQueue, With<ControlledPlayer>>,
) {
    if let Ok(mut player_input_queue) = player_input_queue.get_single_mut() {
        player_input_queue.queue.push_back(*player_input);
    }
    {
        let input_message = bincode::serialize(&*player_input).unwrap();
        client.send_message(ClientChannel::Input.id(), input_message);
    }
    // let input_message = bincode::serialize(&*player_input).unwrap();
    // client.send_message(ClientChannel::Input.id(), input_message);
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
    mut transform_query: Query<&mut Transform>,
    mut controlled_player: Query<
        (&mut PlayerInputQueue, &mut TransformFromServer),
        With<ControlledPlayer>,
    >,
    mut extrapolate: Query<
        (&mut TransformFromServer, &mut VelocityExtrapolate),
        Without<ControlledPlayer>,
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
                        .insert(ControlledPlayer)
                        .insert(PlayerInputQueue::default());
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
            ServerMessages::DespawnProjectile { entity } => {
                if let Some(entity) = network_mapping.0.remove(&entity) {
                    commands.entity(entity).despawn();
                }
            }
        }
    }

    while let Some(message) = client.receive_message(ServerChannel::NetworkFrame.id()) {
        let frame: NetworkFrame = bincode::deserialize(&message).unwrap();
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
            info!(
                "entity {} {:?} -> {:?}",
                i,
                frame.entities.entities[i],
                network_mapping.0.get(&frame.entities.entities[i])
            );

            if let Some(entity) = network_mapping.0.get(&frame.entities.entities[i]) {
                let translation = frame.entities.translations[i];
                let transform = Transform {
                    translation,
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

                if let Ok((mut player_input_queue, mut transform_from_server)) =
                    controlled_player.get_mut(*entity)
                {
                    *transform_from_server = TransformFromServer(transform);
                    player_input_queue.last_server_serial = frame.last_player_input;
                }
                if let Ok(mut ent_transform) = transform_query.get_mut(*entity) {
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
    }
}

fn client_predict_input(
    mut transform_query: Query<
        (&mut Transform, &TransformFromServer, &mut PlayerInputQueue),
        With<ControlledPlayer>,
    >,
    // most_recent_tick: Option<ResMut<MostRecentTick>>,
) {
    if let Ok((mut transform, transform_from_server, mut player_input_queue)) =
        transform_query.get_single_mut()
    {
        let mut pop_min = u32::MAX;
        let mut pop_max = u32::MIN;
        while let Some(input) = player_input_queue.queue.front() {
            let do_pop = input.serial <= player_input_queue.last_server_serial;
            if do_pop {
                if do_pop {
                    pop_min = pop_min.min(input.serial);
                    pop_max = pop_max.max(input.serial);
                }
                player_input_queue.queue.pop_front();
            } else {
                break;
            }
        }
        if pop_min != u32::MAX {
            debug!("pop {}-{}", pop_min, pop_max);
        }
        *transform = transform_from_server.0;

        let mut apply_min = u32::MAX;
        let mut apply_max = u32::MIN;
        for input in &player_input_queue.queue {
            let x = (input.right as i8 - input.left as i8) as f32;
            let y = (input.down as i8 - input.up as i8) as f32;
            let direction = Vec2::new(x, y).normalize_or_zero();

            let offs = direction * PLAYER_MOVE_SPEED * (1.0 / 60.0);
            transform.translation.x += offs.x;
            transform.translation.z += offs.y;
            apply_min = apply_min.min(input.serial);
            apply_max = apply_max.max(input.serial);
        }
        debug!(
            "apply {}-{}: {:?}",
            apply_min, apply_max, transform.translation
        );
    } else {
        warn!("no controlled player");
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
/// update camera tracking
fn update_target_system(
    windows: Res<Windows>,
    mut target_query: Query<&mut Transform, With<Target>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
) {
    let (camera, camera_transform) = camera_query.single();
    let mut target_transform = target_query.single_mut();
    if let Some(ray) = Ray3d::from_screenspace(&windows, camera, camera_transform) {
        if let Some(pos) = ray.intersect_y_plane(1.0) {
            target_transform.translation = pos;
        }
    }
}

fn setup_camera(mut commands: Commands) {
    commands
        .spawn_bundle(LookTransformBundle {
            transform: LookTransform {
                eye: Vec3::new(0.0, 8., 2.5),
                target: Vec3::new(0.0, 0.5, 0.0),
            },
            smoother: Smoother::new(0.9),
        })
        .insert_bundle(Camera3dBundle {
            transform: Transform::from_xyz(0., 8.0, 2.5)
                .looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
            ..default()
        });
}

fn setup_target(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands
        .spawn_bundle(PbrBundle {
            mesh: meshes.add(Mesh::from(shape::Icosphere {
                radius: 0.1,
                subdivisions: 5,
            })),
            material: materials.add(Color::rgb(1.0, 0.0, 0.0).into()),
            transform: Transform::from_xyz(0.0, 0., 0.0),
            ..Default::default()
        })
        .insert(Target);
}

fn camera_follow(
    mut camera_query: Query<&mut LookTransform, (With<Camera>, Without<ControlledPlayer>)>,
    player_query: Query<&Transform, With<ControlledPlayer>>,
) {
    let mut cam_transform = camera_query.single_mut();
    if let Ok(player_transform) = player_query.get_single() {
        cam_transform.eye.x = player_transform.translation.x;
        cam_transform.eye.z = player_transform.translation.z + 2.5;
        cam_transform.target = player_transform.translation;
    }
}
