use bevy::prelude::*;
use smooth_bevy_cameras::{LookTransform, LookTransformBundle, Smoother};

use crate::{ControlledPlayer, Ray3d, WorldSpacePointer};

/// update camera tracking
pub fn update_target_system(
    windows: Res<Windows>,
    mut target_query: Query<&mut Transform, With<WorldSpacePointer>>,
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

pub fn setup_camera(mut commands: Commands) {
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

pub fn setup_target(
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
        .insert(WorldSpacePointer);
}

pub fn camera_follow(
    mut camera_query: Query<&mut LookTransform, (With<Camera>, Without<ControlledPlayer>)>,
    player_query: Query<&Transform, With<ControlledPlayer>>,
) {
    let mut cam_transform = camera_query.single_mut();
    if let Ok(player_transform) = player_query.get_single() {
        cam_transform.eye.x = player_transform.translation.x;
        cam_transform.eye.z = player_transform.translation.z + 8.5;
        cam_transform.target = player_transform.translation;
    }
}
