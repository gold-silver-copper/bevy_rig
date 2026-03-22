use bevy::prelude::*;

use crate::components::{
    CombatStats, Inventory, Item, ItemKind, Name, Position, Projectile, ProjectileVisual,
    Renderable, SPELL_STAMINA_COST, Stamina, ThrownExplosive, display_name,
};
use crate::events::{MolotovCastIntent, SpellCastIntent};
use crate::grid_vec::GridVec;
use crate::resources::{CombatLog, GameMapResource, MapSeed, SpellParticles, TurnCounter};
use crate::systems::input::SAND_STAMINA_COST;
use crate::typedefs::RatColor;
use crate::typeenums::{Floor, Props};

/// Resolves grenade throw intents by spawning shrapnel projectile entities.
///
/// For each `SpellCastIntent`, the player throws a frag grenade that detonates
/// at the target position, spawning shrapnel projectile entities in all
/// directions within the specified radius. Shrapnel entities travel outward
/// over ticks and apply damage when they reach hostile entities.
/// Dynamite also sets some flammable objects on fire.
/// Consumes stamina from the caster.
pub fn spell_system(
    mut commands: Commands,
    mut intents: MessageReader<SpellCastIntent>,
    mut caster_query: Query<(
        &CombatStats,
        Option<&Name>,
        Option<&mut Stamina>,
        Option<&mut Inventory>,
        Option<&Position>,
    )>,
    mut combat_log: ResMut<CombatLog>,
    mut game_map: ResMut<GameMapResource>,
    turn_counter: Res<TurnCounter>,
) {
    for intent in intents.read() {
        let Ok((caster_stats, caster_name, stamina, inventory, caster_pos)) =
            caster_query.get_mut(intent.caster)
        else {
            continue;
        };

        // Sand throw (sentinel: grenade_index == usize::MAX) — creates persistent
        // sand cloud tiles on the map that block line of sight.
        // The cloud is circular but biased to spread toward the target
        // direction (plume shape away from the caster).
        if intent.grenade_index == usize::MAX {
            if let Some(mut stamina) = stamina {
                stamina.spend(SAND_STAMINA_COST);
            }
            let origin = intent.target;
            let radius_f = intent.radius as f64;
            let caster_vec = caster_pos.map(|p| p.as_grid_vec());
            let dir = caster_vec
                .map(|cv| {
                    let d = origin - cv;
                    let len = ((d.x as f64).powi(2) + (d.y as f64).powi(2)).sqrt();
                    if len > 0.01 {
                        (d.x as f64 / len, d.y as f64 / len)
                    } else {
                        (0.0, 0.0)
                    }
                })
                .unwrap_or((0.0, 0.0));

            let scan_radius = intent.radius + 1;
            let base_radius = radius_f * 0.5;
            let directional_scale = radius_f;
            game_map.place_sand_cloud(
                origin,
                turn_counter.0,
                dir,
                scan_radius,
                base_radius,
                directional_scale,
            );
            continue;
        }

        // Consume stamina.
        if let Some(mut stamina) = stamina
            && !stamina.spend(SPELL_STAMINA_COST)
        {
            combat_log.push("Not enough stamina!".into());
            continue;
        }

        // Consume the grenade item from inventory.
        if let Some(mut inv) = inventory
            && let Some(grenade_entity) = inv.remove_at(intent.grenade_index)
        {
            commands.entity(grenade_entity).despawn();
        }

        let caster_gv = caster_pos.map(|p| p.as_grid_vec()).unwrap_or(intent.target);
        let c_name = display_name(caster_name);
        combat_log.push(format!("{c_name} throws dynamite!"));
        // Spawn a traveling explosive projectile toward the target.
        spawn_explosive_projectile(
            &mut commands,
            caster_gv,
            intent.target,
            ThrownExplosive::Dynamite {
                damage: caster_stats.attack,
                radius: intent.radius,
                grenade_index: intent.grenade_index,
            },
            intent.caster,
        );
    }
}

/// Resolves molotov cocktail throws.
/// Spawns a traveling explosive projectile that detonates on first impact,
/// igniting flammable props and generating smoke.
pub fn molotov_system(
    mut commands: Commands,
    mut intents: MessageReader<MolotovCastIntent>,
    mut caster_query: Query<(
        &CombatStats,
        Option<&Name>,
        Option<&mut Stamina>,
        Option<&mut Inventory>,
        Option<&Position>,
    )>,
    mut combat_log: ResMut<CombatLog>,
) {
    for intent in intents.read() {
        let Ok((_caster_stats, caster_name, stamina, inventory, caster_pos)) =
            caster_query.get_mut(intent.caster)
        else {
            continue;
        };

        // Consume stamina.
        if let Some(mut stamina) = stamina
            && !stamina.spend(SPELL_STAMINA_COST)
        {
            combat_log.push("Not enough stamina!".into());
            continue;
        }

        // Consume the molotov item from inventory.
        if let Some(mut inv) = inventory
            && let Some(molotov_entity) = inv.remove_at(intent.item_index)
        {
            commands.entity(molotov_entity).despawn();
        }

        let caster_gv = caster_pos.map(|p| p.as_grid_vec()).unwrap_or(intent.target);
        let c_name = display_name(caster_name);
        combat_log.push(format!("{c_name} hurls a Molotov cocktail!"));
        // Spawn a traveling explosive projectile toward the target.
        spawn_explosive_projectile(
            &mut commands,
            caster_gv,
            intent.target,
            ThrownExplosive::Molotov {
                damage: intent.damage,
                radius: intent.radius,
                item_index: intent.item_index,
            },
            intent.caster,
        );
    }
}

/// Spawns a random loot item when a container (crate/barrel) is destroyed.
/// Containers have a chance to drop supplies or small items.
fn spawn_container_loot(commands: &mut Commands, x: i32, y: i32, roll: f64) {
    if roll < 0.3 {
        commands.spawn((
            Position { x, y },
            Item,
            Name("Whiskey".into()),
            Renderable {
                symbol: "w".into(),
                fg: RatColor::Rgb(180, 120, 60),
                bg: RatColor::Black,
            },
            ItemKind::Whiskey {
                heal: 10,
                blunt_damage: 4,
            },
        ));
    } else if roll < 0.5 {
        commands.spawn((
            Position { x, y },
            Item,
            Name("Knife".into()),
            Renderable {
                symbol: "/".into(),
                fg: RatColor::Rgb(192, 192, 210),
                bg: RatColor::Black,
            },
            ItemKind::Knife {
                attack: 4,
                blunt_damage: 6,
            },
        ));
    } else if roll < 0.65 {
        commands.spawn((
            Position { x, y },
            Item,
            Name("Dynamite".into()),
            Renderable {
                symbol: "*".into(),
                fg: RatColor::Rgb(255, 165, 0),
                bg: RatColor::Black,
            },
            ItemKind::Grenade {
                damage: 8,
                radius: 2,
                blunt_damage: 3,
            },
        ));
    }
    // else: no drop (35% chance)
}

/// Spawns smoke (SandCloud) around a molotov detonation point, similar to gun smoke.
/// The smoke cloud is larger than gun smoke to represent the thick black smoke from fire.
fn spawn_molotov_smoke(
    game_map: &mut GameMapResource,
    origin: crate::grid_vec::GridVec,
    turn: u32,
    radius: i32,
) {
    let smoke_radius = (radius / 2).max(2);
    let mut tiles_to_cloud: Vec<(crate::grid_vec::GridVec, Option<Floor>)> = Vec::new();
    for dx in -smoke_radius..=smoke_radius {
        for dy in -smoke_radius..=smoke_radius {
            let dist = ((dx as f64).powi(2) + (dy as f64).powi(2)).sqrt();
            if dist > smoke_radius as f64 + 0.5 {
                continue;
            }
            let pos = origin + crate::grid_vec::GridVec::new(dx, dy);
            if let Some(voxel) = game_map.0.get_voxel_at(&pos)
                && !matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall))
                && !matches!(voxel.floor, Some(Floor::Fire))
            {
                tiles_to_cloud.push((pos, voxel.floor.clone()));
            }
        }
    }
    for (pos, prev_floor) in tiles_to_cloud {
        game_map
            .0
            .sand_cloud_previous_floor
            .entry(pos)
            .or_insert(prev_floor);
        if let Some(voxel) = game_map.0.get_voxel_at_mut(&pos) {
            voxel.floor = Some(Floor::SandCloud);
        }
        game_map.0.sand_cloud_turns.insert(pos, turn);
    }
}

/// Thrown explosive travel speed in tiles per tick.
const EXPLOSIVE_TILES_PER_TICK: usize = 2;

/// Spawns a projectile entity carrying a thrown explosive (dynamite or molotov).
/// The projectile travels along a Bresenham line from `origin` to `target`
/// and detonates on the first thing it hits (entity, wall, or target tile).
/// If origin equals target, the explosive spawns at the origin and detonates
/// immediately on the next system tick.
fn spawn_explosive_projectile(
    commands: &mut Commands,
    origin: GridVec,
    target: GridVec,
    explosive: ThrownExplosive,
    source: Entity,
) {
    let path = origin.bresenham_line(target);
    // Bresenham always returns at least the origin point, so path is never empty.
    // If target == origin, path = [origin] and we start at index 0.
    // If target != origin, path = [origin, ..., target] and we skip the origin.
    let (start_pos, start_index) = if path.len() <= 1 {
        (origin, 0)
    } else {
        (path[1], 1)
    };
    let (symbol, fg) = match &explosive {
        ThrownExplosive::Dynamite { .. } => ("*", RatColor::Rgb(255, 165, 0)),
        ThrownExplosive::Molotov { .. } => ("m", RatColor::Rgb(255, 100, 0)),
    };
    commands.spawn((
        Position {
            x: start_pos.x,
            y: start_pos.y,
        },
        Renderable {
            symbol: symbol.into(),
            fg,
            bg: RatColor::Black,
        },
        Projectile {
            path,
            path_index: start_index,
            tiles_per_tick: EXPLOSIVE_TILES_PER_TICK,
            damage: 0,
            penetration: 0,
            source,
            tail_pos: None,
            visual: ProjectileVisual::Asterisk,
            is_bullet: false,
            tile_timer: 0.0,
        },
        explosive,
    ));
}

/// Advances thrown explosive projectiles and detonates them on impact.
/// Runs after the normal projectile system. When an explosive projectile
/// hits a wall, an entity, or reaches the end of its path, it detonates
/// at that position.
pub fn explosive_projectile_system(
    mut commands: Commands,
    mut explosives: Query<(Entity, &mut Position, &mut Projectile, &ThrownExplosive)>,
    blockers: Query<
        Entity,
        (
            bevy::prelude::With<crate::components::BlocksMovement>,
            bevy::prelude::Without<Projectile>,
        ),
    >,
    spatial: Res<crate::resources::SpatialIndex>,
    mut combat_log: ResMut<CombatLog>,
    mut game_map: ResMut<GameMapResource>,
    mut spell_particles: ResMut<SpellParticles>,
    seed: Res<MapSeed>,
    turn_counter: Res<TurnCounter>,
) {
    for (proj_entity, mut proj_pos, mut proj, explosive) in &mut explosives {
        let steps = proj.tiles_per_tick;
        let mut detonate_pos: Option<GridVec> = None;

        for _ in 0..steps {
            let tile = proj.path[proj.path_index];
            proj_pos.x = tile.x;
            proj_pos.y = tile.y;

            // Check for wall hit (windows let projectiles through)
            if !game_map.0.is_passable_for_projectiles(&tile) {
                detonate_pos = Some(tile);
                break;
            }

            // Check for blocking entity at this tile (not the source)
            let ents = spatial.entities_at(&tile);
            let hit_entity = ents
                .iter()
                .any(|&e| e != proj.source && blockers.contains(e));
            if hit_entity {
                detonate_pos = Some(tile);
                break;
            }

            // Advance
            proj.path_index += 1;
            if proj.path_index >= proj.path.len() {
                detonate_pos = Some(tile);
                break;
            }
        }

        if let Some(det_pos) = detonate_pos {
            // Detonate at this position
            match explosive {
                ThrownExplosive::Dynamite { damage, radius, .. } => {
                    detonate_dynamite(
                        &mut commands,
                        &mut game_map,
                        &mut combat_log,
                        &seed,
                        &turn_counter,
                        det_pos,
                        *damage,
                        *radius,
                        proj.source,
                    );
                }
                ThrownExplosive::Molotov { damage, radius, .. } => {
                    detonate_molotov(
                        &mut commands,
                        &mut game_map,
                        &mut combat_log,
                        &mut spell_particles,
                        &turn_counter,
                        det_pos,
                        *damage,
                        *radius,
                        proj.source,
                    );
                }
            }
            commands.entity(proj_entity).despawn();
        }
    }
}

/// Detonates dynamite at the given position: spawns shrapnel and destroys obstacles.
/// Dynamite destroys ALL props unconditionally — walls, stone walls, barrels, crates,
/// signs, windows, and everything else within the blast radius.
fn detonate_dynamite(
    commands: &mut Commands,
    game_map: &mut ResMut<GameMapResource>,
    combat_log: &mut ResMut<CombatLog>,
    seed: &Res<MapSeed>,
    turn_counter: &Res<TurnCounter>,
    origin: GridVec,
    damage: i32,
    radius: i32,
    source: Entity,
) {
    crate::systems::projectile::spawn_shrapnel(commands, origin, radius, damage, source);

    let mut destroyed_count = 0;
    let mut water_count = 0;
    let mut gunpowder_positions: Vec<GridVec> = Vec::new();
    // Collect tiles that had fire so we can clear fire tracking data after the voxel loop.
    let mut fire_clear_positions: Vec<GridVec> = Vec::new();
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            let dist = dx.abs().max(dy.abs());
            if dist > 0 && dist <= radius {
                let target_pos = origin + GridVec::new(dx, dy);
                if let Some(voxel) = game_map.0.get_voxel_at_mut(&target_pos) {
                    // Clear Floor::Fire to avoid double-ignition artifacts.
                    if matches!(voxel.floor, Some(Floor::Fire)) {
                        fire_clear_positions.push(target_pos);
                    }

                    if let Some(ref prop) = voxel.props {
                        let is_water_trough = matches!(prop, Props::WaterTrough);
                        let is_lootable = matches!(prop, Props::Crate | Props::Barrel);
                        let is_gunpowder = matches!(prop, Props::GunpowderBarrel);
                        if is_water_trough {
                            voxel.props = None;
                            voxel.floor = Some(Floor::Water);
                            water_count += 1;
                        } else {
                            if is_gunpowder {
                                gunpowder_positions.push(target_pos);
                            }
                            if is_lootable {
                                let loot_roll = crate::noise::value_noise(
                                    target_pos.x,
                                    target_pos.y,
                                    seed.0.wrapping_add(88888),
                                );
                                spawn_container_loot(
                                    commands,
                                    target_pos.x,
                                    target_pos.y,
                                    loot_roll,
                                );
                            }
                            // Dynamite destroys everything unconditionally.
                            voxel.props = None;
                            destroyed_count += 1;
                        }
                    }
                }
            }
        }
    }
    // Restore previous floor for tiles that were on fire (avoid double-ignition).
    for pos in fire_clear_positions {
        let prev = game_map.0.fire_previous_floor.remove(&pos).flatten();
        game_map.0.fire_turns.remove(&pos);
        if let Some(voxel) = game_map.0.get_voxel_at_mut(&pos) {
            voxel.floor = prev.or(Some(Floor::ScorchedEarth));
        }
    }
    // Chain-react gunpowder barrels: each one explodes in a fire radius
    for gp_pos in &gunpowder_positions {
        combat_log.push("Gunpowder barrel explodes!".to_string());
        let fire_destroyed = game_map.detonate_gunpowder_barrel(*gp_pos, turn_counter.0);
        if fire_destroyed > 0 {
            combat_log.push(format!("{fire_destroyed} object(s) catch fire!"));
        }
    }
    if destroyed_count > 0 {
        combat_log.push(format!(
            "The explosion destroys {destroyed_count} obstacle(s)!"
        ));
    }
    if water_count > 0 {
        combat_log.push(format!("{water_count} water trough(s) spill water!"));
    }
}

/// Detonates a molotov at the given position: sets area on fire and generates smoke.
/// Molotovs produce fire only — no shrapnel.
fn detonate_molotov(
    _commands: &mut Commands,
    game_map: &mut ResMut<GameMapResource>,
    combat_log: &mut ResMut<CombatLog>,
    spell_particles: &mut ResMut<SpellParticles>,
    turn_counter: &Res<TurnCounter>,
    origin: GridVec,
    _damage: i32,
    radius: i32,
    _source: Entity,
) {
    spell_particles.add_aoe(origin, 6);

    let mut fire_count = 0;
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            let dist = dx.abs().max(dy.abs());
            if dist <= radius {
                let target_pos = origin + GridVec::new(dx, dy);
                if let Some(voxel) = game_map.0.get_voxel_at_mut(&target_pos) {
                    if let Some(ref prop) = voxel.props {
                        if prop.is_flammable() {
                            voxel.props = None;
                            voxel.floor = Some(Floor::Fire);
                            fire_count += 1;
                        }
                    } else if dist <= 1 {
                        voxel.floor = Some(Floor::Fire);
                        fire_count += 1;
                    }
                }
            }
        }
    }
    if fire_count > 0 {
        combat_log.push(format!(
            "A blazing inferno! {fire_count} tile(s) set ablaze!"
        ));
    }

    spawn_molotov_smoke(game_map, origin, turn_counter.0, radius);
}
