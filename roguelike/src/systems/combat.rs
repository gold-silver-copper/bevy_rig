use bevy::prelude::*;

use crate::components::{
    AiMemory, AiPursuitBoost, AiState, AiTarget, Caliber, CollectibleKind, CombatStats, Dead,
    Faction, Health, Inventory, Item, ItemKind, LastDamageSource, LootTable, Name,
    PlayerControlled, Position, Renderable, Stamina, Viewshed, display_name,
};
use crate::events::{
    AiRangedAttackIntent, AttackIntent, DamageEvent, MeleeWideIntent, RangedAttackIntent,
};
use crate::grid_vec::GridVec;
use crate::noise::value_noise;
use crate::resources::{
    Collectibles, CombatLog, DynamicRng, GameMapResource, GameState, KillCount, MapSeed,
    SoundEvents, TurnCounter,
};
use crate::typedefs::{CoordinateUnit, RatColor};
use crate::typeenums::Props;

/// Computes the bullet endpoint by scaling a direction vector so the
/// Bresenham line extends approximately `range` tiles along the
/// dominant axis, preserving the exact trajectory angle.
#[inline]
fn bullet_endpoint(
    origin: GridVec,
    dx: CoordinateUnit,
    dy: CoordinateUnit,
    range: CoordinateUnit,
) -> GridVec {
    let max_comp = dx.abs().max(dy.abs());
    let scale = range.div_euclid(max_comp).max(1);
    origin + GridVec::new(dx * scale, dy * scale)
}

/// Gun smoke scan radius (Chebyshev distance from the firing position).
const SMOKE_SCAN_RADIUS: i32 = 2;
/// Base effective radius for gun smoke (before directional bias).
const SMOKE_BASE_RADIUS: f64 = 0.8;
/// Additional radius in the firing direction (directional plume stretch).
const SMOKE_DIRECTIONAL_SCALE: f64 = 2.0;

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

/// Maximum spread angle in radians (~1.5°). Very small — just enough to
/// occasionally shift a bullet by one tile at long range.
const GUN_SPREAD_ANGLE: f64 = 0.026;

/// Applies a small angular spread to a direction vector.
/// `roll` is a `[0, 1)` noise value that drives the random offset.
/// Returns the adjusted `(dx, dy)` direction, guaranteed non-zero when
/// the input was non-zero.
fn apply_gun_spread(
    dx: CoordinateUnit,
    dy: CoordinateUnit,
    roll: f64,
) -> (CoordinateUnit, CoordinateUnit) {
    let fdx = dx as f64;
    let fdy = dy as f64;
    let len = (fdx * fdx + fdy * fdy).sqrt();
    if len < 0.01 {
        return (dx, dy);
    }
    // Map roll [0,1) to angle offset [-SPREAD, +SPREAD].
    let angle_offset = (roll * 2.0 - 1.0) * GUN_SPREAD_ANGLE;
    let cos_a = angle_offset.cos();
    let sin_a = angle_offset.sin();
    let new_dx = fdx * cos_a - fdy * sin_a;
    let new_dy = fdx * sin_a + fdy * cos_a;
    // Scale back to integer, preserving magnitude so Bresenham line length stays correct.
    let new_len = (new_dx * new_dx + new_dy * new_dy).sqrt();
    let scale = len / new_len;
    let rdx = (new_dx * scale).round() as CoordinateUnit;
    let rdy = (new_dy * scale).round() as CoordinateUnit;
    // Ensure we don't collapse the direction to zero.
    if rdx == 0 && rdy == 0 {
        (dx, dy)
    } else {
        (rdx, rdy)
    }
}

/// Spawns a small cloud of gun smoke at the firing position.
/// Places persistent SandCloud floor tiles on the map that block sight.
/// Saves the previous floor type for restoration when smoke dissipates.
/// The smoke is biased in the firing direction for a more natural plume.
fn spawn_gun_smoke(
    game_map: &mut GameMapResource,
    origin: GridVec,
    turn: u32,
    fire_dx: i32,
    fire_dy: i32,
) {
    let flen = ((fire_dx as f64).powi(2) + (fire_dy as f64).powi(2)).sqrt();
    let dir = if flen > 0.01 {
        (fire_dx as f64 / flen, fire_dy as f64 / flen)
    } else {
        (0.0, 0.0)
    };
    game_map.place_sand_cloud(
        origin,
        turn,
        dir,
        SMOKE_SCAN_RADIUS,
        SMOKE_BASE_RADIUS,
        SMOKE_DIRECTIONAL_SCALE,
    );
}

/// Resolves attack intents into damage events.
///
/// Damage = attacker.attack + bonus from a random inventory item's blunt_damage.
/// When an entity melee attacks (bump attack), a random item in their inventory
/// provides bonus blunt damage (e.g., pistol-whipping with a gun, smashing with
/// a bottle). Both players and NPCs use this system.
/// Uses `CombatStats::damage_against` for the base damage model.
/// Emits a `DamageEvent` for each successful hit and logs combat messages.
///
/// After dealing damage, nearby same-faction NPCs within 8 tiles become alert;
/// Nearby same-faction NPCs within 8 tiles also become hostile;
/// other-faction NPCs within 8 tiles start fleeing.
pub fn combat_system(
    mut commands: Commands,
    mut intents: MessageReader<AttackIntent>,
    mut damage_events: MessageWriter<DamageEvent>,
    stats_query: Query<(&CombatStats, Option<&Name>, Option<&Position>)>,
    inventory_query: Query<&Inventory>,
    item_kind_query: Query<&ItemKind>,
    dynamic_rng: Res<crate::resources::DynamicRng>,
    seed: Res<crate::resources::MapSeed>,
    mut combat_log: ResMut<CombatLog>,
    faction_query: Query<(Option<&Faction>, Option<&Position>)>,
    npc_query: Query<(Entity, &Position, Option<&Faction>), Without<PlayerControlled>>,
    player_query: Query<(&Position, Option<&Viewshed>), With<PlayerControlled>>,
    turn_counter: Res<TurnCounter>,
) {
    // Collect aggro events to apply after processing all intents
    let mut aggro_targets: Vec<(Entity, Entity, Option<GridVec>)> = Vec::new(); // (attacker, target, target_pos)
    let player_visibility = player_query
        .single()
        .ok()
        .map(|(pos, viewshed)| (pos.as_grid_vec(), viewshed));

    for intent in intents.read() {
        let Ok((attacker_stats, attacker_name, attacker_pos)) = stats_query.get(intent.attacker)
        else {
            continue;
        };
        let Ok((_target_stats, target_name, _)) = stats_query.get(intent.target) else {
            continue;
        };

        let base_damage = attacker_stats.damage_against();
        let a_name = display_name(attacker_name);
        let t_name = display_name(target_name);
        let pos = attacker_pos.map(|p| p.as_grid_vec());

        // Add bonus damage from a random inventory item's blunt_damage.
        let mut bonus = 0;
        let mut bonus_item_name: Option<String> = None;
        if let Ok(inv) = inventory_query.get(intent.attacker)
            && !inv.items.is_empty()
        {
            let idx = dynamic_rng.random_index(
                seed.0,
                intent.attacker.to_bits() ^ 0xB1A7,
                inv.items.len(),
            );
            if let Ok(kind) = item_kind_query.get(inv.items[idx]) {
                let bd = kind.blunt_damage();
                if bd > 0 {
                    bonus = bd;
                    bonus_item_name = Some(kind.display_name());
                }
            }
        }

        let damage = base_damage + bonus;

        if damage > 0 {
            if let Some(event_pos) = pos.filter(|pos| player_can_see_event(*pos, player_visibility)) {
                if let Some(item_name) = bonus_item_name {
                    combat_log.push_at(
                        format!("{a_name} hits {t_name} with {item_name} for {damage} damage"),
                        event_pos,
                    );
                } else {
                    combat_log.push_at(
                        format!("{a_name} hits {t_name} for {damage} damage"),
                        event_pos,
                    );
                }
            }
            damage_events.write(DamageEvent {
                target: intent.target,
                amount: damage,
                source: Some(intent.attacker),
            });
        } else if let Some(event_pos) = pos.filter(|pos| player_can_see_event(*pos, player_visibility))
        {
            combat_log.push_at(
                format!("{a_name} attacks {t_name} but deals no damage"),
                event_pos,
            );
        }

        // Collect aggro info
        let target_pos = if let Ok((_, _, tp)) = stats_query.get(intent.target) {
            tp.map(|p| p.as_grid_vec())
        } else {
            None
        };
        aggro_targets.push((intent.attacker, intent.target, target_pos));
    }

    // Apply aggro: nearby allies of the target should join the fight against the attacker.
    let _player_entity = player_visibility.map(|(pos, _)| pos);
    for (attacker, target, target_pos) in aggro_targets {
        // Get target's faction for propagation
        let target_faction = faction_query.get(target).ok().and_then(|(f, _)| f.copied());
        let attacker_pos = faction_query
            .get(attacker)
            .ok()
            .and_then(|(_, pos)| pos.map(|p| p.as_grid_vec()));

        if let Some(t_pos) = target_pos {
            const AGGRO_RADIUS: i32 = 8;
            for (nearby_ent, nearby_pos, nearby_fac) in &npc_query {
                if nearby_ent == target || nearby_ent == attacker {
                    continue;
                }
                let nv = nearby_pos.as_grid_vec();
                if nv.chebyshev_distance(t_pos) > AGGRO_RADIUS {
                    continue;
                }

                if let Some(&nf) = nearby_fac {
                    if target_faction.is_some_and(|tf| tf == nf) {
                        let last_pos = attacker_pos.unwrap_or(t_pos);
                        commands.entity(nearby_ent).insert((
                            AiState::Chasing,
                            AiTarget {
                                entity: attacker,
                                last_pos,
                                last_seen: turn_counter.0,
                                locked: false,
                            },
                            AiMemory {
                                last_known_pos: Some(last_pos),
                                last_seen_turn: turn_counter.0,
                                ..Default::default()
                            },
                            AiPursuitBoost {
                                extra_range: 8,
                                last_spotted_turn: turn_counter.0,
                            },
                        ));
                    }
                }
            }
        }
    }
}

/// Applies damage events to entity health pools using `Health::apply_damage`.
/// Also records the damage source on the target for kill attribution.
/// When god mode is active, damage to the player is ignored.
///
/// A 1d3 variance is applied to all damage: final = base ± rand(1..=3).
pub fn apply_damage_system(
    mut commands: Commands,
    mut events: MessageReader<DamageEvent>,
    mut health_query: Query<&mut Health>,
    player_query: Query<Entity, With<PlayerControlled>>,
    god_mode: Res<crate::resources::GodMode>,
    dynamic_rng: Res<crate::resources::DynamicRng>,
    seed: Res<crate::resources::MapSeed>,
) {
    let player_entity = player_query.single().ok();
    for (event_index, event) in events.read().enumerate() {
        // God mode: skip damage to the player.
        if god_mode.0 && player_entity == Some(event.target) {
            continue;
        }
        if let Ok(mut health) = health_query.get_mut(event.target) {
            // 1d3 variance: roll [0.0, 1.0), map to {-3,-2,-1,+1,+2,+3}
            let source_bits = event.source.map(Entity::to_bits).unwrap_or(0);
            let roll = dynamic_rng.roll(
                seed.0,
                event.target.to_bits()
                    ^ source_bits.rotate_left(11)
                    ^ (event.amount as u64).rotate_left(23)
                    ^ (event_index as u64).wrapping_mul(0x9E37_79B9),
            );
            let variance = if roll < 0.5 {
                // Negative: map [0, 0.5) → {-3, -2, -1}
                -((roll * 6.0) as i32 + 1).min(3)
            } else {
                // Positive: map [0.5, 1.0) → {1, 2, 3}
                (((roll - 0.5) * 6.0) as i32 + 1).min(3)
            };
            let final_damage = if event.amount > 0 {
                (event.amount + variance).max(1)
            } else {
                0
            };
            health.apply_damage(final_damage);
            if let Some(source) = event.source {
                commands
                    .entity(event.target)
                    .insert(LastDamageSource(source));
            }
        }
    }
}

/// Despawns entities whose health has reached zero.
/// Logs a death message, increments the kill counter for hostile entities
/// killed by the player, drops the entity's entire inventory on the ground,
/// and removes the entity.
/// Animals (Wildlife faction) drop nothing. Non-wildlife NPCs drop their full
/// inventory including guns with ammo.
/// If the player dies, transitions to the Dead state.
pub fn death_system(
    mut commands: Commands,
    query: Query<(
        Entity,
        &Health,
        Option<&Name>,
        Option<&Position>,
        Option<&LootTable>,
        Option<&PlayerControlled>,
        Option<&LastDamageSource>,
        Option<&Inventory>,
        Option<&Faction>,
    )>,
    player_entities: Query<Entity, With<PlayerControlled>>,
    item_query: Query<&ItemKind>,
    mut combat_log: ResMut<CombatLog>,
    mut kill_count: ResMut<KillCount>,
    _next_game_state: ResMut<NextState<GameState>>,
    seed: Res<MapSeed>,
    mut spectating: ResMut<crate::resources::SpectatingAfterDeath>,
    player_visibility_query: Query<(&Position, Option<&Viewshed>), With<PlayerControlled>>,
) {
    let player_entity = player_entities.single().ok();
    let player_visibility = player_visibility_query
        .single()
        .ok()
        .map(|(pos, viewshed)| (pos.as_grid_vec(), viewshed));

    for (
        entity,
        health,
        name,
        pos,
        loot_table,
        is_player,
        last_damage_source,
        inventory,
        faction,
    ) in &query
    {
        if !health.is_dead() {
            continue;
        }

        // If the player died and we already processed their death (spectating),
        // skip entirely — don't re-log messages or re-trigger state changes.
        if is_player.is_some() && spectating.0 {
            continue;
        }

        let label = name.map_or("Something", |n| &n.0);
        if let Some(event_pos) = pos
            .map(|p| p.as_grid_vec())
            .filter(|pos| player_can_see_event(*pos, player_visibility))
        {
            combat_log.push_at(format!("{label} has been slain!"), event_pos);
        }

        // If the player died, mark them dead and enable spectating so the
        // world keeps ticking. The input system blocks player actions when
        // the Dead component is present.
        if is_player.is_some() {
            combat_log.push("You have fallen... Press R to restart.".into());
            spectating.0 = true; // auto-advance turns while dead
            commands.entity(entity).insert(Dead);
            if let Some(p) = pos {
                commands.spawn((
                    Position { x: p.x, y: p.y },
                    Name(format!("{label}'s corpse")),
                    Renderable {
                        symbol: "X".into(),
                        fg: RatColor::Rgb(120, 60, 60),
                        bg: RatColor::Black,
                    },
                ));
            }
            continue; // don't despawn the player (UI reads stats from it)
        }

        // Count kills for any entity with a faction (hostility is faction-based).
        if faction.is_some() {
            let player_killed =
                player_entity.is_some_and(|pe| last_damage_source.is_some_and(|lds| lds.0 == pe));
            if player_killed {
                kill_count.0 += 1;
            }
        }
        let is_wildlife = faction.is_some_and(|f| matches!(f, Faction::Wildlife));

        // Drop entire NPC inventory on the ground (animals drop nothing).
        if !is_wildlife && let (Some(inv), Some(_p)) = (inventory, pos) {
            for &item_entity in &inv.items {
                commands
                    .entity(item_entity)
                    .insert(Position { x: _p.x, y: _p.y });
            }
        }

        // Loot drop: non-wildlife entities with a LootTable may also drop collectible supplies.
        // Ammo drops now match the caliber of any gun the NPC was carrying.
        if !is_wildlife && let (Some(_lt), Some(p)) = (loot_table, pos) {
            // Drop collectible supplies (caps + powder + matching ammo).
            let coll_roll = value_noise(
                p.x.wrapping_add(kill_count.0 as i32 + 1),
                p.y,
                seed.0.wrapping_add(33333),
            );
            if coll_roll < 0.5 {
                let caps_amount = ((coll_roll * 20.0) as i32).max(1);
                commands.spawn((
                    Position { x: p.x, y: p.y },
                    Item,
                    Name(format!("{caps_amount} Caps")),
                    Renderable {
                        symbol: "$".into(),
                        fg: RatColor::Rgb(255, 215, 0),
                        bg: RatColor::Black,
                    },
                    CollectibleKind::Caps(caps_amount),
                ));
            }
            // Drop powder (needed for reloading).
            let powder_roll = value_noise(
                p.x.wrapping_add(kill_count.0 as i32 + 2),
                p.y,
                seed.0.wrapping_add(55555),
            );
            if powder_roll < 0.4 {
                let amount = ((powder_roll * 10.0) as i32).max(1);
                commands.spawn((
                    Position { x: p.x, y: p.y },
                    Item,
                    Name(format!("{amount}x Powder")),
                    Renderable {
                        symbol: "·".into(),
                        fg: RatColor::Rgb(140, 140, 140),
                        bg: RatColor::Black,
                    },
                    CollectibleKind::Powder(amount),
                ));
            }
            // Drop ammo matching the NPC's weapon, if they had one.
            let ammo_roll = value_noise(
                p.y.wrapping_add(kill_count.0 as i32 + 1),
                p.x,
                seed.0.wrapping_add(44444),
            );
            if ammo_roll < 0.4 {
                let amount = ((ammo_roll * 15.0) as i32).max(1);
                let arrow_amount = amount.max(2);
                let ammo_drop = inventory.and_then(|inv| {
                    inv.items.iter().find_map(|&ent| {
                        item_query.get(ent).ok().and_then(|k| match k {
                            ItemKind::Gun { caliber, .. } => Some((
                                format!("{amount}x {caliber} Bullets"),
                                Renderable {
                                    symbol: "·".into(),
                                    fg: RatColor::Rgb(180, 180, 180),
                                    bg: RatColor::Black,
                                },
                                match caliber {
                                    Caliber::Cal31 => CollectibleKind::Bullets31(amount),
                                    Caliber::Cal36 => CollectibleKind::Bullets36(amount),
                                    Caliber::Cal44 => CollectibleKind::Bullets44(amount),
                                    Caliber::Cal50 => CollectibleKind::Bullets50(amount),
                                    Caliber::Cal58 => CollectibleKind::Bullets58(amount),
                                    Caliber::Cal577 => CollectibleKind::Bullets577(amount),
                                    Caliber::Cal69 => CollectibleKind::Bullets69(amount),
                                },
                            )),
                            ItemKind::Bow { .. } => Some((
                                format!("{arrow_amount}x Arrows"),
                                Renderable {
                                    symbol: "|".into(),
                                    fg: RatColor::Rgb(170, 120, 70),
                                    bg: RatColor::Black,
                                },
                                CollectibleKind::Arrows(arrow_amount),
                            )),
                            _ => None,
                        })
                    })
                });
                if let Some((name, renderable, collectible)) = ammo_drop {
                    commands.spawn((
                        Position { x: p.x, y: p.y },
                        Item,
                        Name(name),
                        renderable,
                        collectible,
                    ));
                }
            }
        }

        // Spawn a corpse marker for human (non-wildlife) NPCs.
        if !is_wildlife && let Some(p) = pos {
            commands.spawn((
                Position { x: p.x, y: p.y },
                Name(format!("{label}'s corpse")),
                Renderable {
                    symbol: "X".into(),
                    fg: RatColor::Rgb(120, 60, 60),
                    bg: RatColor::Black,
                },
            ));
        }

        commands.entity(entity).despawn();
    }
}

/// Small chance for a gun to misfire (ammo consumed but no bullet fires).
const MISFIRE_CHANCE: f64 = 0.05;

/// Resolves targeted ranged attack intents by spawning bullet projectile entities.
///
/// The bullet path is computed using **Bresenham's line algorithm** from the
/// caster's position to the maximum range endpoint. Instead of applying damage
/// instantly, a bullet entity is spawned that travels along the path over
/// multiple ticks. Damage is applied when the projectile reaches a hostile.
///
/// Works for both the player and NPCs — the attacker entity is taken from the
/// intent. Consumes 1 loaded round from the gun item.
/// There is a small chance for the gun to misfire.
pub fn ranged_attack_system(
    mut commands: Commands,
    mut intents: MessageReader<RangedAttackIntent>,
    mut caster_query: Query<(&Position, Option<&Name>)>,
    mut combat_log: ResMut<CombatLog>,
    mut collectibles: ResMut<Collectibles>,
    mut item_kind_query: Query<&mut ItemKind>,
    mut sound_events: ResMut<SoundEvents>,
    dynamic_rng: Res<DynamicRng>,
    seed: Res<MapSeed>,
    mut game_map: ResMut<GameMapResource>,
    turn_counter: Res<TurnCounter>,
    player_visibility_query: Query<(&Position, Option<&Viewshed>), With<PlayerControlled>>,
) {
    enum ShotKind {
        Gun,
        Bow,
    }

    let player_visibility = player_visibility_query
        .single()
        .ok()
        .map(|(pos, viewshed)| (pos.as_grid_vec(), viewshed));
    for intent in intents.read() {
        let Ok((caster_pos, caster_name)) = caster_query.get_mut(intent.attacker) else {
            continue;
        };
        let origin = caster_pos.as_grid_vec();
        let c_name = display_name(caster_name);

        // Determine damage and consume a loaded round from the gun item.
        // Damage is strictly based on caliber: .31 cal = 31 damage, etc.
        let (damage, shot_kind);
        if let Some(gun_entity) = intent.gun_item {
            if let Ok(mut kind) = item_kind_query.get_mut(gun_entity) {
                match kind.as_mut() {
                    ItemKind::Gun {
                        loaded, caliber, ..
                    } => {
                        if *loaded <= 0 {
                            combat_log.push("Gun is empty!".into());
                            continue;
                        }
                        *loaded -= 1;
                        (damage, shot_kind) = (caliber.damage(), ShotKind::Gun);
                    }
                    ItemKind::Bow { attack, .. } => {
                        if collectibles.arrows <= 0 {
                            combat_log.push("No arrows!".into());
                            continue;
                        }
                        collectibles.arrows -= 1;
                        (damage, shot_kind) = (*attack, ShotKind::Bow);
                    }
                    _ => continue,
                }
            } else {
                continue;
            }
        } else {
            combat_log.push("No weapon available!".into());
            continue;
        }

        let dx = intent.dx;
        let dy = intent.dy;

        if dx == 0 && dy == 0 {
            combat_log.push("Invalid aim direction!".into());
            continue;
        }

        if matches!(shot_kind, ShotKind::Gun) {
            // Misfire check: small chance the gun misfires (ammo wasted, no bullet).
            let misfire_roll = dynamic_rng.roll(
                seed.0,
                (origin.x as u64) << 32 | (origin.y as u64 & 0xFFFFFFFF) ^ 0xDEAD,
            );
            if misfire_roll < MISFIRE_CHANCE {
                if player_can_see_event(origin, player_visibility) {
                    combat_log.push_at(format!("{c_name}'s gun misfires! *click*"), origin);
                }
                sound_events.add(origin);
                continue;
            }

            // Apply gun spread: slight random angular offset to the trajectory.
            let spread_roll = dynamic_rng.roll(
                seed.0,
                (origin.x as u64).wrapping_mul(7) ^ (origin.y as u64) ^ 0x5BAD,
            );
            let (spread_dx, spread_dy) = apply_gun_spread(dx, dy, spread_roll);

            // Compute the bullet endpoint — bullets travel until they hit a wall,
            // entity, or reach the map boundary. No maximum range cap.
            let map_range = game_map.0.width.max(game_map.0.height);
            let endpoint = bullet_endpoint(origin, spread_dx, spread_dy, map_range);

            sound_events.add(origin);

            // Spawn gun smoke at the firing position (persists and blocks sight).
            spawn_gun_smoke(&mut game_map, origin, turn_counter.0, dx, dy);

            // Spawn a bullet projectile entity that will travel along the path.
            crate::systems::projectile::spawn_bullet(
                &mut commands,
                origin,
                endpoint,
                damage,
                intent.attacker,
            );
        } else {
            let endpoint = bullet_endpoint(origin, dx, dy, intent.range.max(1));
            crate::systems::projectile::spawn_arrow(
                &mut commands,
                origin,
                endpoint,
                damage,
                intent.attacker,
            );
        }
    }
}

/// Resolves AI ranged attack intents by spawning bullet projectile entities.
/// Fires a bullet from the attacker toward the target entity.
pub fn ai_ranged_attack_system(
    mut commands: Commands,
    mut intents: MessageReader<AiRangedAttackIntent>,
    attacker_query: Query<(&Position, &CombatStats, Option<&Name>, Option<&Inventory>)>,
    target_query: Query<&Position>,
    mut item_kind_query: Query<&mut ItemKind>,
    mut sound_events: ResMut<SoundEvents>,
    mut game_map: ResMut<GameMapResource>,
    turn_counter: Res<TurnCounter>,
) {
    for intent in intents.read() {
        let Ok((attacker_pos, attacker_stats, attacker_name, inventory)) =
            attacker_query.get(intent.attacker)
        else {
            continue;
        };
        let Ok(target_pos) = target_query.get(intent.target) else {
            continue;
        };

        let origin = attacker_pos.as_grid_vec();
        let target_vec = target_pos.as_grid_vec();
        let _ = attacker_name;

        // Use the actual direction delta for accurate aiming (not just signum).
        let dx = target_vec.x - origin.x;
        let dy = target_vec.y - origin.y;

        if dx == 0 && dy == 0 {
            continue;
        }

        // NPC consumes a loaded round from their gun, just like the player.
        // Damage is strictly based on caliber: .31 cal = 31 damage, etc.
        let mut damage = attacker_stats.attack;
        if let Some(inv) = inventory {
            let mut fired = false;
            for &item_ent in &inv.items {
                if let Ok(mut kind) = item_kind_query.get_mut(item_ent)
                    && let ItemKind::Gun {
                        ref mut loaded,
                        caliber,
                        ..
                    } = *kind
                    && *loaded > 0
                {
                    *loaded -= 1;
                    damage = caliber.damage();
                    fired = true;
                    break;
                }
            }
            if !fired {
                // No loaded gun — NPC can't fire this turn.
                continue;
            }
        }

        // Apply gun spread: slight random angular offset to the trajectory.
        let spread_seed =
            (origin.x as u64).wrapping_mul(13) ^ (origin.y as u64) ^ intent.attacker.to_bits();
        let spread_roll = crate::noise::value_noise(origin.x, origin.y, spread_seed);
        let (spread_dx, spread_dy) = apply_gun_spread(dx, dy, spread_roll);

        // Compute bullet endpoint — bullets travel until impact or map boundary.
        let map_range = game_map.0.width.max(game_map.0.height);
        let endpoint = bullet_endpoint(origin, spread_dx, spread_dy, map_range);

        sound_events.add(origin);

        // Spawn gun smoke at the firing position (persists and blocks sight).
        spawn_gun_smoke(&mut game_map, origin, turn_counter.0, dx, dy);

        // Spawn a bullet projectile entity.
        crate::systems::projectile::spawn_bullet(
            &mut commands,
            origin,
            endpoint,
            damage,
            intent.attacker,
        );
    }
}

/// Resolves roundhouse kick attack intents as an AoE push attack.
/// Attempts to push all adjacent entities (Chebyshev distance 1) away from the
/// attacker by one tile. Only pushes if the destination tile is unoccupied and passable.
/// Deals damage to all adjacent entities (not just hostile ones).
/// Also damages adjacent props; props are only destroyed when their health reaches zero.
///
/// Minimum prop damage per kick to ensure weak kicks still damage props.
const MIN_PROP_DAMAGE: i32 = 5;
const ROUNDHOUSE_STAMINA_COST: i32 = 10;
pub fn melee_wide_system(
    _commands: Commands,
    mut intents: MessageReader<MeleeWideIntent>,
    mut damage_events: MessageWriter<DamageEvent>,
    mut attacker_query: Query<
        (&Position, &CombatStats, Option<&Name>, Option<&mut Stamina>),
        With<PlayerControlled>,
    >,
    mut targets: Query<
        (Entity, &mut Position, Option<&Name>, Option<&Health>),
        Without<PlayerControlled>,
    >,
    blockers: Query<(), With<crate::components::BlocksMovement>>,
    mut combat_log: ResMut<CombatLog>,
    mut game_map: ResMut<GameMapResource>,
    spatial: Res<crate::resources::SpatialIndex>,
    mut prop_health: ResMut<crate::resources::PropHealth>,
    turn_counter: Res<TurnCounter>,
) {
    for intent in intents.read() {
        let Ok((attacker_pos, attacker_stats, attacker_name, attacker_stamina)) =
            attacker_query.get_mut(intent.attacker)
        else {
            continue;
        };
        if let Some(mut stamina) = attacker_stamina
            && !stamina.spend(ROUNDHOUSE_STAMINA_COST)
        {
            combat_log.push("Not enough stamina for roundhouse!".into());
            continue;
        }
        let origin = attacker_pos.as_grid_vec();
        let a_name = display_name(attacker_name);
        let damage = attacker_stats.damage_against();
        let mut hit_count = 0;
        let mut push_count = 0;

        // Collect entities to push (can't modify positions while iterating spatially)
        let mut push_list: Vec<(Entity, GridVec, GridVec)> = Vec::new(); // (entity, current, push_to)

        for (target_entity, target_pos, target_name, _target_hp) in &targets {
            let tv = target_pos.as_grid_vec();
            let dist = origin.chebyshev_distance(tv);
            if dist != 1 || target_entity == intent.attacker {
                continue;
            }

            // Deal damage
            let t_name = display_name(target_name);
            if damage > 0 {
                damage_events.write(DamageEvent {
                    target: target_entity,
                    amount: damage,
                    source: Some(intent.attacker),
                });
                combat_log.push(format!(
                    "{a_name} roundhouse kicks {t_name} for {damage} damage!"
                ));
                hit_count += 1;
            }

            // Calculate push direction (away from attacker)
            let push_dir = (tv - origin).king_step();
            let push_to = tv + push_dir;

            // Check if push destination is passable and unoccupied
            let tile_passable = game_map.0.is_passable(&push_to);
            let entity_blocked = spatial
                .entities_at(&push_to)
                .iter()
                .any(|&e| e != target_entity && blockers.contains(e));

            if tile_passable && !entity_blocked {
                push_list.push((target_entity, tv, push_to));
            }
        }

        // Apply pushes
        for (entity, _from, to) in &push_list {
            if let Ok((_, mut pos, _, _)) = targets.get_mut(*entity) {
                pos.x = to.x;
                pos.y = to.y;
                push_count += 1;
            }
        }

        // Damage adjacent props (using prop health system)
        let mut props_damaged = 0;
        for dx in -1..=1i32 {
            for dy in -1..=1i32 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let tile = origin + GridVec::new(dx, dy);
                if let Some(voxel) = game_map.0.get_voxel_at(&tile)
                    && let Some(ref prop) = voxel.props
                {
                    let max_hp = prop.max_health();
                    if max_hp == i32::MAX {
                        continue; // indestructible
                    }
                    let is_gunpowder = matches!(prop, Props::GunpowderBarrel);
                    // Initialize prop health if not yet tracked
                    let current_hp = prop_health.hp.entry(tile).or_insert(max_hp);
                    *current_hp -= damage.max(MIN_PROP_DAMAGE);
                    props_damaged += 1;
                    if *current_hp <= 0 {
                        // Destroy the prop
                        if let Some(voxel) = game_map.0.get_voxel_at_mut(&tile) {
                            voxel.props = None;
                        }
                        prop_health.hp.remove(&tile);
                        if is_gunpowder {
                            // Gunpowder barrel explodes — set fire in radius
                            combat_log.push("Gunpowder barrel explodes!".to_string());
                            game_map.detonate_gunpowder_barrel(tile, turn_counter.0);
                        } else {
                            combat_log.push(format!("{a_name} destroys a prop!"));
                        }
                    }
                }
            }
        }

        if hit_count == 0 && props_damaged == 0 {
            combat_log.push(format!("{a_name} roundhouse kicks but hits nothing!"));
        } else if push_count > 0 {
            combat_log.push(format!("{a_name} pushes {push_count} away!"));
        }
    }
}
