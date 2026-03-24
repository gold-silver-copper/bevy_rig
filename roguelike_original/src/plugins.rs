use std::collections::HashSet;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::components::{
    ACTION_COST, AiMemory, AiPersonality, AiState, BlocksMovement, Caliber, CameraFollow,
    CombatStats, Cursor, Energy, Faction, Health, Inventory, Item, ItemKind, Name, PatrolOrigin,
    PlayerControlled, Position, Renderable, Speed, Stamina, Viewshed,
};
use crate::events::{
    AiRangedAttackIntent, AttackIntent, DamageEvent, MeleeWideIntent, MolotovCastIntent,
    MoveIntent, PickupItemIntent, RangedAttackIntent, SpellCastIntent, ThrowItemIntent,
    UseItemIntent,
};
use crate::gamemap::GameMap;
use crate::grid_vec::GridVec;
use crate::noise::value_noise;
use crate::resources::{
    BloodMap, CameraPosition, Collectibles, CombatLog, CursorPosition, DynamicRng, ExtraWorldTicks,
    GameMapResource, GameState, InputState, KillCount, MapSeed, RestartRequested, SoundEvents,
    SpatialIndex, SpectatingAfterDeath, SpellParticles, StartupExplosionPending, TurnCounter,
    TurnState,
    WindowedKeyRepeat,
};
use crate::systems::spawn::MONSTER_TEMPLATES;
use crate::systems::{
    ai, camera, combat, input, inventory, movement, projectile, render, spatial_index, spawn,
    spell, turn, visibility,
};
use crate::typedefs::{RatColor, SPAWN_X, SPAWN_Y};

// ─────────────────────────── System Sets ───────────────────────────

/// Top-level system ordering for the roguelike.
///
/// ```text
///   Index → Action → Consequence → Render
/// ```
///
/// - **Index**: rebuild the spatial index (runs unconditionally).
/// - **Action**: process player and NPC actions (movement, combat).
/// - **Consequence**: recalculate derived state (FOV, camera).
/// - **Render**: draw the frame (runs unconditionally).
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum RoguelikeSet {
    /// Rebuild spatial index so later systems have O(1) position lookups.
    Index,
    /// Process intents — movement, combat, AI.
    Action,
    /// Recalculate derived state — visibility, camera follow.
    Consequence,
    /// Draw the frame to the terminal.
    Render,
}

// ─────────────────────────── Plugins ───────────────────────────────
//
// Following the Bevy plugin best practice (see `examples/app/plugin.rs`),
// the game is split into domain-specific plugins. Each plugin owns the
// systems for a single responsibility. `RoguelikePlugin` composes them
// all together so `main.rs` stays minimal.

/// Top-level Bevy plugin. Registers resources, messages, states, startup
/// systems, and adds all domain sub-plugins.
pub struct RoguelikePlugin;

const OFFSCREEN_REINFORCEMENT_INTERVAL: u32 = 100;
const OFFSCREEN_REINFORCEMENT_MIN_DISTANCE: i32 = 28;
const OFFSCREEN_REINFORCEMENT_MAX_DISTANCE: i32 = 52;

impl Plugin for RoguelikePlugin {
    fn build(&self, app: &mut App) {
        // Use an existing MapSeed if the user inserted one, otherwise use a
        // time-based seed for a unique experience each playthrough.
        let seed = app
            .world()
            .get_resource::<MapSeed>()
            .map(|s| s.0)
            .unwrap_or_else(|| fresh_seed(42));

        let game_map = GameMap::new(800, 560, seed);
        // Compute actual player spawn position so camera+cursor start centered on it.
        // Must match the spawn logic in do_spawn_player().
        let center = GridVec::new(game_map.width / 2, game_map.height / 2);
        let player_spawn = game_map
            .find_jail_interior(center, 48)
            .or_else(|| game_map.find_building_interior(center, 40))
            .or_else(|| game_map.find_spawnable_near(center, 20))
            .unwrap_or(GridVec::new(SPAWN_X, SPAWN_Y));

        if !app.is_plugin_added::<bevy::state::app::StatesPlugin>() {
            app.add_plugins(bevy::state::app::StatesPlugin);
        }

        app
            // ── Messages ──
            .add_message::<MoveIntent>()
            .add_message::<AttackIntent>()
            .add_message::<DamageEvent>()
            .add_message::<SpellCastIntent>()
            .add_message::<UseItemIntent>()
            .add_message::<PickupItemIntent>()
            .add_message::<RangedAttackIntent>()
            .add_message::<MeleeWideIntent>()
            .add_message::<AiRangedAttackIntent>()
            .add_message::<ThrowItemIntent>()
            .add_message::<MolotovCastIntent>()
            // ── Resources ──
            .insert_resource(MapSeed(seed))
            .insert_resource(GameMapResource(game_map))
            .insert_resource(CameraPosition(player_spawn))
            .init_resource::<SpatialIndex>()
            .init_resource::<CombatLog>()
            .init_resource::<TurnCounter>()
            .init_resource::<KillCount>()
            .init_resource::<SpellParticles>()
            .init_resource::<InputState>()
            .init_resource::<RestartRequested>()
            .init_resource::<StartupExplosionPending>()
            .init_resource::<WindowedKeyRepeat>()
            .insert_resource(CursorPosition::at(player_spawn))
            .init_resource::<Collectibles>()
            .init_resource::<ExtraWorldTicks>()
            .init_resource::<SoundEvents>()
            .init_resource::<BloodMap>()
            .init_resource::<SpectatingAfterDeath>()
            .init_resource::<crate::resources::DeathFade>()
            .init_resource::<DynamicRng>()
            .init_resource::<crate::resources::GodMode>()
            .init_resource::<crate::resources::PropHealth>()
            // ── States ──
            .init_state::<GameState>()
            .add_sub_state::<TurnState>()
            // ── Startup ──
            .add_systems(Startup, (spawn_player, spawn_monsters).chain())
            // ── System-set ordering ──
            .configure_sets(
                Update,
                (
                    RoguelikeSet::Index,
                    RoguelikeSet::Action,
                    RoguelikeSet::Consequence,
                    RoguelikeSet::Render,
                )
                    .chain(),
            )
            // ── Domain sub-plugins ──
            .add_plugins((InputPlugin, ActionPlugin, WorldPlugin, ViewPlugin));
    }
}

/// Handles player input and game restart logic.
/// Runs in `PreUpdate` so intents are emitted before `Update` processes them.
struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, input::input_system)
            .add_systems(PreUpdate, play_startup_explosion_smoke.after(input::input_system))
            .add_systems(PreUpdate, restart_system);
    }
}

/// Processes gameplay actions: movement, combat, inventory, spells, and
/// projectiles. All systems are gated on `GameState::Playing`.
struct ActionPlugin;

impl Plugin for ActionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            spatial_index::spatial_index_system.in_set(RoguelikeSet::Index),
        )
        .add_systems(
            Update,
            (
                movement::movement_system,
                inventory::pickup_system,
                inventory::auto_pickup_system,
                inventory::use_item_system,
                inventory::throw_system,
                inventory::reload_system,
                spell::spell_system,
                spell::molotov_system,
                combat::ranged_attack_system,
                combat::melee_wide_system,
                combat::combat_system,
                projectile::projectile_system,
            )
                .chain()
                .in_set(RoguelikeSet::Action)
                .run_if(in_state(GameState::Playing)),
        )
        .add_systems(
            Update,
            (
                spell::explosive_projectile_system,
                combat::apply_damage_system,
                combat::death_system,
                movement::victory_check_system,
            )
                .chain()
                .after(projectile::projectile_system)
                .in_set(RoguelikeSet::Action)
                .run_if(in_state(GameState::Playing)),
        );
    }
}

/// Manages turn state transitions, fire spreading, and AI behaviour.
/// Turn-phase systems are gated on their respective sub-states.
struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            turn::end_player_turn_system
                .after(RoguelikeSet::Consequence)
                .run_if(in_state(TurnState::PlayerTurn)),
        )
        .add_systems(
            Update,
            (
                ai::energy_accumulate_system,
                ai::ai_system,
                combat::ai_ranged_attack_system,
                turn::fire_system,
                spawn_offscreen_reinforcements_system,
            )
                .chain()
                .after(RoguelikeSet::Consequence)
                .run_if(in_state(TurnState::WorldTurn)),
        )
        .add_systems(
            Update,
            turn::end_world_turn_system
                .after(spawn_offscreen_reinforcements_system)
                .run_if(in_state(TurnState::WorldTurn)),
        );
    }
}

/// Visibility, camera tracking, and terminal rendering.
/// Consequence systems are gated on `GameState::Playing`; render systems
/// run unconditionally to show overlays (pause, death, victory screens).
struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (visibility::visibility_system, camera::camera_follow_system)
                .chain()
                .in_set(RoguelikeSet::Consequence)
                .run_if(in_state(GameState::Playing)),
        )
        .add_systems(
            Update,
            (render::cursor_blink_system, render::particle_tick_system)
                .in_set(RoguelikeSet::Render),
        )
        .add_systems(
            Update,
            render::draw_system
                .in_set(RoguelikeSet::Render)
                .after(render::cursor_blink_system)
                .after(render::particle_tick_system),
        );
    }
}

/// Spawns the player entity with all required ECS components.
fn spawn_player(
    mut commands: Commands,
    mut map: ResMut<GameMapResource>,
    mut collectibles: ResMut<Collectibles>,
    mut startup_explosion_pending: ResMut<StartupExplosionPending>,
) {
    let (caliber, _spawn_pos, breach_tiles) = do_spawn_player(&mut commands, &mut map);
    *collectibles = Collectibles::for_starting_caliber(caliber);
    startup_explosion_pending.breach_tiles = breach_tiles;
}

/// Spawns monsters on passable tiles using deterministic noise placement.
/// Monsters are spawned near the Enemy Stronghold area, using the map seed for
/// deterministic placement.
fn spawn_monsters(mut commands: Commands, map: Res<GameMapResource>, seed: Res<MapSeed>) {
    do_spawn_monsters(&mut commands, &map, seed.0);
}

/// Helper: spawns the player entity.
/// Returns the caliber of the player's starting gun so collectibles can be
/// initialized with matching ammo.
fn do_spawn_player(
    commands: &mut Commands,
    map: &mut GameMapResource,
) -> (Caliber, GridVec, Vec<GridVec>) {
    // Spawn the player inside a building near the center of the map.
    let center = GridVec::new(map.0.width / 2, map.0.height / 2);
    let spawn_pos = map
        .0
        .find_jail_interior(center, 48)
        .or_else(|| map.0.find_building_interior(center, 40))
        .or_else(|| map.0.find_spawnable_near(center, 20))
        .unwrap_or(center);

    // Clear props directly around the player spawn so they don't start blocked.
    let breach_tiles = crate::gamemap::clear_around_collect_walls(&mut map.0, spawn_pos, 3);

    // Use spawn position as seed for deterministic randomization.
    let rng_seed = (spawn_pos.x.wrapping_mul(7919) ^ spawn_pos.y.wrapping_mul(6271)) as u32;

    // Random starting gun from the full weapon pool
    let weapon_pool: &[(&str, Caliber, i32, &str)] = &[
        ("Colt Sheriff", Caliber::Cal36, 5, "p"),
        ("Colt Army", Caliber::Cal44, 6, "p"),
        ("Colt Pocket", Caliber::Cal31, 5, "p"),
        ("Remington New Model Army", Caliber::Cal44, 6, "p"),
        ("Starr 1858 DA", Caliber::Cal44, 6, "p"),
        ("Savage 1856", Caliber::Cal36, 6, "p"),
        ("Adams Revolver", Caliber::Cal44, 5, "p"),
        ("Hawken Rifle", Caliber::Cal50, 1, "r"),
        ("Springfield 1842", Caliber::Cal69, 1, "r"),
        ("Springfield 1855", Caliber::Cal58, 1, "r"),
        ("Enfield 1853", Caliber::Cal577, 1, "r"),
    ];
    let gun_idx = (rng_seed as usize) % weapon_pool.len();
    let (gun_name, caliber, capacity, symbol) = weapon_pool[gun_idx];

    let gun = commands
        .spawn((
            Item,
            Name(gun_name.into()),
            Renderable {
                symbol: symbol.into(),
                fg: RatColor::Rgb(160, 150, 140),
                bg: RatColor::Black,
            },
            ItemKind::Gun {
                loaded: capacity,
                capacity,
                caliber,
                attack: caliber.damage(),
                name: gun_name.into(),
                blunt_damage: 5,
            },
        ))
        .id();

    // Random alcohol
    let alcohol_idx = (rng_seed.wrapping_mul(31) as usize) % 6;
    let alcohol = match alcohol_idx {
        0 => commands
            .spawn((
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
            ))
            .id(),
        1 => commands
            .spawn((
                Item,
                Name("Beer".into()),
                Renderable {
                    symbol: "b".into(),
                    fg: RatColor::Rgb(200, 180, 80),
                    bg: RatColor::Black,
                },
                ItemKind::Beer {
                    heal: 5,
                    blunt_damage: 3,
                },
            ))
            .id(),
        2 => commands
            .spawn((
                Item,
                Name("Ale".into()),
                Renderable {
                    symbol: "a".into(),
                    fg: RatColor::Rgb(190, 150, 70),
                    bg: RatColor::Black,
                },
                ItemKind::Ale {
                    heal: 7,
                    blunt_damage: 3,
                },
            ))
            .id(),
        3 => commands
            .spawn((
                Item,
                Name("Stout".into()),
                Renderable {
                    symbol: "s".into(),
                    fg: RatColor::Rgb(80, 50, 30),
                    bg: RatColor::Black,
                },
                ItemKind::Stout {
                    heal: 8,
                    blunt_damage: 4,
                },
            ))
            .id(),
        4 => commands
            .spawn((
                Item,
                Name("Wine".into()),
                Renderable {
                    symbol: "w".into(),
                    fg: RatColor::Rgb(140, 40, 60),
                    bg: RatColor::Black,
                },
                ItemKind::Wine {
                    heal: 6,
                    blunt_damage: 3,
                },
            ))
            .id(),
        _ => commands
            .spawn((
                Item,
                Name("Rum".into()),
                Renderable {
                    symbol: "r".into(),
                    fg: RatColor::Rgb(160, 100, 40),
                    bg: RatColor::Black,
                },
                ItemKind::Rum {
                    heal: 12,
                    blunt_damage: 4,
                },
            ))
            .id(),
    };

    // Randomize between knife, tomahawk, or nothing
    let melee_idx = (rng_seed.wrapping_mul(47) as usize) % 3;
    let melee_item = match melee_idx {
        0 => Some(
            commands
                .spawn((
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
                ))
                .id(),
        ),
        1 => Some(
            commands
                .spawn((
                    Item,
                    Name("Tomahawk".into()),
                    Renderable {
                        symbol: "t".into(),
                        fg: RatColor::Rgb(160, 120, 80),
                        bg: RatColor::Black,
                    },
                    ItemKind::Tomahawk {
                        attack: 5,
                        blunt_damage: 7,
                    },
                ))
                .id(),
        ),
        _ => None, // nothing
    };

    // Randomize between dynamite and molotov
    let explosive_idx = (rng_seed.wrapping_mul(59) as usize) % 2;
    let explosive = if explosive_idx == 0 {
        commands
            .spawn((
                Item,
                Name("Dynamite".into()),
                Renderable {
                    symbol: "d".into(),
                    fg: RatColor::Rgb(200, 50, 50),
                    bg: RatColor::Black,
                },
                ItemKind::Grenade {
                    damage: 8,
                    radius: 3,
                    blunt_damage: 5,
                },
            ))
            .id()
    } else {
        commands
            .spawn((
                Item,
                Name("Molotov".into()),
                Renderable {
                    symbol: "m".into(),
                    fg: RatColor::Rgb(255, 100, 0),
                    bg: RatColor::Black,
                },
                ItemKind::Molotov {
                    damage: 6,
                    radius: 4,
                    blunt_damage: 4,
                },
            ))
            .id()
    };

    let mut items = vec![gun, alcohol];
    if let Some(m) = melee_item {
        items.push(m);
    }
    items.push(explosive);

    commands
        .spawn((
            Position {
                x: spawn_pos.x,
                y: spawn_pos.y,
            },
            PlayerControlled,
            Name("Rogue".into()),
            Renderable {
                symbol: "@".into(),
                fg: RatColor::White,
                bg: RatColor::Black,
            },
            CameraFollow,
            BlocksMovement,
            Faction::Civilians,
            Health {
                current: 100,
                max: 100,
            },
            Stamina {
                current: 100,
                max: 100,
            },
            CombatStats { attack: 5 },
            Speed(ACTION_COST),
            Energy(0),
        ))
        .insert((
            Inventory { items },
            Viewshed {
                range: 40,
                visible_tiles: HashSet::new(),
                revealed_tiles: HashSet::new(),
                dirty: true,
            },
            AiState::Idle,
            Cursor {
                pos: GridVec::new(spawn_pos.x, spawn_pos.y) + GridVec::NORTH,
            },
            PatrolOrigin(GridVec::new(spawn_pos.x, spawn_pos.y)),
            AiMemory::default(),
            AiPersonality {
                aggression: 0.5,
                courage: 1.0,
            },
        ));

    // debug_assert that the ammo caliber matches the gun's caliber at spawn.
    debug_assert!(
        weapon_pool[gun_idx].1 == caliber,
        "Starting ammo caliber must match the player's starting gun caliber"
    );

    (caliber, spawn_pos, breach_tiles)
}

fn add_start_breach_effect(spell_particles: &mut SpellParticles, breach_tiles: &[GridVec]) {
    for &origin in breach_tiles {
        for &(dx, dy, life, delay, vx, vy, is_smoke) in &[
            (0, 0, 7, 0, 0, 0, false),
            (-1, 0, 6, 0, -1, 0, false),
            (1, 0, 6, 0, 1, 0, false),
            (0, 1, 6, 1, 0, 1, false),
            (0, -1, 5, 1, 0, -1, false),
            (-1, 1, 22, 0, -1, 1, true),
            (0, 1, 28, 0, 0, 1, true),
            (1, 1, 22, 0, 1, 1, true),
            (-2, 1, 26, 1, -1, 1, true),
            (2, 1, 26, 1, 1, 1, true),
            (-1, 2, 30, 2, -1, 1, true),
            (0, 2, 34, 3, 0, 1, true),
            (1, 2, 30, 2, 1, 1, true),
            (-2, 2, 24, 4, -1, 1, true),
            (2, 2, 24, 4, 1, 1, true),
            (-1, 0, 20, 1, -1, 0, true),
            (1, 0, 20, 1, 1, 0, true),
        ] {
            spell_particles
                .particles
                .push((origin + GridVec::new(dx, dy), life, delay, is_smoke, vx, vy));
        }
    }
}

fn play_startup_explosion_smoke(
    input_state: Res<InputState>,
    mut pending: ResMut<StartupExplosionPending>,
    mut spell_particles: ResMut<SpellParticles>,
) {
    if pending.breach_tiles.is_empty() || input_state.welcome_visible {
        return;
    }

    add_start_breach_effect(&mut spell_particles, &pending.breach_tiles);
    pending.breach_tiles.clear();
}

fn resolve_player_spawn(map: &GameMap) -> GridVec {
    let center = GridVec::new(map.width / 2, map.height / 2);
    map.find_jail_interior(center, 48)
        .or_else(|| map.find_building_interior(center, 40))
        .or_else(|| map.find_spawnable_near(center, 20))
        .unwrap_or(center)
}

fn spawn_gang_group(
    commands: &mut Commands,
    map: &GameMapResource,
    player_spawn: GridVec,
    anchor: GridVec,
    templates: &[usize],
    group_size: i32,
    cluster_radius: i32,
    min_spacing: i32,
    max_spawn_dist_sq: i32,
    visible_tiles: Option<&HashSet<GridVec>>,
) -> i32 {
    let min_spawn_dist_sq = crate::systems::spawn::PLAYER_SAFE_SPAWN_RADIUS
        * crate::systems::spawn::PLAYER_SAFE_SPAWN_RADIUS;
    let mut spawned = 0;
    let mut occupied = Vec::new();

    for ring in 0..=cluster_radius {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if ring > 0 && dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                if spawned >= group_size {
                    return spawned;
                }
                let pos = anchor + GridVec::new(dx, dy);
                let local_dist = pos.chebyshev_distance(anchor);
                if local_dist > cluster_radius || local_dist < 2 {
                    continue;
                }
                if pos.distance_squared(player_spawn) < min_spawn_dist_sq
                    || pos.distance_squared(player_spawn) > max_spawn_dist_sq
                {
                    continue;
                }
                if visible_tiles.is_some_and(|tiles| tiles.contains(&pos)) {
                    continue;
                }
                if !map.0.is_spawnable(&pos) {
                    continue;
                }
                if occupied
                    .iter()
                    .any(|other: &GridVec| other.chebyshev_distance(pos) < min_spacing)
                {
                    continue;
                }

                let template_idx = templates[(spawned as usize) % templates.len()];
                let template = &MONSTER_TEMPLATES[template_idx];
                spawn::spawn_monster(commands, template, pos.x, pos.y, 0, 0);
                occupied.push(pos);
                spawned += 1;
            }
        }
    }

    spawned
}

fn spawn_offscreen_reinforcement_group(
    commands: &mut Commands,
    map: &GameMapResource,
    seed: u64,
    turn: u32,
    player_pos: GridVec,
    camera_pos: GridVec,
    visible_tiles: Option<&HashSet<GridVec>>,
) -> bool {
    let faction_configs: &[(&[usize], u64)] = &[
        (&[2, 3], 10), // Apache
        (&[0], 20),    // Vaqueros
        (&[4, 5], 40), // Police
        (&[6, 7], 50), // Outlaws
        (&[8, 9], 60), // Lawmen
    ];
    let choice =
        ((value_noise(turn as i32, 17, seed.wrapping_add(0xA11CE)) * faction_configs.len() as f64)
            as usize)
            % faction_configs.len();
    let (templates, offset) = faction_configs[choice];

    for attempt in 0..48 {
        let key = seed
            .wrapping_add(turn as u64 * 4099)
            .wrapping_add(offset * 100)
            .wrapping_add(attempt as u64);
        let dx = ((value_noise(turn as i32, attempt, key) - 0.5)
            * 2.0
            * OFFSCREEN_REINFORCEMENT_MAX_DISTANCE as f64) as i32;
        let dy = ((value_noise(attempt, turn as i32, key.wrapping_add(1)) - 0.5)
            * 2.0
            * OFFSCREEN_REINFORCEMENT_MAX_DISTANCE as f64) as i32;
        let anchor = GridVec::new(
            (camera_pos.x + dx).clamp(10, map.0.width - 10),
            (camera_pos.y + dy).clamp(10, map.0.height - 10),
        );
        let camera_dist = anchor.chebyshev_distance(camera_pos);
        if !(OFFSCREEN_REINFORCEMENT_MIN_DISTANCE..=OFFSCREEN_REINFORCEMENT_MAX_DISTANCE)
            .contains(&camera_dist)
        {
            continue;
        }
        if anchor.chebyshev_distance(player_pos) < OFFSCREEN_REINFORCEMENT_MIN_DISTANCE - 4 {
            continue;
        }
        if visible_tiles.is_some_and(|tiles| tiles.contains(&anchor)) {
            continue;
        }

        let spawned = spawn_gang_group(
            commands,
            map,
            player_pos,
            anchor,
            templates,
            7,
            18,
            3,
            OFFSCREEN_REINFORCEMENT_MAX_DISTANCE * OFFSCREEN_REINFORCEMENT_MAX_DISTANCE,
            visible_tiles,
        );
        if spawned >= 4 {
            return true;
        }
    }

    false
}

/// Helper: spawns NPCs in faction groups distributed across the full map.
/// Enemy gangs spawn all over roads and inside buildings within a 150-tile
/// radius of the player, with many more groups than before.
fn do_spawn_monsters(commands: &mut Commands, map: &GameMapResource, seed: u64) {
    let group_seed = seed.wrapping_add(54321);
    let player_spawn = resolve_player_spawn(&map.0);
    let min_spawn_dist_sq = crate::systems::spawn::PLAYER_SAFE_SPAWN_RADIUS
        * crate::systems::spawn::PLAYER_SAFE_SPAWN_RADIUS;
    // keep a generous clear zone around player spawn
    let max_spawn_dist_sq: i32 = 150 * 150; // non-civilian NPCs spawn within 150 tiles of player

    // ── Faction gang groups across the full map ──────────────────
    // Groups nearer the player spawn slightly smaller and more diffuse so the
    // opening has pressure without becoming a dogpile. Farther out, groups
    // continue to appear but taper down in size instead of disappearing.
    let gang_seed = group_seed.wrapping_add(99999);
    let center_x = map.0.width / 2;
    let center_y = map.0.height / 2;
    let spawn_radius: i32 = 150;

    // (template indices, faction offset seed, number of groups, is_civilian)
    let gang_configs: &[(&[usize], u64, i32, bool)] = &[
        (&[2, 3], 10, 24, false), // Apache
        (&[0], 20, 24, false),    // Vaqueros
        (&[1], 30, 12, true),     // Civilians (exempt from radius)
        (&[4, 5], 40, 20, false), // Police
        (&[6, 7], 50, 28, false), // Outlaws
        (&[8, 9], 60, 20, false), // Lawmen
    ];

    for &(templates, offset, num_groups, is_civilian_group) in gang_configs {
        for group_idx in 0..num_groups {
            // Pick a random anchor point across the full map
            let gs = gang_seed.wrapping_add(offset * 1000 + group_idx as u64);
            let anchor_x = center_x
                + ((value_noise(group_idx, offset as i32, gs) - 0.5) * 2.0 * spawn_radius as f64)
                    as i32;
            let anchor_y = center_y
                + ((value_noise(offset as i32, group_idx, gs.wrapping_add(1)) - 0.5)
                    * 2.0
                    * spawn_radius as f64) as i32;
            let anchor_x = anchor_x.clamp(10, map.0.width - 10);
            let anchor_y = anchor_y.clamp(10, map.0.height - 10);

            let anchor = GridVec::new(anchor_x, anchor_y);
            let distance = anchor.chebyshev_distance(player_spawn);
            let (min_size, max_size, cluster_radius, min_spacing) = if distance < 55 {
                (6, 9, 22, 3)
            } else if distance < 80 {
                (7, 10, 20, 3)
            } else if distance < 120 {
                (5, 8, 18, 3)
            } else {
                (3, 6, 16, 2)
            };
            let group_size = min_size
                + (value_noise(anchor_x, anchor_y, gs.wrapping_add(2))
                    * (max_size - min_size + 1) as f64) as i32;
            let mut spawned = 0;
            let mut occupied = Vec::new();
            for dy in -cluster_radius..=cluster_radius {
                for dx in -cluster_radius..=cluster_radius {
                    if spawned >= group_size {
                        break;
                    }
                    let pos = GridVec::new(anchor_x + dx, anchor_y + dy);
                    let local_dist = pos.chebyshev_distance(anchor);
                    if local_dist > cluster_radius || local_dist < 3 {
                        continue;
                    }
                    if pos.distance_squared(player_spawn) < min_spawn_dist_sq {
                        continue;
                    }
                    // Non-civilian NPCs must spawn within 150 tiles of the player.
                    if !is_civilian_group && pos.distance_squared(player_spawn) > max_spawn_dist_sq
                    {
                        continue;
                    }
                    if !map.0.is_spawnable(&pos) {
                        continue;
                    }
                    let tile_noise = value_noise(pos.x, pos.y, gs.wrapping_add(3333));
                    if tile_noise > 0.62 {
                        continue;
                    }
                    if occupied
                        .iter()
                        .any(|other: &GridVec| other.chebyshev_distance(pos) < min_spacing)
                    {
                        continue;
                    }
                    let template_idx = templates[(spawned as usize) % templates.len()];
                    let template = &MONSTER_TEMPLATES[template_idx];
                    spawn::spawn_monster(commands, template, pos.x, pos.y, 0, 0);
                    occupied.push(pos);
                    spawned += 1;
                }
                if spawned >= group_size {
                    break;
                }
            }
        }
    }
}

fn spawn_offscreen_reinforcements_system(
    mut commands: Commands,
    turn_counter: Res<TurnCounter>,
    map: Res<GameMapResource>,
    seed: Res<MapSeed>,
    camera: Res<CameraPosition>,
    player_query: Query<(&Position, &Viewshed), With<PlayerControlled>>,
) {
    if turn_counter.0 == 0 || !turn_counter.0.is_multiple_of(OFFSCREEN_REINFORCEMENT_INTERVAL) {
        return;
    }

    let Ok((player_pos, viewshed)) = player_query.single() else {
        return;
    };

    let _ = spawn_offscreen_reinforcement_group(
        &mut commands,
        &map,
        seed.0,
        turn_counter.0,
        player_pos.as_grid_vec(),
        camera.0,
        Some(&viewshed.visible_tiles),
    );
}

/// Bundles all mutable resources needed by `restart_system` into a single
/// `SystemParam`, following the Bevy best practice of using `#[derive(SystemParam)]`
/// to stay under the 16-parameter system limit (see `examples/ecs/ecs_guide.rs`).
#[derive(SystemParam)]
struct RestartResources<'w> {
    combat_log: ResMut<'w, CombatLog>,
    kill_count: ResMut<'w, KillCount>,
    turn_counter: ResMut<'w, TurnCounter>,
    spell_particles: ResMut<'w, SpellParticles>,
    startup_explosion_pending: ResMut<'w, StartupExplosionPending>,
    input_state: ResMut<'w, InputState>,
    next_game_state: ResMut<'w, NextState<GameState>>,
    seed: ResMut<'w, MapSeed>,
    game_map: ResMut<'w, GameMapResource>,
    camera: ResMut<'w, CameraPosition>,
    cursor: ResMut<'w, CursorPosition>,
    collectibles: ResMut<'w, Collectibles>,
    extra_ticks: ResMut<'w, ExtraWorldTicks>,
    blood_map: ResMut<'w, BloodMap>,
    spectating: ResMut<'w, SpectatingAfterDeath>,
    dynamic_rng: ResMut<'w, DynamicRng>,
    key_repeat: ResMut<'w, WindowedKeyRepeat>,
    god_mode: ResMut<'w, crate::resources::GodMode>,
    prop_health: ResMut<'w, crate::resources::PropHealth>,
    death_fade: ResMut<'w, crate::resources::DeathFade>,
}

/// System that handles game restart by despawning all entities and re-spawning.
fn restart_system(
    mut commands: Commands,
    mut restart: ResMut<RestartRequested>,
    game_entities: Query<Entity, With<Position>>,
    mut res: RestartResources,
) {
    if !restart.0 {
        return;
    }
    restart.0 = false;

    for entity in &game_entities {
        commands.entity(entity).despawn();
    }

    // Generate a new seed each restart so the map is different every time.
    let new_seed = fresh_seed(res.seed.0.wrapping_add(1));
    res.seed.0 = new_seed;

    res.combat_log.clear();
    res.kill_count.0 = 0;
    res.turn_counter.0 = 0;
    res.spell_particles.particles.clear();
    *res.input_state = InputState::default();
    *res.game_map = GameMapResource(GameMap::new(800, 560, res.seed.0));
    let center = GridVec::new(res.game_map.0.width / 2, res.game_map.0.height / 2);
    let player_spawn = res
        .game_map
        .0
        .find_jail_interior(center, 48)
        .or_else(|| res.game_map.0.find_building_interior(center, 40))
        .or_else(|| res.game_map.0.find_spawnable_near(center, 20))
        .unwrap_or(GridVec::new(SPAWN_X, SPAWN_Y));
    res.camera.0 = player_spawn;
    *res.cursor = CursorPosition::at(player_spawn);
    *res.collectibles = Collectibles::default();
    res.extra_ticks.0 = 0;
    res.blood_map.stains.clear();
    res.spectating.0 = false;
    res.god_mode.0 = false;
    res.dynamic_rng.reset();
    res.prop_health.hp.clear();
    res.death_fade.frames = 0;
    res.next_game_state.set(GameState::Playing);
    res.startup_explosion_pending.breach_tiles.clear();

    #[cfg(feature = "windowed")]
    res.key_repeat.held.clear();

    let (caliber, _spawn_pos, breach_tiles) = do_spawn_player(&mut commands, &mut res.game_map);
    *res.collectibles = Collectibles::for_starting_caliber(caliber);
    res.startup_explosion_pending.breach_tiles = breach_tiles;
    do_spawn_monsters(&mut commands, &res.game_map, res.seed.0);
}

#[cfg(target_arch = "wasm32")]
fn fresh_seed(fallback: u64) -> u64 {
    let millis = js_sys::Date::now();
    if millis.is_finite() {
        (millis * 1_000.0) as u64
    } else {
        fallback
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fresh_seed(fallback: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(fallback)
}
