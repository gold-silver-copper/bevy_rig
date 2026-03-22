use std::collections::HashSet;

use bevy::prelude::*;

use crate::components::{Cursor, Faction, PlayerControlled, Position, Viewshed};
use crate::grid_vec::GridVec;
use crate::resources::{CursorPosition, GameMapResource};
use crate::typedefs::{CoordinateUnit, MyPoint};

/// Minimum FOV radius when cursor is centered on the player (circle in all directions).
pub const FOV_MIN_RADIUS: CoordinateUnit = 80;

/// Maximum FOV range when cursor is far from the player.
pub const FOV_MAX_RANGE: CoordinateUnit = 120;
const NPC_RAY_COUNT: usize = 9;

/// Recomputes the `visible_tiles` set for every entity whose `Viewshed` is
/// dirty (e.g., because the entity moved). Uses recursive symmetric
/// shadowcasting — the mathematically correct O(visible_tiles) algorithm
/// that eliminates the artifacts of Bresenham ray casting.
///
/// FOV model:
/// - When the cursor/look direction is centered on the entity, FOV is a circle
///   with radius `FOV_MIN_RADIUS` in all directions.
/// - As the cursor moves further from the entity, the FOV range increases
///   (up to `FOV_MAX_RANGE`) but the cone narrows proportionally.
/// - This creates a "spotlight" effect: far aiming = long narrow beam,
///   close aiming = short wide circle.
///
/// Reference: Albert Ford, "Symmetric Shadowcasting" (2017).
pub fn visibility_system(
    game_map: Res<GameMapResource>,
    mut query: Query<(
        Entity,
        &Position,
        &mut Viewshed,
        Option<&Cursor>,
        Option<&Faction>,
    )>,
    player_query: Query<Entity, With<PlayerControlled>>,
    cursor: Res<CursorPosition>,
    spell_particles: Res<crate::resources::SpellParticles>,
) {
    let player_entity = player_query.single().ok();

    // Collect sand cloud positions (particles with lifetime > 0 and delay == 0).
    let sand_cloud_tiles: HashSet<MyPoint> = spell_particles
        .particles
        .iter()
        .filter(|(_, life, delay, _, _, _)| *delay == 0 && *life > 0)
        .map(|(pos, _, _, _, _, _)| *pos)
        .collect();

    for (entity, pos, mut viewshed, cursor_component, faction) in &mut query {
        let is_player = player_entity == Some(entity);
        // Always recalculate player FOV every tick so newly placed smoke/sand
        // clouds block vision immediately, not just after the player moves.
        if !viewshed.dirty && !is_player {
            continue;
        }

        viewshed.visible_tiles.clear();
        let origin = pos.as_grid_vec();

        let is_wildlife = faction.is_some_and(|f| matches!(f, Faction::Wildlife));
        let is_npc = !is_player && cursor_component.is_some();

        // Determine the aiming direction.
        let cone_dir = if is_player {
            let d = cursor.pos - origin;
            if d.is_zero() { None } else { Some(d) }
        } else {
            cursor_component.and_then(|cursor| {
                let delta = (cursor.pos - origin).king_step();
                if delta.is_zero() { None } else { Some(delta) }
            })
        };

        // Compute dynamic FOV range and cone width.
        let (effective_range, cos_threshold) = if is_wildlife {
            // Animals: very small FOV — short range, narrow cone.
            let range = viewshed.range.min(8);
            let cos_t = if cone_dir.is_some() { 0.6 } else { -1.0 };
            (range, cos_t)
        } else if is_npc {
            // Human NPCs: moderate sight range with a practical cone.
            if let Some(_dir) = cone_dir {
                let range = 150;
                // Slightly tighter cone than before, but with longer useful range.
                let cos_t = 0.92;
                (range, cos_t)
            } else {
                (150, 0.92)
            }
        } else {
            // PlayerControlled: use the original formula.
            compute_fov_params(cone_dir)
        };

        viewshed.visible_tiles.insert(origin);

        if is_npc {
            if let Some(dir) = cone_dir {
                cast_npc_cone(
                    &game_map,
                    &mut viewshed.visible_tiles,
                    origin,
                    dir,
                    effective_range,
                    cos_threshold,
                    &sand_cloud_tiles,
                );
            }
        } else {
            // Cast shadows in all 8 octants via the cardinal/diagonal transform.
            for octant in 0..8u8 {
                shadowcast_octant(
                    &game_map,
                    &mut viewshed.visible_tiles,
                    origin,
                    effective_range,
                    octant,
                    1,
                    Slope { y: 1, x: 1 }, // start slope = 1/1
                    Slope { y: 0, x: 1 }, // end slope   = 0/1
                    &sand_cloud_tiles,
                );
            }

            // Directional FOV: filter visible tiles to a cone when aiming.
            // When the cursor is off-center, the player can no longer see behind
            // themselves — only tiles within the computed cone are kept.
            if let Some(dir) = cone_dir {
                let (cdx, cdy) = (dir.x as f64, dir.y as f64);
                let cursor_len = (cdx * cdx + cdy * cdy).sqrt();

                viewshed.visible_tiles.retain(|&tile| {
                    let diff = tile - origin;
                    if diff == GridVec::ZERO {
                        return true; // always see own tile
                    }
                    // Keyhole effect: tiles directly adjacent (Chebyshev ≤ 1)
                    // are always visible for the player.
                    if is_player && diff.x.abs() <= 1 && diff.y.abs() <= 1 {
                        return true;
                    }
                    let (dx, dy) = (diff.x as f64, diff.y as f64);
                    let len = (dx * dx + dy * dy).sqrt();
                    let dot = (dx * cdx + dy * cdy) / (len * cursor_len);
                    dot >= cos_threshold
                });
            }

            viewshed.visible_tiles.retain(|&tile| {
                has_clear_los(&game_map, origin, tile, &sand_cloud_tiles)
            });
        }

        // Smoke attenuation: smoke particles halve remaining vision range
        // rather than blocking it entirely. For each visible tile, trace a
        // ray from origin and check for smoke along the path. Each smoke
        // tile halves the remaining travel distance from that point onward.
        // Tiles beyond the attenuated range are removed.
        if is_player && (!sand_cloud_tiles.is_empty() || !game_map.0.sand_cloud_turns.is_empty()) {
            viewshed.visible_tiles.retain(|&tile| {
                let diff = tile - origin;
                if diff == GridVec::ZERO {
                    return true;
                }
                let ray = origin.bresenham_line(tile);
                let mut remaining = effective_range as f64;
                for &ray_tile in ray.iter().skip(1) {
                    remaining -= 1.0;
                    if remaining < 0.0 {
                        return false;
                    }
                    let is_smoke = sand_cloud_tiles.contains(&ray_tile)
                        || game_map.0.get_voxel_at(&ray_tile).is_some_and(|v| {
                            matches!(v.floor, Some(crate::typeenums::Floor::SandCloud))
                        });
                    if is_smoke && ray_tile != tile {
                        remaining /= 2.0;
                    }
                }
                true
            });
        }

        if is_player {
            let newly_visible: Vec<MyPoint> = viewshed.visible_tiles.iter().copied().collect();
            viewshed.revealed_tiles.extend(newly_visible);
        } else {
            viewshed.revealed_tiles.clear();
        }
        viewshed.dirty = false;
    }
}

/// Computes FOV range and cone half-angle cosine based on cursor distance.
///
/// - `cursor_dir`: None means cursor is centered (full circle at min radius).
/// - Returns `(effective_range, cos_threshold)`.
///
/// When cursor is close: range = FOV_MIN_RADIUS (36), cos_threshold ≈ -1 (360°).
/// At cursor distance 6: range ≈ 108, cos_threshold ≈ 0.0 (forward hemisphere).
/// When cursor is very far (~20+ tiles): cos_threshold ≈ 0.985 (cone ~10°).
/// This simulates tunnel vision when aiming far away.
/// The keyhole effect (adjacent tiles always illuminated) is applied separately.
pub fn compute_fov_params(cursor_dir: Option<GridVec>) -> (CoordinateUnit, f64) {
    let Some(dir) = cursor_dir else {
        return (FOV_MIN_RADIUS, -1.0); // full circle
    };

    let dist = ((dir.x as f64).powi(2) + (dir.y as f64).powi(2)).sqrt();
    if dist < 1.0 {
        return (FOV_MIN_RADIUS, -1.0); // full circle
    }

    // Range grows aggressively: +12 tiles per tile of cursor distance.
    let range = (FOV_MIN_RADIUS as f64 + dist * 12.0).min(FOV_MAX_RANGE as f64);

    // Cone narrows significantly with distance.
    // At dist=4: cos ≈ -0.01 (broad cone, nearly hemisphere).
    // At dist=8: cos ≈ 0.49 (moderate cone ~60°).
    // At dist=12: cos ≈ 0.74 (cone ~42°).
    // At dist=20+: cos ≈ 0.985 (cone ~10° — tunnel vision).
    let cone_t = (dist / 20.0).min(1.0);
    let cos_threshold = -1.0 + cone_t * 1.985;

    (range as CoordinateUnit, cos_threshold)
}

// ───────────────────────── Shadowcasting internals ─────────────────────────

/// Rational slope represented as y/x to avoid floating-point.
/// Invariant: `x > 0`.
#[derive(Clone, Copy)]
struct Slope {
    y: CoordinateUnit,
    x: CoordinateUnit,
}

impl Slope {
    /// self >= other
    fn ge(self, other: Self) -> bool {
        self.y * other.x >= other.y * self.x
    }
}

/// Returns `true` if the tile at `point` blocks line-of-sight.
/// Smoke (SandCloud) does NOT block vision — it attenuates range instead
/// (handled in the post-filter step of `visibility_system`).
fn is_opaque(game_map: &GameMapResource, point: MyPoint, _sand_clouds: &HashSet<MyPoint>) -> bool {
    match game_map.0.get_voxel_at(&point) {
        Some(v) => v.props.as_ref().is_some_and(|f| f.blocks_vision()),
        None => true, // off-map ⇒ opaque
    }
}

fn has_clear_los(
    game_map: &GameMapResource,
    origin: MyPoint,
    tile: MyPoint,
    sand_clouds: &HashSet<MyPoint>,
) -> bool {
    if tile == origin {
        return true;
    }

    for ray_tile in origin.bresenham_line(tile).into_iter().skip(1) {
        if ray_tile != tile && is_opaque(game_map, ray_tile, sand_clouds) {
            return false;
        }
    }

    true
}

fn cast_npc_cone(
    game_map: &GameMapResource,
    visible: &mut HashSet<MyPoint>,
    origin: MyPoint,
    dir: GridVec,
    range: CoordinateUnit,
    cos_threshold: f64,
    sand_clouds: &HashSet<MyPoint>,
) {
    let dir_x = dir.x as f64;
    let dir_y = dir.y as f64;
    let dir_len = (dir_x * dir_x + dir_y * dir_y).sqrt();
    if dir_len <= f64::EPSILON {
        return;
    }

    let base_angle = dir_y.atan2(dir_x);
    let half_angle = cos_threshold.clamp(-1.0, 1.0).acos();
    let ray_count = NPC_RAY_COUNT.max(1);

    for ray_idx in 0..ray_count {
        let t = if ray_count == 1 {
            0.5
        } else {
            ray_idx as f64 / (ray_count - 1) as f64
        };
        let angle = base_angle - half_angle + (2.0 * half_angle * t);
        let endpoint = origin
            + GridVec::new(
                (angle.cos() * range as f64).round() as CoordinateUnit,
                (angle.sin() * range as f64).round() as CoordinateUnit,
            );

        for tile in origin.bresenham_line(endpoint).into_iter().skip(1) {
            visible.insert(tile);
            if is_opaque(game_map, tile, sand_clouds) {
                break;
            }
        }
    }
}

/// Transforms octant-local (row, col) into world coordinates.
///
/// The 8 octants are indexed 0..7, each covering a 45° sector.
/// Octant 0 is "north-northeast": row increases upward, col goes right.
fn transform(origin: MyPoint, octant: u8, row: CoordinateUnit, col: CoordinateUnit) -> MyPoint {
    match octant {
        0 => origin + GridVec::new(col, row),
        1 => origin + GridVec::new(row, col),
        2 => origin + GridVec::new(row, -col),
        3 => origin + GridVec::new(col, -row),
        4 => origin + GridVec::new(-col, -row),
        5 => origin + GridVec::new(-row, -col),
        6 => origin + GridVec::new(-row, col),
        7 => origin + GridVec::new(-col, row),
        _ => unreachable!(),
    }
}

/// Recursive symmetric shadowcasting for a single octant.
///
/// `row`        — current distance from origin (increases outward).
/// `start`/`end` — the visible angular window, as rational slopes.
///
/// The key insight: we scan each row from `start` to `end`, tracking
/// transitions between opaque and transparent tiles. Each contiguous
/// opaque run narrows the visible window for the next row; each
/// transparent run spawns a recursive sub-scan with the adjusted window.
fn shadowcast_octant(
    game_map: &GameMapResource,
    visible: &mut HashSet<MyPoint>,
    origin: MyPoint,
    range: CoordinateUnit,
    octant: u8,
    row: CoordinateUnit,
    mut start: Slope,
    end: Slope,
    sand_clouds: &HashSet<MyPoint>,
) {
    if row > range {
        return;
    }
    // Ensure the window is still valid.
    if !start.ge(end) {
        return;
    }

    let range_sq = range * range;
    let mut prev_opaque = false;
    let mut saved_start = start;

    // Column range for this row: from ceil(row * end) to floor(row * start).
    let min_col = round_down(row, end);
    let max_col = round_up(row, start);

    for col in (min_col..=max_col).rev() {
        let dist_sq = row * row + col * col;
        if dist_sq > range_sq {
            continue;
        }

        let world = transform(origin, octant, row, col);
        visible.insert(world);

        let cur_opaque = is_opaque(game_map, world, sand_clouds);
        if cur_opaque {
            if !prev_opaque {
                // Transition transparent → opaque: save start slope for the
                // recursive sub-scan that will use the narrowed window.
                saved_start = start;
            }
            // Shrink the start slope past this opaque tile.
            start = Slope {
                y: 2 * col - 1,
                x: 2 * row,
            };
        } else if prev_opaque {
            // Transition opaque → transparent: recurse with the saved window
            // narrowed by the opaque run we just passed.
            let next_end = Slope {
                y: 2 * col + 1,
                x: 2 * row,
            };
            shadowcast_octant(
                game_map,
                visible,
                origin,
                range,
                octant,
                row + 1,
                saved_start,
                next_end,
                sand_clouds,
            );
        }
        prev_opaque = cur_opaque;
    }

    // If the last tile in the row was transparent, continue scanning.
    if !prev_opaque {
        shadowcast_octant(
            game_map,
            visible,
            origin,
            range,
            octant,
            row + 1,
            start,
            end,
            sand_clouds,
        );
    }
}

/// Computes the maximum column index for this row: ⌈row × slope⌉.
/// Uses ceiling division to ensure we don't miss the first column in the scan.
fn round_up(row: CoordinateUnit, slope: Slope) -> CoordinateUnit {
    (row * slope.y + slope.x - 1).div_euclid(slope.x)
}

/// Computes the minimum column index for this row using the half-tile
/// centre convention of the symmetric shadowcasting algorithm.
/// Returns ⌊(2 × row × slope + 1) / (2 × slope.x)⌋, clamped to ≥ 0.
fn round_down(row: CoordinateUnit, slope: Slope) -> CoordinateUnit {
    ((row * 2 * slope.y + slope.x) / (2 * slope.x)).max(0)
}
