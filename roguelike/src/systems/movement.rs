use bevy::prelude::*;

use crate::components::{BlocksMovement, Health, PlayerControlled, Position, Viewshed};
use crate::events::MoveIntent;
use crate::grid_vec::GridVec;
use crate::resources::{
    BloodMap, CombatLog, CursorPosition, GameMapResource, GameState, SpatialIndex, TurnCounter,
};
/// Health threshold below which entities leave blood trails when moving.
const BLOOD_DRIP_THRESHOLD: i32 = 40;

/// Processes `MoveIntent` events: checks the target tile on the `GameMap` for
/// walkability *and* the `SpatialIndex` for entities that block movement.
///
/// When the target tile is blocked by another entity, movement is simply
/// skipped (no bump-to-attack). Combat is now initiated explicitly via
/// the punch command (G key).
///
/// **Spatial index atomicity**: after each successful move, the spatial index
/// is updated inline (entity removed from old tile, added to new tile). This
/// ensures that when processing multiple `MoveIntent`s in a single frame
/// (e.g., from AI during WorldTurn), subsequent intents see accurate
/// occupancy data. Without this, two entities could move to the same tile
/// simultaneously because the index would still show their original positions.
///
/// Also marks the entity's `Viewshed` as dirty so FOV is recalculated.
pub fn movement_system(
    mut intents: MessageReader<MoveIntent>,
    game_map: Res<GameMapResource>,
    mut spatial: ResMut<SpatialIndex>,
    mut cursor: ResMut<CursorPosition>,
    mut blood_map: ResMut<BloodMap>,
    turn_counter: Res<TurnCounter>,
    blockers: Query<(), With<BlocksMovement>>,
    players: Query<(), With<PlayerControlled>>,
    healths: Query<&Health>,
    mut movers: Query<(&mut Position, Option<&mut Viewshed>)>,
) {
    for intent in intents.read() {
        let Ok((mut pos, viewshed)) = movers.get_mut(intent.entity) else {
            continue;
        };

        let target = pos.as_grid_vec() + GridVec::new(intent.dx, intent.dy);

        // 1. Check map tile walkability (no blocking props).
        let tile_passable = game_map.0.is_passable(&target);

        // 1b. Water blocks NPC/player movement (but not projectiles).
        let water_blocked = game_map.0.is_water(&target);

        // 2. Check spatial index for blocking entities at the target.
        let entity_blocked = spatial
            .entities_at(&target)
            .iter()
            .any(|&e| e != intent.entity && blockers.contains(e));

        let is_player = players.contains(intent.entity);

        if tile_passable && !water_blocked && !entity_blocked {
            let old_pos = pos.as_grid_vec();

            // ── Blood trail: wounded entities leave blood below 40 HP ─
            if let Ok(hp) = healths.get(intent.entity)
                && hp.current < BLOOD_DRIP_THRESHOLD
            {
                blood_map.stains.insert(old_pos, turn_counter.0);
            }

            let delta = GridVec::new(intent.dx, intent.dy);
            pos.x = target.x;
            pos.y = target.y;

            // ── Maintain spatial index invariant ─────────────────
            // Atomically move the entity in the index so subsequent
            // intents in this frame see accurate occupancy.
            spatial.move_entity(&old_pos, target, intent.entity);

            // ── Cursor follows player movement ──────────────────
            // When the player moves, the cursor moves by the same delta
            // so the player keeps looking in the same relative direction.
            if is_player {
                cursor.pos += delta;
            }

            // Mark viewshed dirty so visibility is recalculated.
            if let Some(mut vs) = viewshed {
                vs.dirty = true;
            }
        }
    }

    // Periodically prune old blood stains to prevent unbounded growth.
    blood_map.prune(turn_counter.0);
}

/// Checks if the player has reached any edge of the map.
/// Transitions to Victory state when the player escapes the town.
///
/// Uses `Single` (see `examples/ecs/fallible_params.rs`): the system is
/// automatically skipped when the player entity doesn't exist.
pub fn victory_check_system(
    player_pos: Single<&Position, With<PlayerControlled>>,
    game_map: Res<GameMapResource>,
    mut next_state: ResMut<NextState<GameState>>,
    mut combat_log: ResMut<CombatLog>,
    state: Res<State<GameState>>,
) {
    if *state.get() != GameState::Playing {
        return;
    }
    let gv = player_pos.as_grid_vec();
    // Win by reaching any edge of the map (escape the town)
    if gv.x <= 1 || gv.y <= 1 || gv.x >= game_map.0.width - 2 || gv.y >= game_map.0.height - 2 {
        combat_log.push("You escaped Yerba Buena! YOU WIN!".into());
        next_state.set(GameState::Victory);
    }
}
