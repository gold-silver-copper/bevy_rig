use bevy::prelude::*;

use crate::components::{CameraFollow, PlayerControlled, Position};
use crate::resources::{CameraPosition, CursorPosition};

/// Centers the camera between the player and cursor positions.
/// This gives the player a wider view of the area they're aiming at.
///
/// Uses `Single` (a fallible system parameter, see `examples/ecs/fallible_params.rs`):
/// the system is automatically skipped when the player entity doesn't exist
/// (e.g., during restart), without requiring manual `if let Ok(...)` checks.
pub fn camera_follow_system(
    player: Single<&Position, (With<CameraFollow>, With<PlayerControlled>)>,
    cursor: Res<CursorPosition>,
    mut camera: ResMut<CameraPosition>,
) {
    let player_pos = player.as_grid_vec();
    // Center camera between player and cursor
    camera.0 = crate::grid_vec::GridVec::new(
        (player_pos.x + cursor.pos.x) / 2,
        (player_pos.y + cursor.pos.y) / 2,
    );
}
