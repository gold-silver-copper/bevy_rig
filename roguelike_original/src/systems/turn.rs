use bevy::prelude::*;

use crate::components::{Dead, Health, PlayerControlled, Position, Stamina, Viewshed};
use crate::grid_vec::GridVec;
use crate::resources::{
    CombatLog, DynamicRng, ExtraWorldTicks, GameMapResource, MapSeed, SoundEvents,
    SpectatingAfterDeath, TurnCounter, TurnState,
};
use crate::typeenums::Floor;

/// Stamina regenerated per world turn.
const STAMINA_REGEN_PER_TURN: i32 = 1;

/// Fire spreads every N world turns.
const FIRE_SPREAD_INTERVAL: u32 = 8;

/// Damage dealt to entities standing on fire per turn.
const FIRE_DAMAGE: i32 = 2;

/// Maximum number of world turns a fire tile persists before burning out.
const FIRE_BURNOUT_TURNS: u32 = 20;

/// Number of world turns before a sand cloud tile dissipates.
const SAND_CLOUD_LIFETIME: u32 = 8;

/// Advances the turn state from `PlayerTurn` → `WorldTurn`.
/// Runs only during `TurnState::PlayerTurn` after all player-phase systems.
pub fn end_player_turn_system(mut next_state: ResMut<NextState<TurnState>>) {
    next_state.set(TurnState::WorldTurn);
}

/// Advances the turn state from `WorldTurn` → `AwaitingInput`, or stays in
/// `WorldTurn` if `ExtraWorldTicks` has remaining ticks (physical movement
/// costs 2 ticks). Increments the turn counter and regenerates player stats.
pub fn end_world_turn_system(
    mut next_state: ResMut<NextState<TurnState>>,
    mut turn_counter: ResMut<TurnCounter>,
    mut player_query: Query<(&mut Stamina, &mut Health), (With<PlayerControlled>, Without<Dead>)>,
    mut extra_ticks: ResMut<ExtraWorldTicks>,
    mut sound_events: ResMut<SoundEvents>,
    spectating: Res<SpectatingAfterDeath>,
    mut dynamic_rng: ResMut<DynamicRng>,
) {
    turn_counter.0 += 1;
    dynamic_rng.advance();
    sound_events.tick();

    if let Ok((mut stamina, _health)) = player_query.single_mut() {
        // Regenerate player stamina using the pool's recover method.
        stamina.recover(STAMINA_REGEN_PER_TURN);
    }

    // If spectating after death, stay in Playing and go to AwaitingInput
    // so the input system can auto-advance the next turn.
    if spectating.0 {
        extra_ticks.0 = 0;
        next_state.set(TurnState::AwaitingInput);
        return;
    }

    if extra_ticks.0 > 0 {
        extra_ticks.0 -= 1;
        // Stay in WorldTurn for the extra tick — don't transition yet.
    } else {
        next_state.set(TurnState::AwaitingInput);
    }
}

/// Fire system: spreads fire to adjacent flammable tiles and damages entities
/// standing on fire. Fire tiles burn out deterministically after
/// `FIRE_BURNOUT_TURNS` world turns, destroying any props and leaving
/// scorched earth.
///
/// Runs every world turn during `WorldTurn`.
pub fn fire_system(
    mut game_map: ResMut<GameMapResource>,
    turn_counter: Res<TurnCounter>,
    mut health_query: Query<&mut Health>,
    position_query: Query<(Entity, &Position, Option<&crate::components::Name>)>,
    mut combat_log: ResMut<CombatLog>,
    dynamic_rng: Res<DynamicRng>,
    seed: Res<MapSeed>,
    player_visibility_query: Query<(&Position, Option<&Viewshed>), With<PlayerControlled>>,
) {
    let player_visibility = player_visibility_query
        .single()
        .ok()
        .map(|(pos, viewshed)| (pos.as_grid_vec(), viewshed));
    let map_width = game_map.0.width;
    let map_height = game_map.0.height;

    // Damage entities standing on fire tiles.
    for (entity, pos, name) in &position_query {
        let p = pos.as_grid_vec();
        let entity_name = crate::components::display_name(name);
        if let Some(voxel) = game_map.0.get_voxel_at(&p)
            && matches!(voxel.floor, Some(Floor::Fire))
            && let Ok(mut hp) = health_query.get_mut(entity)
        {
            let actual = hp.apply_damage(FIRE_DAMAGE);
            if actual > 0
                && player_visibility.is_some_and(|(player_pos, viewshed)| {
                    p == player_pos || viewshed.is_some_and(|vs| vs.visible_tiles.contains(&p))
                })
            {
                combat_log.push_at(
                    format!("{entity_name} is burned by fire for {actual} damage!"),
                    p,
                );
            }
        }
    }

    // Register any new fire tiles that the tracker doesn't know about yet.
    // Also randomly generate smoke from active fire tiles each tick.
    let mut fire_smoke_tiles: Vec<GridVec> = Vec::new();
    for y in 1..map_height - 1 {
        for x in 1..map_width - 1 {
            let pos = GridVec::new(x, y);
            if let Some(voxel) = game_map.0.get_voxel_at(&pos)
                && matches!(voxel.floor, Some(Floor::Fire))
            {
                game_map.0.fire_turns.entry(pos).or_insert(turn_counter.0);
                // ~30% chance per fire tile per tick to generate smoke.
                let smoke_roll = dynamic_rng.roll(
                    seed.0,
                    (pos.x as u64).wrapping_mul(9973).wrapping_add(pos.y as u64),
                );
                if smoke_roll < 0.3 {
                    fire_smoke_tiles.push(pos);
                }
            }
        }
    }

    // Place smoke clouds adjacent to fire tiles (smoke rises and drifts).
    for fire_pos in fire_smoke_tiles {
        let dirs = [
            GridVec::new(1, 0),
            GridVec::new(-1, 0),
            GridVec::new(0, 1),
            GridVec::new(0, -1),
            GridVec::new(1, 1),
            GridVec::new(-1, 1),
            GridVec::new(1, -1),
            GridVec::new(-1, -1),
        ];
        let dir_hash = dynamic_rng.random_index(
            seed.0,
            (fire_pos.x as u64)
                .wrapping_add(fire_pos.y as u64)
                .wrapping_mul(7),
            dirs.len(),
        );
        let smoke_pos = fire_pos + dirs[dir_hash];
        if let Some(voxel) = game_map.0.get_voxel_at(&smoke_pos)
            && !matches!(voxel.floor, Some(Floor::SandCloud))
            && !matches!(voxel.floor, Some(Floor::Fire))
            && !matches!(
                voxel.props,
                Some(crate::typeenums::Props::Wall) | Some(crate::typeenums::Props::StoneWall)
            )
        {
            let prev_floor = voxel.floor.clone();
            game_map
                .0
                .sand_cloud_previous_floor
                .entry(smoke_pos)
                .or_insert(prev_floor);
            if let Some(v) = game_map.0.get_voxel_at_mut(&smoke_pos) {
                v.floor = Some(Floor::SandCloud);
            }
            game_map
                .0
                .sand_cloud_turns
                .insert(smoke_pos, turn_counter.0);
        }
    }

    // Spread fire and burn out old fire tiles every FIRE_SPREAD_INTERVAL turns.
    if !turn_counter.0.is_multiple_of(FIRE_SPREAD_INTERVAL) {
        return;
    }

    // Collect fire tile positions and tiles to spread fire to.
    let mut new_fire_tiles: Vec<GridVec> = Vec::new();
    let mut burnout_tiles: Vec<GridVec> = Vec::new();

    for y in 1..map_height - 1 {
        for x in 1..map_width - 1 {
            let pos = GridVec::new(x, y);
            if let Some(voxel) = game_map.0.get_voxel_at(&pos) {
                if !matches!(voxel.floor, Some(Floor::Fire)) {
                    continue;
                }

                // Deterministic burnout: fire burns out after FIRE_BURNOUT_TURNS world turns.
                if let Some(&ignited_at) = game_map.0.fire_turns.get(&pos)
                    && turn_counter.0.saturating_sub(ignited_at) >= FIRE_BURNOUT_TURNS
                {
                    burnout_tiles.push(pos);
                }

                // Spread fire to adjacent flammable props and wooden/bridge floors.
                for neighbor in pos.cardinal_neighbors() {
                    if let Some(n_voxel) = game_map.0.get_voxel_at(&neighbor) {
                        // Skip tiles already on fire
                        if matches!(n_voxel.floor, Some(Floor::Fire)) {
                            continue;
                        }
                        let has_flammable_prop =
                            n_voxel.props.as_ref().is_some_and(|f| f.is_flammable());
                        let has_flammable_floor =
                            matches!(n_voxel.floor, Some(Floor::WoodPlanks) | Some(Floor::Bridge));
                        if has_flammable_prop || has_flammable_floor {
                            new_fire_tiles.push(neighbor);
                        }
                    }
                }
            }
        }
    }

    // Apply fire spread.
    for tile in new_fire_tiles {
        if let Some(voxel) = game_map.0.get_voxel_at(&tile) {
            let prev = voxel.floor.clone();
            game_map.0.fire_previous_floor.entry(tile).or_insert(prev);
        }
        if let Some(voxel) = game_map.0.get_voxel_at_mut(&tile) {
            voxel.props = None;
            voxel.floor = Some(Floor::Fire);
        }
    }

    // Apply burnout: destroy any remaining props. Burned-out bridges
    // revert to water (matching adjacent river tiles); everything else
    // becomes scorched earth.
    for tile in &burnout_tiles {
        let was_bridge = game_map
            .0
            .fire_previous_floor
            .get(tile)
            .is_some_and(|f| matches!(f, Some(Floor::Bridge)));
        let replacement = if was_bridge {
            // Determine water depth by inspecting the cardinal neighbors
            // for river tiles and picking the most common type.
            let mut shallow = 0u32;
            let mut deep = 0u32;
            for n in tile.cardinal_neighbors() {
                if let Some(nv) = game_map.0.get_voxel_at(&n) {
                    match nv.floor {
                        Some(Floor::ShallowWater) => shallow += 1,
                        Some(Floor::DeepWater) => deep += 1,
                        _ => {}
                    }
                }
            }
            if deep > shallow {
                Floor::DeepWater
            } else {
                Floor::ShallowWater
            }
        } else {
            Floor::ScorchedEarth
        };
        if let Some(voxel) = game_map.0.get_voxel_at_mut(tile) {
            voxel.props = None;
            voxel.floor = Some(replacement);
        }
        game_map.0.fire_turns.remove(tile);
        game_map.0.fire_previous_floor.remove(tile);
    }

    // ── Sand cloud dissipation ──────────────────────────────────────
    // Remove sand cloud tiles that have exceeded their lifetime.
    let expired_clouds: Vec<GridVec> = game_map
        .0
        .sand_cloud_turns
        .iter()
        .filter(|entry| turn_counter.0.saturating_sub(*entry.1) >= SAND_CLOUD_LIFETIME)
        .map(|entry| *entry.0)
        .collect();

    for tile in &expired_clouds {
        // Retrieve the saved floor before mutating the voxel.
        let previous = game_map.0.sand_cloud_previous_floor.remove(tile);
        if let Some(voxel) = game_map.0.get_voxel_at_mut(tile)
            && matches!(voxel.floor, Some(Floor::SandCloud))
        {
            voxel.floor = previous.unwrap_or(Some(Floor::Sand));
        }
        game_map.0.sand_cloud_turns.remove(tile);
    }

    // ── Sand cloud drift ────────────────────────────────────────────
    // Every 3 turns, some edge cloud tiles shift by 1 tile for a slow
    // drifting effect. Uses a deterministic hash to pick which tiles move.
    if turn_counter.0.is_multiple_of(3) {
        let active_clouds: Vec<(GridVec, u32)> = game_map
            .0
            .sand_cloud_turns
            .iter()
            .map(|(&pos, &turn)| (pos, turn))
            .collect();

        // Collect drift operations: (old_pos, new_pos, placed_turn)
        // Primes 7919 and 6271 provide good hash distribution; modulo 5 = 20% drift probability.
        let dirs = [
            GridVec::new(1, 0),
            GridVec::new(-1, 0),
            GridVec::new(0, 1),
            GridVec::new(0, -1),
        ];
        let mut drift_ops: Vec<(GridVec, GridVec, u32)> = Vec::new();
        for (tile, placed_turn) in &active_clouds {
            let hash = (tile.x.wrapping_mul(7919) ^ tile.y.wrapping_mul(6271))
                .wrapping_add(turn_counter.0 as i32) as u32;
            if !hash.is_multiple_of(5) {
                continue;
            }
            let dir_idx = (hash / 5) as usize % 4;
            let new_pos = *tile + dirs[dir_idx];
            if let Some(new_voxel) = game_map.0.get_voxel_at(&new_pos)
                && !matches!(new_voxel.floor, Some(Floor::SandCloud))
                && !matches!(
                    new_voxel.props,
                    Some(crate::typeenums::Props::Wall) | Some(crate::typeenums::Props::StoneWall)
                )
                && !game_map.0.sand_cloud_turns.contains_key(&new_pos)
            {
                drift_ops.push((*tile, new_pos, *placed_turn));
            }
        }
        // Apply drift operations.
        for (old_pos, new_pos, placed_turn) in drift_ops {
            // Read the new position's current floor before modifying.
            let new_floor = game_map
                .0
                .get_voxel_at(&new_pos)
                .and_then(|v| v.floor.clone());
            // Restore old position.
            let previous = game_map.0.sand_cloud_previous_floor.remove(&old_pos);
            if let Some(voxel) = game_map.0.get_voxel_at_mut(&old_pos)
                && matches!(voxel.floor, Some(Floor::SandCloud))
            {
                voxel.floor = previous.unwrap_or(Some(Floor::Sand));
            }
            game_map.0.sand_cloud_turns.remove(&old_pos);
            // Place cloud at new position.
            game_map
                .0
                .sand_cloud_previous_floor
                .entry(new_pos)
                .or_insert(new_floor);
            if let Some(voxel) = game_map.0.get_voxel_at_mut(&new_pos) {
                voxel.floor = Some(Floor::SandCloud);
            }
            game_map.0.sand_cloud_turns.insert(new_pos, placed_turn);
        }
    }
}
