use bevy::prelude::*;

use crate::components::{
    Faction, Health, Name, PlayerControlled, Position, Projectile, ProjectileVisual, Renderable,
    ThrownItemProjectile, Viewshed, display_name,
};
use crate::events::DamageEvent;
use crate::grid_vec::GridVec;
use crate::noise::value_noise;
use crate::resources::{CombatLog, GameMapResource, SoundEvents};
use crate::systems::ai::factions_are_hostile;
use crate::typedefs::RatColor;

/// Bullet travel speed in tiles per game turn.
/// Bullets cross 12 tiles per tick; intra-tick animation renders each tile
/// individually with a short delay so the player can watch the bullet travel.
pub const BULLET_TILES_PER_TICK: usize = 12;

/// Shrapnel travel speed in tiles per tick.
pub const SHRAPNEL_TILES_PER_TICK: usize = 2;

/// Knife/Tomahawk travel speed in tiles per tick.
pub const THROWN_TILES_PER_TICK: usize = 2;

/// Arrow travel speed in tiles per tick (same as bullets for bows).
pub const ARROW_TILES_PER_TICK: usize = 3;

/// Maximum range for thrown knives and tomahawks (in tiles).
pub const THROWN_RANGE: i32 = 12;

/// Delay in seconds between each tile step for projectile advancement.
/// 10ms gives a fast but perceptible travel speed.
pub const TILE_STEP_DELAY: f32 = 0.01;

/// Shrapnel self-damage multiplier (fraction of original damage dealt to the caster).
/// Shrapnel that hits the player who threw the grenade deals reduced damage.
const SELF_DAMAGE_DIVISOR: i32 = 2;

#[inline]
fn player_can_see_event(
    pos: GridVec,
    player_visibility: Option<(GridVec, Option<&Viewshed>)>,
) -> bool {
    let Some((player_pos, viewshed)) = player_visibility else {
        return false;
    };
    pos == player_pos || viewshed.is_some_and(|vs| vs.visible_tiles.contains(&pos))
}

/// Result of resolving a bullet hit-chance roll.
enum BulletHitResult {
    Miss,
    Headshot { damage: i32 },
    Hit { damage: i32 },
}

/// Resolves bullet hit-chance, headshot, and miss rolls for a single target.
/// Returns the outcome so callers can handle damage events and logging.
fn resolve_bullet_hit(
    tile: GridVec,
    _aim_point: GridVec,
    path_index: usize,
    target_max_hp: i32,
    penetration: i32,
) -> BulletHitResult {
    // Accuracy degrades with distance traveled from the shooter.
    let distance = path_index as f64;
    let hit_chance = (0.98 - distance * 0.02).clamp(0.35, 0.98);
    let headshot_chance = 0.02 + if distance < 1.0 { 0.08 } else { 0.0 };

    let roll_seed = 7919_u64.wrapping_add(path_index as u64);
    let roll = value_noise(tile.x, tile.y + path_index as i32, roll_seed);

    if roll > hit_chance {
        return BulletHitResult::Miss;
    }

    let hs_roll = value_noise(
        tile.x,
        tile.y + path_index as i32,
        roll_seed.wrapping_add(13),
    );
    if hs_roll < headshot_chance {
        return BulletHitResult::Headshot {
            damage: target_max_hp,
        };
    }

    BulletHitResult::Hit {
        damage: penetration,
    }
}

/// Spawns a bullet projectile entity along a Bresenham line from origin to endpoint.
pub fn spawn_bullet(
    commands: &mut Commands,
    origin: GridVec,
    endpoint: GridVec,
    damage: i32,
    source: Entity,
) {
    let path = origin.bresenham_line(endpoint);
    if path.len() <= 1 {
        return;
    }
    let start_pos = path.get(1).copied().unwrap_or(origin);
    commands.spawn((
        Position {
            x: start_pos.x,
            y: start_pos.y,
        },
        Renderable {
            symbol: "◦".into(),
            fg: RatColor::Rgb(255, 200, 80),
            bg: RatColor::Black,
        },
        Projectile {
            path,
            path_index: 1, // skip origin (index 0)
            tiles_per_tick: BULLET_TILES_PER_TICK,
            damage,
            penetration: damage,
            source,
            tail_pos: None,
            visual: ProjectileVisual::BulletTrail,
            is_bullet: true,
            tile_timer: 0.0,
        },
    ));
}

/// Spawns an arrow projectile entity along a Bresenham line from origin to endpoint.
/// Arrows travel slower than bullets and are rendered differently.
pub fn spawn_arrow(
    commands: &mut Commands,
    origin: GridVec,
    endpoint: GridVec,
    damage: i32,
    source: Entity,
) {
    let path = origin.bresenham_line(endpoint);
    if path.len() <= 1 {
        return;
    }
    let start_pos = path.get(1).copied().unwrap_or(origin);
    commands.spawn((
        Position {
            x: start_pos.x,
            y: start_pos.y,
        },
        Renderable {
            symbol: "◦".into(),
            fg: RatColor::Rgb(139, 90, 43),
            bg: RatColor::Black,
        },
        Projectile {
            path,
            path_index: 1,
            tiles_per_tick: ARROW_TILES_PER_TICK,
            damage,
            penetration: damage,
            source,
            tail_pos: None,
            visual: ProjectileVisual::BulletTrail,
            is_bullet: false,
            tile_timer: 0.0,
        },
    ));
}

/// Fixed shrapnel damage per fragment.
pub const SHRAPNEL_DAMAGE: i32 = 20;

/// Spawns shrapnel projectile entities for a grenade blast.
/// One projectile per radial direction within the blast radius.
pub fn spawn_shrapnel(
    commands: &mut Commands,
    origin: GridVec,
    radius: i32,
    _damage: i32,
    source: Entity,
) {
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            let dist = dx.abs().max(dy.abs());
            if dist == 0 || dist > radius {
                continue;
            }
            let endpoint = origin + GridVec::new(dx, dy);
            let path = origin.bresenham_line(endpoint);
            if path.len() <= 1 {
                continue;
            }
            let start_pos = path.get(1).copied().unwrap_or(origin);
            commands.spawn((
                Position {
                    x: start_pos.x,
                    y: start_pos.y,
                },
                Renderable {
                    symbol: "·".into(),
                    fg: RatColor::Rgb(255, 165, 0),
                    bg: RatColor::Black,
                },
                Projectile {
                    path,
                    path_index: 1,
                    tiles_per_tick: SHRAPNEL_TILES_PER_TICK,
                    damage: SHRAPNEL_DAMAGE,
                    penetration: SHRAPNEL_DAMAGE,
                    source,
                    tail_pos: None,
                    visual: ProjectileVisual::ShrapnelTrail,
                    is_bullet: false,
                    tile_timer: 0.0,
                },
            ));
        }
    }
}

/// Spawns a thrown-item projectile (knife or tomahawk) along a Bresenham line.
/// The projectile travels at `THROWN_TILES_PER_TICK` and uses the spinning-blade
/// visual. When it reaches a hostile or the end of its path, the item entity is
/// placed at the landing position with a `Thrown` marker for recovery.
pub fn spawn_thrown_projectile(
    commands: &mut Commands,
    origin: GridVec,
    endpoint: GridVec,
    damage: i32,
    source: Entity,
    item_entity: Entity,
) {
    let path = origin.bresenham_line(endpoint);
    if path.len() <= 1 {
        return;
    }
    let start_pos = path.get(1).copied().unwrap_or(origin);
    commands.spawn((
        Position {
            x: start_pos.x,
            y: start_pos.y,
        },
        Renderable {
            symbol: "/".into(),
            fg: RatColor::Rgb(200, 200, 200),
            bg: RatColor::Black,
        },
        Projectile {
            path,
            path_index: 1,
            tiles_per_tick: THROWN_TILES_PER_TICK,
            damage,
            penetration: damage,
            source,
            tail_pos: None,
            visual: ProjectileVisual::SpinningBlade,
            is_bullet: false,
            tile_timer: 0.0,
        },
        ThrownItemProjectile { item_entity },
    ));
}

/// Advances all projectile entities along their paths using a per-tile timer.
/// Each tile step takes `TILE_STEP_DELAY` seconds of accumulated real time.
/// When a projectile reaches a tile with a hostile entity, it applies damage.
/// Visual position, renderable updates, and despawn all happen in this system.
/// When `Time` is not available (e.g. in tests), falls back to advancing
/// `tiles_per_tick` tiles per frame for backwards compatibility.
pub fn projectile_system(
    mut commands: Commands,
    mut projectiles: Query<
        (
            Entity,
            &mut Position,
            &mut Projectile,
            &mut Renderable,
            Option<&ThrownItemProjectile>,
        ),
        Without<crate::components::ThrownExplosive>,
    >,
    mut damage_events: MessageWriter<DamageEvent>,
    targets: Query<
        (Entity, &Position, &Health, Option<&Name>, Option<&Faction>),
        (Without<PlayerControlled>, Without<Projectile>),
    >,
    player_query: Query<
        (Entity, &Position, &Health, Option<&Name>, Option<&Viewshed>),
        (With<PlayerControlled>, Without<Projectile>),
    >,
    source_factions: Query<Option<&Faction>>,
    source_names: Query<Option<&Name>>,
    game_map: Res<GameMapResource>,
    mut combat_log: ResMut<CombatLog>,
    mut sound_events: ResMut<SoundEvents>,
    time: Option<Res<Time>>,
) {
    // Build a lookup of damageable entities by position for O(1) hit detection.
    // Hostility is purely faction-based: entities of different factions are hostile.
    let mut target_by_pos: std::collections::HashMap<
        GridVec,
        Vec<(Entity, String, i32, Option<Faction>)>,
    > = std::collections::HashMap::new();
    for (target_entity, target_pos, target_health, target_name, target_faction) in &targets {
        let t_name = display_name(target_name).to_string();
        target_by_pos
            .entry(target_pos.as_grid_vec())
            .or_default()
            .push((
                target_entity,
                t_name,
                target_health.max,
                target_faction.copied(),
            ));
    }

    // PlayerControlled position for NPC bullet hits and shrapnel self-damage.
    let player_info = player_query.single().ok();
    let player_visibility = player_info.map(|(_, pos, _, _, viewshed)| (pos.as_grid_vec(), viewshed));

    for (proj_entity, mut proj_pos, mut proj, mut renderable, thrown_item) in &mut projectiles {
        // Determine how many tiles to advance this frame.
        // With Time: accumulate delta and advance one tile per TILE_STEP_DELAY.
        // Without Time (tests): advance tiles_per_tick tiles instantly.
        let steps = if let Some(ref time) = time {
            let dt = time.delta_secs();
            proj.tile_timer += dt;
            let tile_steps = (proj.tile_timer / TILE_STEP_DELAY).floor() as usize;
            proj.tile_timer -= tile_steps as f32 * TILE_STEP_DELAY;
            tile_steps.min(proj.tiles_per_tick)
        } else {
            proj.tiles_per_tick
        };

        let mut despawn = false;

        // Look up the name and faction of the entity that fired this projectile.
        let source_name = display_name(source_names.get(proj.source).ok().flatten());
        let source_faction = source_factions.get(proj.source).ok().flatten().copied();

        // Label for combat log messages based on projectile type.
        let proj_label = match proj.visual {
            ProjectileVisual::SpinningBlade => "thrown weapon",
            ProjectileVisual::BulletTrail if proj.is_bullet => "bullet",
            ProjectileVisual::BulletTrail => "projectile",
            ProjectileVisual::ShrapnelTrail | ProjectileVisual::Asterisk => "shrapnel",
        };

        for _ in 0..steps {
            // Check current tile for damage before advancing.
            let tile = proj.path[proj.path_index];

            // Stop if hitting an impassable wall (windows let projectiles through).
            if !game_map.0.is_passable_for_projectiles(&tile) {
                sound_events.add(tile);
                despawn = true;
                break;
            }

            // Check for entities at this tile.
            // Hostility is faction-based: only hit entities of different factions.
            // Penetration model: the first hit deals full penetration damage.
            if let Some(entities_here) = target_by_pos.get(&tile) {
                for (target_entity, t_name, t_max_hp, target_faction) in entities_here {
                    if proj.penetration <= 0 {
                        break;
                    }
                    // Skip same-faction targets (friendly fire protection).
                    if let (Some(sf), Some(tf)) = (source_faction, target_faction) {
                        if !factions_are_hostile(sf, *tf) {
                            continue;
                        }
                    }

                    // Chance-to-hit for bullets (shrapnel/thrown always hits).
                    let is_bullet = proj.is_bullet;
                    if is_bullet {
                        let aim_point = proj.path.last().copied().unwrap_or(tile);
                        match resolve_bullet_hit(
                            tile,
                            aim_point,
                            proj.path_index,
                            *t_max_hp,
                            proj.penetration,
                        ) {
                            BulletHitResult::Miss => {
                                if player_can_see_event(tile, player_visibility) {
                                    combat_log.push_at(
                                        format!("{source_name}'s bullet barely misses {t_name}!"),
                                        tile,
                                    );
                                }
                                continue;
                            }
                            BulletHitResult::Headshot { damage: hs_damage } => {
                                damage_events.write(DamageEvent {
                                    target: *target_entity,
                                    amount: hs_damage,
                                    source: Some(proj.source),
                                });
                                if player_can_see_event(tile, player_visibility) {
                                    combat_log
                                        .push_at(format!("{source_name} headshots {t_name}!"), tile);
                                }
                                sound_events.add(tile);
                                continue;
                            }
                            BulletHitResult::Hit { .. } => {
                                // Fall through to normal hit handling below,
                                // which uses proj.penetration (same value).
                            }
                        }
                    }

                    let hit_damage = proj.penetration;
                    damage_events.write(DamageEvent {
                        target: *target_entity,
                        amount: hit_damage,
                        source: Some(proj.source),
                    });
                    if player_can_see_event(tile, player_visibility) {
                        combat_log.push_at(
                            format!(
                                "{source_name}'s {proj_label} hits {t_name} for {hit_damage} damage!"
                            ),
                            tile,
                        );
                    }
                    sound_events.add(tile);
                    // Thrown weapons (knives/tomahawks) stop on first hit.
                    if proj.visual == ProjectileVisual::SpinningBlade {
                        proj.penetration = 0;
                        break;
                    }
                }
                if proj.penetration <= 0 {
                    despawn = true;
                    break;
                }
            }

            // NPC bullet hitting the player: check if the projectile source
            // is NOT the player and it landed on the player's tile.
            // Always stop the bullet after hitting the player to prevent
            // any possibility of double damage.
            if let Some((player_entity, player_pos, player_health, player_name, _)) = &player_info
                && proj.source != *player_entity
                && tile == player_pos.as_grid_vec()
                && proj.penetration > 0
            {
                // Chance-to-hit for bullets (shrapnel/thrown always hits).
                let is_bullet = proj.is_bullet;
                if is_bullet {
                    let aim_point = proj.path.last().copied().unwrap_or(tile);
                    let p_name = display_name(*player_name);
                    match resolve_bullet_hit(
                        tile,
                        aim_point,
                        proj.path_index,
                        player_health.max,
                        proj.penetration,
                    ) {
                        BulletHitResult::Miss => {
                            if player_can_see_event(tile, player_visibility) {
                                combat_log.push_at(
                                    format!("{source_name}'s bullet barely misses {p_name}!"),
                                    tile,
                                );
                            }
                            // Bullet continues through on miss — don't despawn.
                        }
                        BulletHitResult::Headshot { damage: hs_damage } => {
                            damage_events.write(DamageEvent {
                                target: *player_entity,
                                amount: hs_damage,
                                source: Some(proj.source),
                            });
                            if player_can_see_event(tile, player_visibility) {
                                combat_log.push_at(format!("{source_name} headshots {p_name}!"), tile);
                            }
                            sound_events.add(tile);
                            despawn = true;
                            break;
                        }
                        BulletHitResult::Hit { damage: hit_damage } => {
                            damage_events.write(DamageEvent {
                                target: *player_entity,
                                amount: hit_damage,
                                source: Some(proj.source),
                            });
                            if player_can_see_event(tile, player_visibility) {
                                combat_log.push_at(
                                    format!(
                                        "{source_name}'s bullet hits {p_name} for {hit_damage} damage!"
                                    ),
                                    tile,
                                );
                            }
                            sound_events.add(tile);
                            despawn = true;
                            break;
                        }
                    }
                } else {
                    // Non-bullet projectile always hits.
                    let hit_damage = proj.penetration;
                    damage_events.write(DamageEvent {
                        target: *player_entity,
                        amount: hit_damage,
                        source: Some(proj.source),
                    });
                    let p_name = display_name(*player_name);
                    if player_can_see_event(tile, player_visibility) {
                        combat_log.push_at(
                            format!(
                                "{source_name}'s {proj_label} hits {p_name} for {hit_damage} damage!"
                            ),
                            tile,
                        );
                    }
                    sound_events.add(tile);
                    despawn = true;
                    break;
                }
            }

            // Shrapnel self-damage: if the projectile's source is the player
            // and the projectile lands on the player's tile.
            if let Some((player_entity, player_pos, _, _, _)) = &player_info
                && proj.source == *player_entity
                && tile == player_pos.as_grid_vec()
            {
                let self_damage = (proj.damage / SELF_DAMAGE_DIVISOR).max(1);
                damage_events.write(DamageEvent {
                    target: *player_entity,
                    amount: self_damage,
                    source: Some(proj.source),
                });
                if player_can_see_event(tile, player_visibility) {
                    combat_log.push_at(format!("Shrapnel hits you for {self_damage} damage!"), tile);
                }
                despawn = true;
                break;
            }

            // Record tail position before advancing.
            proj.tail_pos = Some(tile);

            // Advance to next tile.
            proj.path_index += 1;
            if proj.path_index >= proj.path.len() {
                despawn = true;
                break;
            }
        }

        // Update visual position to current path_index.
        let di = proj.path_index.min(proj.path.len() - 1);
        let display_tile = proj.path[di];
        proj_pos.x = display_tile.x;
        proj_pos.y = display_tile.y;

        // Visual updates based on projectile type.
        match proj.visual {
            ProjectileVisual::BulletTrail => {
                let remaining = proj.path.len().saturating_sub(di);
                if remaining <= 2 {
                    renderable.symbol = "·".into();
                    renderable.fg = RatColor::Rgb(180, 120, 0);
                } else if remaining <= 4 {
                    renderable.symbol = "·".into();
                    renderable.fg = RatColor::Rgb(255, 180, 40);
                }
            }
            ProjectileVisual::ShrapnelTrail => {
                let remaining = proj.path.len().saturating_sub(di);
                renderable.symbol = if remaining <= 2 { "x".into() } else { "•".into() };
                renderable.fg = if remaining <= 2 {
                    RatColor::Rgb(255, 120, 40)
                } else {
                    RatColor::Rgb(255, 190, 80)
                };
            }
            ProjectileVisual::SpinningBlade => {
                const SPIN_FRAMES: [&str; 4] = ["/", "—", "\\", "|"];
                renderable.symbol = SPIN_FRAMES[di % SPIN_FRAMES.len()].into();
            }
            ProjectileVisual::Asterisk => {}
        }

        if despawn {
            // If this projectile carries a thrown item, place it at the
            // landing position so the player can recover it.
            if let Some(thrown) = thrown_item {
                let tile = proj.path[proj.path_index.min(proj.path.len() - 1)];
                commands.entity(thrown.item_entity).insert(Position {
                    x: tile.x,
                    y: tile.y,
                });
            }
            commands.entity(proj_entity).despawn();
        }
    }
}
