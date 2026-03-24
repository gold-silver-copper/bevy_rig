use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use bevy::prelude::*;

use crate::components::{
    AiMemory, AiPersonality, AiPursuitBoost, AiState, AiTarget, AimingStyle, BlocksMovement,
    Cursor, Energy, Faction, Health, Inventory, Item, ItemKind, PatrolOrigin, PlayerControlled,
    Position, SPELL_STAMINA_COST, Speed, Stamina, Viewshed,
};
use crate::events::{
    AttackIntent, MolotovCastIntent, MoveIntent, PickupItemIntent, RangedAttackIntent,
    SpellCastIntent, UseItemIntent,
};
use crate::grid_vec::GridVec;
use crate::resources::{GameMapResource, SpatialIndex, SpellParticles, TurnCounter};
use crate::typeenums::{Floor, Props};

mod cost {
    pub const BASE: i32 = 10;
    pub const FIRE: i32 = 70;
    pub const NEAR_FIRE: i32 = 28;
    pub const NEAR_CACTUS: i32 = 10;
    pub const SAND_CLOUD: i32 = 16;
    pub const COVER_PER_WALL: i32 = 2;
    pub const EXPOSED: i32 = 4;
}

const MAX_A_STAR_NODES: usize = 640;
const MAX_DIJKSTRA_NODES: usize = 640;

const MEMORY_DURATION: u32 = 56;
const TARGET_LOCK_TIMEOUT: u32 = 40;
const PURSUIT_AWARENESS_BOOST: i32 = 18;
const PURSUIT_BOOST_DECAY_TURNS: u32 = 4;
const PROXIMITY_OVERRIDE_RANGE: i32 = 10;
const ALLY_SHARE_RANGE: i32 = 32;
const PATROL_RADIUS: i32 = 18;
const LOOK_AROUND_BASE_INTERVAL: u32 = 40;
const LOOK_AROUND_DICE_RANGE: u32 = 5;
const MAX_SEARCH_SWEEPS: u8 = 3;
const FULL_ROTATION_STEPS: u8 = GridVec::DIRECTIONS_4.len() as u8;
const ROTATION_MOVE_INTERVAL: u8 = 2;
const STUCK_FLANK_TURNS: u8 = 2;
const ITEM_INTEREST_RANGE: i32 = 8;
const FLEE_RECOVERY_FRACTION: f64 = 0.75;
const EXPLOSIVE_MIN_RANGE: i32 = 4;
const EXPLOSIVE_MAX_RANGE: i32 = 8;
const BOW_MAX_RANGE: i32 = 40;
const SCAN_CURSOR_DISTANCE: i32 = 6;
const UNSEEN_CURSOR_ADJUST_INTERVAL: u32 = 40;
const HEAL_CHECK_INTERVAL: u32 = 5;
const HEAL_CHECK_PERIOD: u32 = 20;
const REVERSE_STEP_PENALTY: i32 = 160;

const HASH_KNUTH: u64 = 2_654_435_761;
const HASH_MIX_A: u64 = 0xff51afd7ed558ccd;
const HASH_MIX_B: u64 = 0xc4ceb9fe1a85ec53;

#[derive(Clone, Copy)]
struct Threat {
    entity: Entity,
    pos: GridVec,
    distance: i32,
    score: i32,
}

#[derive(Clone, Copy)]
struct TrackedTarget {
    entity: Entity,
    pos: GridVec,
    visible: bool,
    locked: bool,
}

#[derive(Clone, Copy)]
struct WeaponProfile {
    min_range: i32,
    preferred_range: i32,
    max_range: i32,
}

#[derive(Clone, Copy)]
struct GunChoice {
    entity: Entity,
    profile: WeaponProfile,
}

#[derive(Clone, Copy)]
struct BowChoice {
    attack: i32,
    profile: WeaponProfile,
}

#[derive(Clone, Copy)]
struct ExplosiveChoice {
    index: usize,
    damage: i32,
    radius: i32,
}

#[derive(Clone, Copy)]
struct HealChoice {
    index: usize,
    amount: i32,
}

#[derive(Default)]
struct InventoryTactics {
    loaded_gun: Option<GunChoice>,
    reloadable_gun: Option<GunChoice>,
    bow: Option<BowChoice>,
    heal: Option<HealChoice>,
    grenade: Option<ExplosiveChoice>,
    molotov: Option<ExplosiveChoice>,
}

#[inline]
fn hash64(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(HASH_MIX_A);
    x ^= x >> 33;
    x = x.wrapping_mul(HASH_MIX_B);
    x ^= x >> 33;
    x
}

#[inline]
fn turn_hash(entity: Entity, turn: u32, salt: u64) -> u64 {
    hash64(entity.to_bits() ^ ((turn as u64) << 32) ^ salt)
}

#[inline]
fn look_interval(entity: Entity) -> u32 {
    LOOK_AROUND_BASE_INTERVAL + (entity.to_bits() as u32 % LOOK_AROUND_DICE_RANGE.max(1))
}

#[inline]
fn scan_rotation_due(entity: Entity, turn: u32) -> bool {
    turn % UNSEEN_CURSOR_ADJUST_INTERVAL == entity.to_bits() as u32 % UNSEEN_CURSOR_ADJUST_INTERVAL
}

#[inline]
fn cardinal_step_toward(delta: GridVec) -> GridVec {
    if delta.x.abs() >= delta.y.abs() {
        if delta.x != 0 {
            GridVec::new(delta.x.signum(), 0)
        } else {
            GridVec::new(0, delta.y.signum())
        }
    } else {
        GridVec::new(0, delta.y.signum())
    }
}

#[inline]
fn has_visibility(viewshed: Option<&Viewshed>, pos: GridVec) -> bool {
    viewshed.is_some_and(|vs| vs.visible_tiles.contains(&pos))
}

#[inline]
fn is_walkable_tile(game_map: &GameMapResource, pos: GridVec) -> bool {
    game_map.0.is_passable(&pos) && !game_map.0.is_water(&pos)
}

fn base_tile_cost(pos: GridVec, game_map: &GameMapResource) -> Option<i32> {
    if !is_walkable_tile(game_map, pos) {
        return None;
    }

    let mut value = cost::BASE;

    if let Some(voxel) = game_map.0.get_voxel_at(&pos) {
        match voxel.floor {
            Some(Floor::Fire) => value += cost::FIRE,
            Some(Floor::SandCloud) => value += cost::SAND_CLOUD,
            _ => {}
        }
    }

    let mut wall_count = 0;
    let mut near_fire = false;
    let mut near_cactus = false;

    for neighbor in pos.all_neighbors() {
        if let Some(voxel) = game_map.0.get_voxel_at(&neighbor) {
            if matches!(voxel.floor, Some(Floor::Fire)) {
                near_fire = true;
            }
            if matches!(voxel.props, Some(Props::Cactus)) {
                near_cactus = true;
            }
            if voxel
                .props
                .as_ref()
                .is_some_and(|prop| prop.blocks_movement())
            {
                wall_count += 1;
            }
        }
    }

    if near_fire {
        value += cost::NEAR_FIRE;
    }
    if near_cactus {
        value += cost::NEAR_CACTUS;
    }

    if wall_count > 0 {
        value -= (wall_count * cost::COVER_PER_WALL).min(value - 1);
    } else {
        value += cost::EXPOSED;
    }

    Some(value)
}

fn chase_tile_cost(pos: GridVec, game_map: &GameMapResource) -> Option<i32> {
    if !is_walkable_tile(game_map, pos) {
        return None;
    }

    let mut value = cost::BASE;

    if let Some(voxel) = game_map.0.get_voxel_at(&pos) {
        match voxel.floor {
            Some(Floor::Fire) => value += cost::FIRE,
            Some(Floor::SandCloud) => value += cost::SAND_CLOUD,
            _ => {}
        }
    }

    for neighbor in pos.all_neighbors() {
        if let Some(voxel) = game_map.0.get_voxel_at(&neighbor) {
            if matches!(voxel.floor, Some(Floor::Fire)) {
                value += cost::NEAR_FIRE;
            }
            if matches!(voxel.props, Some(Props::Cactus)) {
                value += cost::NEAR_CACTUS;
            }
        }
    }

    Some(value)
}

fn tile_cost_for_ai(
    pos: GridVec,
    self_entity: Entity,
    my_faction: Option<Faction>,
    game_map: &GameMapResource,
    spatial: &SpatialIndex,
    blockers: &Query<(), With<BlocksMovement>>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> Option<i32> {
    if spatial
        .entities_at(&pos)
        .iter()
        .any(|&entity| entity != self_entity && blockers.contains(entity))
    {
        return None;
    }
    let mut value = base_tile_cost(pos, game_map)?;
    value += ally_cohesion_cost(pos, self_entity, my_faction, npc_overview);
    Some(value)
}

fn tile_cost_for_ai_chase(
    pos: GridVec,
    self_entity: Entity,
    my_faction: Option<Faction>,
    game_map: &GameMapResource,
    spatial: &SpatialIndex,
    blockers: &Query<(), With<BlocksMovement>>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> Option<i32> {
    if spatial
        .entities_at(&pos)
        .iter()
        .any(|&entity| entity != self_entity && blockers.contains(entity))
    {
        return None;
    }
    let mut value = chase_tile_cost(pos, game_map)?;
    value += ally_cohesion_cost(pos, self_entity, my_faction, npc_overview);
    Some(value)
}

fn ally_cohesion_cost(
    pos: GridVec,
    self_entity: Entity,
    my_faction: Option<Faction>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> i32 {
    let Some(my_faction) = my_faction else {
        return 0;
    };

    let mut close_allies = 0;
    let mut support_allies = 0;

    for (other_entity, other_pos, other_faction, _) in npc_overview {
        if other_entity == self_entity || other_faction.copied() != Some(my_faction) {
            continue;
        }

        let dist = pos.chebyshev_distance(other_pos.as_grid_vec());
        if dist == 0 {
            continue;
        }
        if dist == 1 {
            close_allies += 1;
        } else if dist <= 6 {
            support_allies += 1;
        }
    }

    let support_bonus = (support_allies * 5).min(18);
    let crowding_penalty = close_allies * 7;
    crowding_penalty - support_bonus
}

fn a_star_first_step(
    start: GridVec,
    goal: GridVec,
    cost_fn: impl Fn(GridVec) -> Option<i32>,
) -> Option<GridVec> {
    if start == goal {
        return None;
    }

    if start.manhattan_distance(goal) == 1 && cost_fn(goal).is_some() {
        return Some(goal - start);
    }

    let mut open: BinaryHeap<Reverse<(i32, i32, GridVec)>> = BinaryHeap::new();
    let mut came_from: HashMap<GridVec, GridVec> = HashMap::new();
    let mut g_score: HashMap<GridVec, i32> = HashMap::new();
    let mut closed: HashSet<GridVec> = HashSet::new();

    let start_h = start.manhattan_distance(goal) * cost::BASE;
    g_score.insert(start, 0);
    open.push(Reverse((start_h, start_h, start)));

    let mut explored = 0usize;

    while let Some(Reverse((_, _, current))) = open.pop() {
        if current == goal {
            let mut step = current;
            while let Some(&prev) = came_from.get(&step) {
                if prev == start {
                    return Some(step - start);
                }
                step = prev;
            }
            return None;
        }

        if !closed.insert(current) {
            continue;
        }

        explored += 1;
        if explored >= MAX_A_STAR_NODES {
            break;
        }

        let current_g = g_score[&current];

        for dir in GridVec::DIRECTIONS_4 {
            let neighbor = current + dir;
            if closed.contains(&neighbor) {
                continue;
            }

            let edge_cost = if neighbor == goal {
                cost::BASE
            } else {
                match cost_fn(neighbor) {
                    Some(cost) => cost,
                    None => continue,
                }
            };

            let candidate_g = current_g + edge_cost;
            if candidate_g < *g_score.get(&neighbor).unwrap_or(&i32::MAX) {
                came_from.insert(neighbor, current);
                g_score.insert(neighbor, candidate_g);
                let h = neighbor.manhattan_distance(goal) * cost::BASE;
                open.push(Reverse((candidate_g + h, h, neighbor)));
            }
        }
    }

    None
}

pub fn a_star_first_step_pub(
    start: GridVec,
    goal: GridVec,
    is_walkable: impl Fn(GridVec) -> bool,
) -> Option<GridVec> {
    a_star_first_step(start, goal, |pos| {
        if is_walkable(pos) {
            Some(cost::BASE)
        } else {
            None
        }
    })
}

fn dijkstra_map(
    sources: &[GridVec],
    cost_fn: impl Fn(GridVec) -> Option<i32>,
) -> HashMap<GridVec, i32> {
    let mut dist = HashMap::with_capacity(MAX_DIJKSTRA_NODES);
    let mut open: BinaryHeap<Reverse<(i32, GridVec)>> = BinaryHeap::new();

    for &source in sources {
        dist.insert(source, 0);
        open.push(Reverse((0, source)));
    }

    let mut explored = 0usize;

    while let Some(Reverse((current_dist, current))) = open.pop() {
        if current_dist > *dist.get(&current).unwrap_or(&i32::MAX) {
            continue;
        }

        explored += 1;
        if explored >= MAX_DIJKSTRA_NODES {
            break;
        }

        for dir in GridVec::DIRECTIONS_4 {
            let neighbor = current + dir;
            let edge_cost = match cost_fn(neighbor) {
                Some(cost) => cost,
                None => continue,
            };
            let next_dist = current_dist + edge_cost;
            if next_dist < *dist.get(&neighbor).unwrap_or(&i32::MAX) {
                dist.insert(neighbor, next_dist);
                open.push(Reverse((next_dist, neighbor)));
            }
        }
    }

    dist
}

fn gun_profile(name: &str, capacity: i32) -> WeaponProfile {
    let is_long_gun = capacity == 1
        || name.contains("Rifle")
        || name.contains("Springfield")
        || name.contains("Enfield")
        || name.contains("Hawken");

    if is_long_gun {
        WeaponProfile {
            min_range: 3,
            preferred_range: 14,
            max_range: 28,
        }
    } else {
        WeaponProfile {
            min_range: 2,
            preferred_range: 8,
            max_range: 18,
        }
    }
}

fn adjust_profile_for_style(
    profile: WeaponProfile,
    style: Option<AimingStyle>,
    personality: AiPersonality,
) -> WeaponProfile {
    let aggression_bias = if personality.aggression >= 0.85 { 1 } else { 0 };

    match style {
        Some(AimingStyle::CarefulAim) => WeaponProfile {
            min_range: (profile.min_range + 1).max(2),
            preferred_range: profile.preferred_range + 1,
            max_range: profile.max_range,
        },
        Some(AimingStyle::SnapShot) => WeaponProfile {
            min_range: (profile.min_range - 1).max(1),
            preferred_range: (profile.preferred_range - (1 + aggression_bias)).max(2),
            max_range: profile.max_range + 1,
        },
        Some(AimingStyle::Suppression) => WeaponProfile {
            min_range: (profile.min_range - 1).max(1),
            preferred_range: (profile.preferred_range - aggression_bias).max(2),
            max_range: profile.max_range + 2,
        },
        None => profile,
    }
}

fn analyze_inventory(
    inventory: Option<&Inventory>,
    item_kinds: &mut Query<&mut ItemKind>,
) -> InventoryTactics {
    let Some(inventory) = inventory else {
        return InventoryTactics::default();
    };

    let mut tactics = InventoryTactics::default();

    for (index, &item_entity) in inventory.items.iter().enumerate() {
        let Ok(kind) = item_kinds.get_mut(item_entity) else {
            continue;
        };

        match &*kind {
            ItemKind::Gun {
                loaded,
                capacity,
                name,
                ..
            } => {
                let choice = GunChoice {
                    entity: item_entity,
                    profile: gun_profile(name, *capacity),
                };
                if *loaded > 0 && tactics.loaded_gun.is_none() {
                    tactics.loaded_gun = Some(choice);
                }
                if *loaded < *capacity && tactics.reloadable_gun.is_none() {
                    tactics.reloadable_gun = Some(choice);
                }
            }
            ItemKind::Bow { attack, .. } => {
                if tactics.bow.is_none() {
                    tactics.bow = Some(BowChoice {
                        attack: *attack,
                        profile: WeaponProfile {
                            min_range: 4,
                            preferred_range: 16,
                            max_range: BOW_MAX_RANGE,
                        },
                    });
                }
            }
            ItemKind::Grenade { damage, radius, .. } => {
                if tactics.grenade.is_none() {
                    tactics.grenade = Some(ExplosiveChoice {
                        index,
                        damage: *damage,
                        radius: *radius,
                    });
                }
            }
            ItemKind::Molotov { damage, radius, .. } => {
                if tactics.molotov.is_none() {
                    tactics.molotov = Some(ExplosiveChoice {
                        index,
                        damage: *damage,
                        radius: *radius,
                    });
                }
            }
            ItemKind::Whiskey { heal, .. }
            | ItemKind::Beer { heal, .. }
            | ItemKind::Ale { heal, .. }
            | ItemKind::Stout { heal, .. }
            | ItemKind::Wine { heal, .. }
            | ItemKind::Rum { heal, .. } => {
                let current_best = tactics.heal.map(|choice| choice.amount).unwrap_or(i32::MIN);
                if *heal > current_best {
                    tactics.heal = Some(HealChoice {
                        index,
                        amount: *heal,
                    });
                }
            }
            _ => {}
        }
    }

    tactics
}

fn should_flee(health: Option<&Health>, has_heal: bool, personality: AiPersonality) -> bool {
    let Some(health) = health else {
        return false;
    };
    if has_heal {
        return false;
    }

    let base_threshold = 10;
    let resolve_modifier =
        ((personality.courage * 6.0) + (personality.aggression * 4.0)).round() as i32;
    let hp_threshold = (base_threshold - resolve_modifier).clamp(2, 12);
    let panic_fraction = (0.16 - personality.aggression * 0.08).clamp(0.06, 0.16);
    health.current <= hp_threshold || health.fraction() <= panic_fraction
}

fn should_attempt_heal(entity: Entity, turn: u32, health: &Health) -> bool {
    if health.fraction() >= 0.5 {
        return false;
    }

    let check_offset = entity.to_bits() as u32 % HEAL_CHECK_INTERVAL;
    if turn % HEAL_CHECK_INTERVAL != check_offset {
        return false;
    }

    let check_index = turn / HEAL_CHECK_INTERVAL;
    let trigger_offset = ((entity.to_bits() >> 8) as u32) % HEAL_CHECK_PERIOD;
    check_index % HEAL_CHECK_PERIOD == trigger_offset
}

fn effective_awareness_range(
    base_range: i32,
    pursuit_boost: Option<&AiPursuitBoost>,
    personality: AiPersonality,
    current_turn: u32,
) -> i32 {
    let bonus = pursuit_boost
        .map(|boost| {
            let unseen_turns = current_turn.saturating_sub(boost.last_spotted_turn);
            let decay = (unseen_turns / PURSUIT_BOOST_DECAY_TURNS) as i32;
            (boost.extra_range - decay).max(0)
        })
        .unwrap_or(0);

    base_range + bonus + (personality.aggression * 10.0).round() as i32
}

fn threat_score(distance: i32) -> i32 {
    (32 - distance).max(0) * 3
}

fn select_priority_target(hostiles: &[Threat]) -> Option<Threat> {
    hostiles
        .iter()
        .min_by_key(|threat| (threat.distance, Reverse(threat.score)))
        .copied()
}

fn choose_aiming_style(entity: Entity, turn: u32, personality: AiPersonality) -> AimingStyle {
    if personality.aggression >= 0.65 {
        return AimingStyle::SnapShot;
    }

    match turn_hash(entity, turn, HASH_KNUTH) % 4 {
        0 => AimingStyle::CarefulAim,
        1 => AimingStyle::Suppression,
        _ => AimingStyle::SnapShot,
    }
}

fn ensure_aiming_style(
    commands: &mut Commands,
    entity: Entity,
    current: Option<AimingStyle>,
    turn: u32,
    personality: AiPersonality,
    target_changed: bool,
) -> AimingStyle {
    if target_changed || current.is_none() {
        let style = choose_aiming_style(entity, turn, personality);
        commands.entity(entity).insert(style);
        style
    } else {
        current.unwrap_or_else(|| choose_aiming_style(entity, turn, personality))
    }
}

fn set_cursor(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    pos: GridVec,
) {
    if let Some(cursor) = cursor.as_deref_mut() {
        cursor.pos = pos;
    } else {
        commands.entity(entity).insert(Cursor { pos });
    }
}

fn canonical_cursor_pos(origin: GridVec, desired: GridVec) -> GridVec {
    if desired == origin {
        origin + GridVec::NORTH
    } else {
        desired
    }
}

fn ensure_cursor_origin(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    origin: GridVec,
    reset: bool,
) -> GridVec {
    let default_pos = origin + GridVec::NORTH;
    if let Some(cursor_ref) = cursor.as_deref_mut() {
        let canonical = canonical_cursor_pos(origin, cursor_ref.pos);
        if reset || cursor_ref.pos != canonical {
            cursor_ref.pos = canonical;
        }
        canonical
    } else {
        set_cursor(commands, entity, cursor, default_pos);
        default_pos
    }
}

fn clear_engagement(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<AiTarget>();
    commands.entity(entity).remove::<AimingStyle>();
    commands.entity(entity).remove::<Cursor>();
    commands.entity(entity).remove::<AiPursuitBoost>();
}

fn orient_cursor(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    origin: GridVec,
    desired: GridVec,
    viewshed: &mut Option<Mut<Viewshed>>,
) {
    let next = canonical_cursor_pos(origin, desired);
    let next_basis = (next - origin).king_step();
    let changed = if let Some(cursor_ref) = cursor.as_deref() {
        cursor_ref.pos != next && (cursor_ref.pos - origin).king_step() != next_basis
    } else {
        true
    };
    set_cursor(commands, entity, cursor, next);
    if changed {
        if let Some(viewshed) = viewshed.as_deref_mut() {
            viewshed.dirty = true;
        }
    }
}

fn orient_cursor_to_move(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    origin: GridVec,
    move_dir: GridVec,
    viewshed: &mut Option<Mut<Viewshed>>,
) {
    if move_dir.is_zero() {
        return;
    }
    orient_cursor(
        commands,
        entity,
        cursor,
        origin,
        origin + move_dir * SCAN_CURSOR_DISTANCE,
        viewshed,
    );
}

fn scan_cursor_target(origin: GridVec, interest: Option<GridVec>) -> GridVec {
    let dir = interest
        .map(|point| cardinal_step_toward(point - origin))
        .filter(|dir| !dir.is_zero())
        .unwrap_or(GridVec::NORTH);
    origin + dir * SCAN_CURSOR_DISTANCE
}

fn moving_scan_cursor_target(origin: GridVec, interest: GridVec, entity: Entity, turn: u32) -> GridVec {
    let forward = cardinal_step_toward(interest - origin);
    if forward.is_zero() {
        return origin + GridVec::NORTH * SCAN_CURSOR_DISTANCE;
    }

    let phase = ((turn / UNSEEN_CURSOR_ADJUST_INTERVAL.max(1)) as u64 + entity.to_bits()) % 3;
    let dir = match phase {
        0 => forward.rotate_90_ccw(),
        1 => forward,
        _ => forward.rotate_90_cw(),
    };
    origin + dir * SCAN_CURSOR_DISTANCE
}

fn step_cursor_toward(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    origin: GridVec,
    desired_dir: GridVec,
    viewshed: &mut Option<Mut<Viewshed>>,
) {
    let current_index = cursor_direction_index(origin, cursor);
    let desired_index = GridVec::DIRECTIONS_4
        .iter()
        .position(|&candidate| candidate == desired_dir)
        .unwrap_or(current_index);
    let next_index = if current_index == desired_index {
        current_index
    } else {
        let clockwise = (desired_index + GridVec::DIRECTIONS_4.len() - current_index)
            % GridVec::DIRECTIONS_4.len();
        let counter_clockwise = (current_index + GridVec::DIRECTIONS_4.len() - desired_index)
            % GridVec::DIRECTIONS_4.len();
        if clockwise <= counter_clockwise {
            (current_index + 1) % GridVec::DIRECTIONS_4.len()
        } else {
            (current_index + GridVec::DIRECTIONS_4.len() - 1) % GridVec::DIRECTIONS_4.len()
        }
    };
    orient_cursor(
        commands,
        entity,
        cursor,
        origin,
        origin + GridVec::DIRECTIONS_4[next_index] * SCAN_CURSOR_DISTANCE,
        viewshed,
    );
}

fn track_unseen_objective_cursor(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    origin: GridVec,
    interest: GridVec,
    turn: u32,
    viewshed: &mut Option<Mut<Viewshed>>,
) {
    let desired = moving_scan_cursor_target(origin, interest, entity, turn);
    let desired_dir = cardinal_step_toward(desired - origin);
    if desired_dir.is_zero() {
        return;
    }
    if !scan_rotation_due(entity, turn) {
        return;
    }
    step_cursor_toward(commands, entity, cursor, origin, desired_dir, viewshed);
}

fn prime_scan_cursor(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    origin: GridVec,
    interest: Option<GridVec>,
    viewshed: &mut Option<Mut<Viewshed>>,
) {
    orient_cursor(
        commands,
        entity,
        cursor,
        origin,
        scan_cursor_target(origin, interest),
        viewshed,
    );
}

fn cursor_direction_index(origin: GridVec, cursor: &Option<Mut<Cursor>>) -> usize {
    cursor
        .as_ref()
        .map(|cursor| cardinal_step_toward(cursor.pos - origin))
        .and_then(|dir| GridVec::DIRECTIONS_4.iter().position(|&candidate| candidate == dir))
        .unwrap_or(0)
}

fn rotate_cursor(
    commands: &mut Commands,
    entity: Entity,
    cursor: &mut Option<Mut<Cursor>>,
    origin: GridVec,
    viewshed: &mut Option<Mut<Viewshed>>,
    remaining_steps: &mut u8,
) {
    let current_index = cursor_direction_index(origin, cursor);
    let next_dir = GridVec::DIRECTIONS_4[(current_index + 1) % GridVec::DIRECTIONS_4.len()];
    orient_cursor(commands, entity, cursor, origin, origin + next_dir, viewshed);
    *remaining_steps = remaining_steps.saturating_sub(1);
}

fn rotation_probe_step(
    my_pos: GridVec,
    entity: Entity,
    my_faction: Option<Faction>,
    cursor: &Option<Mut<Cursor>>,
    remaining_rotation_steps: u8,
    game_map: &GameMapResource,
    spatial: &SpatialIndex,
    blockers: &Query<(), With<BlocksMovement>>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> Option<GridVec> {
    if remaining_rotation_steps == 0 || remaining_rotation_steps % ROTATION_MOVE_INTERVAL != 0 {
        return None;
    }

    let index = cursor_direction_index(my_pos, cursor);
    let directions = [
        GridVec::DIRECTIONS_4[index],
        GridVec::DIRECTIONS_4[(index + 1) % GridVec::DIRECTIONS_4.len()],
        GridVec::DIRECTIONS_4
            [(index + GridVec::DIRECTIONS_4.len() - 1) % GridVec::DIRECTIONS_4.len()],
    ];

    directions.into_iter().find(|dir| {
        !dir.is_zero()
            && {
                let next = my_pos + *dir;
                tile_cost_for_ai(
                    next,
                    entity,
                    my_faction,
                    game_map,
                    spatial,
                    blockers,
                    npc_overview,
                )
                .is_some()
            }
    })
}

fn has_clear_line_of_sight(
    origin: GridVec,
    target: GridVec,
    game_map: &GameMapResource,
    sand_clouds: &HashSet<GridVec>,
) -> bool {
    if origin == target {
        return true;
    }

    let line = origin.bresenham_line(target);
    for &tile in line.iter().skip(1) {
        if tile == target {
            return true;
        }
        if sand_clouds.contains(&tile) {
            return false;
        }
        match game_map.0.get_voxel_at(&tile) {
            Some(voxel) => {
                if matches!(voxel.floor, Some(Floor::SandCloud)) {
                    return false;
                }
                if voxel
                    .props
                    .as_ref()
                    .is_some_and(|prop| prop.blocks_vision())
                {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

fn has_friendly_in_path(
    origin: GridVec,
    target: GridVec,
    my_faction: Option<Faction>,
    self_entity: Entity,
    spatial: &SpatialIndex,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> bool {
    let Some(my_faction) = my_faction else {
        return false;
    };

    let line = origin.bresenham_line(target);
    for &tile in line.iter().skip(1) {
        if tile == target {
            return false;
        }
        for &entity in spatial.entities_at(&tile) {
            if entity == self_entity {
                continue;
            }
            if let Ok((_, _, Some(faction), _)) = npc_overview.get(entity) {
                if !factions_are_hostile(my_faction, *faction) {
                    return true;
                }
            }
        }
    }

    false
}

fn cursor_aligned_with_target(aim_delta: GridVec, target_delta: GridVec) -> bool {
    if aim_delta.is_zero() || target_delta.is_zero() {
        return false;
    }

    let dot = (aim_delta.x * target_delta.x + aim_delta.y * target_delta.y) as f64;
    if dot <= 0.0 {
        return false;
    }

    let aim_len = ((aim_delta.x * aim_delta.x + aim_delta.y * aim_delta.y) as f64).sqrt();
    let target_len = ((target_delta.x * target_delta.x + target_delta.y * target_delta.y) as f64).sqrt();
    let cos_theta = dot / (aim_len * target_len);
    cos_theta >= 0.8
}

#[inline]
fn reverse_step_penalty(memory: Option<&AiMemory>, current_turn: u32, dir: GridVec) -> i32 {
    let Some(memory) = memory else {
        return 0;
    };
    if dir.is_zero() {
        return 0;
    }
    if memory.last_move_turn + 1 == current_turn
        && memory
            .last_move_dir
            .is_some_and(|last| last == GridVec::new(-dir.x, -dir.y))
    {
        REVERSE_STEP_PENALTY
    } else {
        0
    }
}

fn reset_search_state(memory: &mut AiMemory) {
    memory.search_attempts = 0;
    memory.search_rotation_steps = 0;
}

fn record_move(memory: &mut Option<Mut<AiMemory>>, dir: GridVec, current_turn: u32) {
    if let Some(memory) = memory.as_deref_mut() {
        memory.last_move_dir = Some(dir);
        memory.last_move_turn = current_turn;
    }
}

fn count_hostiles_near(
    center: GridVec,
    radius: i32,
    my_faction: Option<Faction>,
    player_info: Option<(Entity, GridVec, Option<Faction>)>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> usize {
    let mut count = 0usize;

    if let Some((_, player_pos, player_faction)) = player_info {
        let hostile_player = match (my_faction, player_faction) {
            (Some(my_faction), Some(player_faction)) => {
                factions_are_hostile(my_faction, player_faction)
            }
            _ => true,
        };
        if hostile_player && center.chebyshev_distance(player_pos) <= radius {
            count += 1;
        }
    }

    if let Some(my_faction) = my_faction {
        for (_, position, faction, _) in npc_overview {
            if faction.is_some_and(|other| factions_are_hostile(my_faction, *other))
                && center.chebyshev_distance(position.as_grid_vec()) <= radius
            {
                count += 1;
            }
        }
    }

    count
}

fn blast_hits_friendlies(
    center: GridVec,
    radius: i32,
    my_pos: GridVec,
    my_faction: Option<Faction>,
    self_entity: Entity,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> bool {
    if my_pos.chebyshev_distance(center) <= radius + 1 {
        return true;
    }

    let Some(my_faction) = my_faction else {
        return false;
    };

    for (entity, position, faction, _) in npc_overview {
        if entity == self_entity {
            continue;
        }
        if faction.is_some_and(|other| !factions_are_hostile(my_faction, *other))
            && center.chebyshev_distance(position.as_grid_vec()) <= radius
        {
            return true;
        }
    }

    false
}

fn nearest_fire_threat(my_pos: GridVec, game_map: &GameMapResource) -> Option<GridVec> {
    let mut fire_sum = GridVec::ZERO;
    let mut fire_count = 0i32;

    if game_map
        .0
        .get_voxel_at(&my_pos)
        .is_some_and(|voxel| matches!(voxel.floor, Some(Floor::Fire)))
    {
        fire_sum += my_pos;
        fire_count += 1;
    }

    for neighbor in my_pos.all_neighbors() {
        if game_map
            .0
            .get_voxel_at(&neighbor)
            .is_some_and(|voxel| matches!(voxel.floor, Some(Floor::Fire)))
        {
            fire_sum += neighbor;
            fire_count += 1;
        }
    }

    if fire_count == 0 {
        None
    } else {
        Some(GridVec::new(
            fire_sum.x / fire_count,
            fire_sum.y / fire_count,
        ))
    }
}

fn flee_direction(
    my_pos: GridVec,
    threat_pos: GridVec,
    entity: Entity,
    my_faction: Option<Faction>,
    game_map: &GameMapResource,
    spatial: &SpatialIndex,
    blockers: &Query<(), With<BlocksMovement>>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
    memory: Option<&AiMemory>,
    current_turn: u32,
) -> Option<GridVec> {
    let threat_map = dijkstra_map(&[threat_pos], |pos| {
        tile_cost_for_ai(pos, entity, my_faction, game_map, spatial, blockers, npc_overview)
    });

    let my_distance = *threat_map.get(&my_pos).unwrap_or(&0);
    let my_direct_distance = my_pos.chebyshev_distance(threat_pos);
    let mut best_direction = None;
    let mut best_score = i32::MIN;

    for dir in GridVec::DIRECTIONS_4 {
        let neighbor = my_pos + dir;
        let tile_cost = match tile_cost_for_ai(
            neighbor,
            entity,
            my_faction,
            game_map,
            spatial,
            blockers,
            npc_overview,
        ) {
            Some(cost) => cost,
            None => continue,
        };
        let neighbor_distance = *threat_map
            .get(&neighbor)
            .unwrap_or(&(my_distance + cost::BASE * 4));
        let direct_distance = neighbor.chebyshev_distance(threat_pos);
        let score = neighbor_distance * 2 - tile_cost
            + direct_distance * 20
            + if direct_distance > my_direct_distance {
                200
            } else {
                0
            }
            - reverse_step_penalty(memory, current_turn, dir);
        if score > best_score {
            best_score = score;
            best_direction = Some(dir);
        }
    }

    best_direction
}

fn retreat_direction(
    my_pos: GridVec,
    threat_pos: GridVec,
    entity: Entity,
    my_faction: Option<Faction>,
    game_map: &GameMapResource,
    spatial: &SpatialIndex,
    blockers: &Query<(), With<BlocksMovement>>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
    sand_clouds: &HashSet<GridVec>,
    preserve_los: bool,
    memory: Option<&AiMemory>,
    current_turn: u32,
) -> Option<GridVec> {
    let mut best_direction = None;
    let mut best_score = i32::MIN;

    for dir in GridVec::DIRECTIONS_4 {
        let candidate = my_pos + dir;
        let tile_cost = match tile_cost_for_ai(
            candidate,
            entity,
            my_faction,
            game_map,
            spatial,
            blockers,
            npc_overview,
        ) {
            Some(cost) => cost,
            None => continue,
        };
        if preserve_los && !has_clear_line_of_sight(candidate, threat_pos, game_map, sand_clouds) {
            continue;
        }

        let distance_score = candidate.chebyshev_distance(threat_pos) * 20;
        let score = distance_score - tile_cost - reverse_step_penalty(memory, current_turn, dir);
        if score > best_score {
            best_score = score;
            best_direction = Some(dir);
        }
    }

    best_direction
}

fn first_step_toward(
    start: GridVec,
    goal: GridVec,
    entity: Entity,
    my_faction: Option<Faction>,
    game_map: &GameMapResource,
    spatial: &SpatialIndex,
    blockers: &Query<(), With<BlocksMovement>>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
    aggressive: bool,
    memory: Option<&AiMemory>,
    current_turn: u32,
) -> Option<GridVec> {
    let mut best = None;
    let mut best_score = i32::MIN;

    for dir in GridVec::DIRECTIONS_4 {
        let next = start + dir;
        let tile_cost = if aggressive {
            tile_cost_for_ai_chase(
                next,
                entity,
                my_faction,
                game_map,
                spatial,
                blockers,
                npc_overview,
            )
        } else {
            tile_cost_for_ai(
                next,
                entity,
                my_faction,
                game_map,
                spatial,
                blockers,
                npc_overview,
            )
        };
        let Some(tile_cost) = tile_cost else {
            continue;
        };
        let distance_score = 240 - next.chebyshev_distance(goal) * 24;
        let direct_bias = if dir == cardinal_step_toward(goal - start) {
            28
        } else {
            0
        };
        let score = distance_score + direct_bias - tile_cost - reverse_step_penalty(memory, current_turn, dir);
        if score > best_score {
            best_score = score;
            best = Some(dir);
        }
    }

    if let Some(step) = best {
        return Some(step);
    }

    let mut best_fallback = None;
    let mut best_fallback_score = i32::MIN;
    for dir in GridVec::DIRECTIONS_4 {
        let candidate_goal = goal + dir;
        let step = if aggressive {
            a_star_first_step(start, candidate_goal, |pos| {
                tile_cost_for_ai_chase(
                    pos,
                    entity,
                    my_faction,
                    game_map,
                    spatial,
                    blockers,
                    npc_overview,
                )
            })
        } else {
            a_star_first_step(start, candidate_goal, |pos| {
                tile_cost_for_ai(
                    pos,
                    entity,
                    my_faction,
                    game_map,
                    spatial,
                    blockers,
                    npc_overview,
                )
            })
        };
        let Some(step) = step else {
            continue;
        };
        let score = 200 - (start + step).chebyshev_distance(goal) * 20
            - reverse_step_penalty(memory, current_turn, step);
        if score > best_fallback_score {
            best_fallback_score = score;
            best_fallback = Some(step);
        }
    }
    best_fallback
}

fn flank_goal(entity: Entity, turn: u32, my_pos: GridVec, target_pos: GridVec) -> GridVec {
    let bearing = cardinal_step_toward(target_pos - my_pos);
    if bearing.is_zero() {
        return target_pos;
    }

    let left = target_pos + bearing.rotate_90_ccw() * 2;
    let right = target_pos + bearing.rotate_90_cw() * 2;
    if turn_hash(entity, turn, 0xF1A4_9A73) & 1 == 0 {
        left
    } else {
        right
    }
}

fn patrol_direction(
    entity: Entity,
    my_pos: GridVec,
    origin: GridVec,
    my_faction: Option<Faction>,
    game_map: &GameMapResource,
    spatial: &SpatialIndex,
    blockers: &Query<(), With<BlocksMovement>>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
    memory: Option<&AiMemory>,
    turn: u32,
) -> Option<GridVec> {
    let dist_from_origin = my_pos.chebyshev_distance(origin);
    if dist_from_origin > PATROL_RADIUS / 2 {
        return first_step_toward(
            my_pos,
            origin,
            entity,
            my_faction,
            game_map,
            spatial,
            blockers,
            npc_overview,
            false,
            memory,
            turn,
        );
    }

    let mut best_direction = None;
    let mut best_score = i32::MIN;

    for (index, dir) in GridVec::DIRECTIONS_4.iter().copied().enumerate() {
        let candidate = my_pos + dir;
        let tile_cost = match tile_cost_for_ai(
            candidate,
            entity,
            my_faction,
            game_map,
            spatial,
            blockers,
            npc_overview,
        ) {
            Some(cost) => cost,
            None => continue,
        };
        let radius = candidate.chebyshev_distance(origin);
        if radius > PATROL_RADIUS {
            continue;
        }
        let noise = (turn_hash(entity, turn, index as u64 + 0x9E37) % 13) as i32;
        let score =
            noise * 3 - tile_cost - radius * 2 - reverse_step_penalty(memory, turn, dir);
        if score > best_score {
            best_score = score;
            best_direction = Some(dir);
        }
    }

    best_direction
}

fn live_target_position(
    target: Entity,
    player_entity: Option<Entity>,
    player_info: Option<(Entity, GridVec, Option<Faction>)>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> Option<GridVec> {
    if player_info.is_some_and(|(entity, _, _)| entity == target) {
        return player_info.map(|(_, pos, _)| pos);
    }
    if player_entity == Some(target) {
        return None;
    }
    npc_overview
        .get(target)
        .ok()
        .map(|(_, position, _, _)| position.as_grid_vec())
}

fn memory_goal(memory: Option<&AiMemory>, current_turn: u32) -> Option<GridVec> {
    let memory = memory?;
    let goal = memory.last_known_pos?;
    if current_turn.saturating_sub(memory.last_seen_turn) <= MEMORY_DURATION {
        Some(goal)
    } else {
        None
    }
}

fn maybe_reload_gun(choice: Option<GunChoice>, item_kinds: &mut Query<&mut ItemKind>) -> bool {
    let Some(choice) = choice else {
        return false;
    };

    let Ok(mut kind) = item_kinds.get_mut(choice.entity) else {
        return false;
    };

    if let ItemKind::Gun {
        loaded, capacity, ..
    } = kind.as_mut()
    {
        if *loaded < *capacity {
            *loaded += 1;
            return true;
        }
    }

    false
}

fn collect_visible_hostiles(
    entity: Entity,
    my_pos: GridVec,
    my_faction: Option<Faction>,
    current_target: Option<Entity>,
    viewshed: Option<&Viewshed>,
    player_info: Option<(Entity, GridVec, Option<Faction>)>,
    npc_overview: &Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
) -> Vec<Threat> {
    let mut hostiles = Vec::new();

    if let Some((player_entity, player_pos, player_faction)) = player_info {
        let hostile_player = match (my_faction, player_faction) {
            (Some(my_faction), Some(player_faction)) => {
                factions_are_hostile(my_faction, player_faction)
            }
            _ => true,
        };
        if hostile_player {
            let distance = my_pos.chebyshev_distance(player_pos);
            if has_visibility(viewshed, player_pos) || distance <= PROXIMITY_OVERRIDE_RANGE {
                let mut score = threat_score(distance);
                if current_target == Some(player_entity) {
                    score += 18;
                }
                hostiles.push(Threat {
                    entity: player_entity,
                    pos: player_pos,
                    distance,
                    score,
                });
            }
        }
    }

    if let Some(my_faction) = my_faction {
        for (other_entity, other_position, other_faction, other_target) in npc_overview {
            if other_entity == entity {
                continue;
            }
            if !other_faction.is_some_and(|faction| factions_are_hostile(my_faction, *faction)) {
                continue;
            }

            let target_pos = other_position.as_grid_vec();
            let distance = my_pos.chebyshev_distance(target_pos);
            if !has_visibility(viewshed, target_pos) && distance > PROXIMITY_OVERRIDE_RANGE {
                continue;
            }

            let mut score = threat_score(distance);
            if current_target == Some(other_entity) {
                score += 18;
            }
            if other_target.is_some_and(|target| target.entity == entity) {
                score += 24;
            }

            hostiles.push(Threat {
                entity: other_entity,
                pos: target_pos,
                distance,
                score,
            });
        }
    }

    hostiles
}

pub fn factions_are_hostile(a: Faction, b: Faction) -> bool {
    a != b
}

pub fn ai_system(
    mut commands: Commands,
    mut ai_query: Query<
        (
            (
                Entity,
                &Position,
                &mut AiState,
                Option<&mut Viewshed>,
                &mut Energy,
                Option<&Faction>,
                Option<&PatrolOrigin>,
            ),
            (
                Option<&mut Inventory>,
                Option<&mut Health>,
                Option<&mut Stamina>,
                Option<&mut AiMemory>,
                Option<&AiPersonality>,
                Option<&AiTarget>,
                Option<&AimingStyle>,
                Option<&mut Cursor>,
            ),
            Option<&AiPursuitBoost>,
        ),
        Without<PlayerControlled>,
    >,
    player_query: Query<(Entity, &Position, &Health, Option<&Faction>), With<PlayerControlled>>,
    npc_overview: Query<
        (Entity, &Position, Option<&Faction>, Option<&AiTarget>),
        Without<PlayerControlled>,
    >,
    floor_items: Query<(Entity, &Position), With<Item>>,
    (game_map, spatial, turn_counter, spell_particles): (
        Res<GameMapResource>,
        Res<SpatialIndex>,
        Res<TurnCounter>,
        Res<SpellParticles>,
    ),
    blockers: Query<(), With<BlocksMovement>>,
    mut item_kinds: Query<&mut ItemKind>,
    (mut move_intents, mut attack_intents, mut ranged_intents): (
        MessageWriter<MoveIntent>,
        MessageWriter<AttackIntent>,
        MessageWriter<RangedAttackIntent>,
    ),
    (mut spell_intents, mut molotov_intents, mut use_item_intents, mut pickup_intents): (
        MessageWriter<SpellCastIntent>,
        MessageWriter<MolotovCastIntent>,
        MessageWriter<UseItemIntent>,
        MessageWriter<PickupItemIntent>,
    ),
) {
    let player_data = player_query
        .single()
        .ok()
        .map(|(entity, position, health, faction)| {
            (
                entity,
                position.as_grid_vec(),
                !health.is_dead(),
                faction.copied(),
            )
        });
    let player_entity = player_data.map(|(entity, _, _, _)| entity);
    let player_info = player_data.and_then(|(entity, position, alive, faction)| {
        alive.then_some((entity, position, faction))
    });

    let sand_cloud_tiles: HashSet<GridVec> = spell_particles
        .particles
        .iter()
        .filter(|(_, life, delay, _, _, _)| *delay == 0 && *life > 0)
        .map(|(pos, _, _, _, _, _)| *pos)
        .collect();

    let current_turn = turn_counter.0;

    let mut faction_alerts: HashMap<Faction, Vec<GridVec>> = HashMap::new();
    for (
        (entity, _position, ai_state, _viewshed, _energy, faction, _origin),
        (_inventory, _health, _stamina, ai_memory, _personality, ai_target, _aiming_style, _cursor),
        _pursuit_boost,
    ) in &mut ai_query
    {
        let Some(faction) = faction.copied() else {
            continue;
        };
        if !matches!(*ai_state, AiState::Chasing | AiState::Fleeing) {
            continue;
        }

        let target_from_component = ai_target.and_then(|target| {
            if player_entity == Some(target.entity) && player_info.is_none() {
                None
            } else {
                Some(target.last_pos)
            }
        });
        let target_from_memory = memory_goal(ai_memory.as_deref(), current_turn);
        if let Some(alert_pos) = target_from_component.or(target_from_memory) {
            let _ = entity;
            faction_alerts.entry(faction).or_default().push(alert_pos);
        }
    }

    for (
            (
                entity,
                position,
                mut ai_state,
                mut viewshed,
                mut energy,
                faction,
                patrol_origin,
            ),
        (
            inventory,
            health,
            stamina,
            mut ai_memory,
            personality,
            ai_target,
            aiming_style,
            mut cursor,
        ),
        pursuit_boost,
    ) in &mut ai_query
    {
        if !energy.can_act() {
            continue;
        }

        let my_pos = position.as_grid_vec();
        let my_faction = faction.copied();
        let personality = personality.copied().unwrap_or_default();

        if let Some(memory) = ai_memory.as_deref_mut() {
            if memory.prev_pos == Some(my_pos) {
                memory.stationary_turns = memory.stationary_turns.saturating_add(1);
            } else {
                memory.stationary_turns = 0;
            }
            memory.prev_pos = Some(my_pos);
        }

        let inventory_view = inventory.as_deref();
        let tactics = analyze_inventory(inventory_view, &mut item_kinds);
        let has_heal = tactics.heal.is_some();

        if player_info.is_none()
            && player_entity
                .is_some_and(|player| ai_target.is_some_and(|target| target.entity == player))
        {
            if let Some(memory) = ai_memory.as_deref_mut() {
                memory.last_known_pos = None;
                memory.search_attempts = 0;
            }
            clear_engagement(&mut commands, entity);
        }

        let current_target_entity = ai_target.map(|target| target.entity);
        let visible_hostiles = collect_visible_hostiles(
            entity,
            my_pos,
            my_faction,
            current_target_entity,
            viewshed.as_deref(),
            player_info,
            &npc_overview,
        );
        let best_visible = select_priority_target(&visible_hostiles);
        let visible_threat_pos = best_visible.map(|threat| threat.pos);

        let viewshed_range = viewshed
            .as_deref()
            .map(|viewshed| viewshed.range as i32)
            .unwrap_or(8);
        let awareness =
            effective_awareness_range(viewshed_range * 2, pursuit_boost, personality, current_turn);

        let mut target = best_visible.map(|threat| TrackedTarget {
            entity: threat.entity,
            pos: threat.pos,
            visible: true,
            locked: ai_target
                .is_some_and(|current| current.entity == threat.entity && current.locked),
        });

        if target.is_none() {
            if let Some(current_target) = ai_target {
                let target_timeout = if current_target.locked {
                    TARGET_LOCK_TIMEOUT
                } else {
                    MEMORY_DURATION
                };
                let target_position = live_target_position(
                    current_target.entity,
                    player_entity,
                    player_info,
                    &npc_overview,
                );
                let still_valid = target_position.is_some()
                    && current_turn.saturating_sub(current_target.last_seen) <= target_timeout
                    && (current_target.locked
                        || my_pos.chebyshev_distance(current_target.last_pos) <= awareness);
                if still_valid {
                    target = Some(TrackedTarget {
                        entity: current_target.entity,
                        pos: current_target.last_pos,
                        visible: false,
                        locked: current_target.locked,
                    });
                } else {
                    commands.entity(entity).remove::<AiTarget>();
                    commands.entity(entity).remove::<AiPursuitBoost>();
                }
            }
        }

        let mut objective = target
            .map(|target| target.pos)
            .or_else(|| memory_goal(ai_memory.as_deref(), current_turn));

        if objective.is_none() {
            if let Some(my_faction) = my_faction {
                if let Some(alerts) = faction_alerts.get(&my_faction) {
                    if let Some(alert_pos) = alerts
                        .iter()
                        .copied()
                        .filter(|alert| my_pos.chebyshev_distance(*alert) <= ALLY_SHARE_RANGE)
                        .min_by_key(|alert| my_pos.chebyshev_distance(*alert))
                    {
                        objective = Some(alert_pos);
                        if let Some(memory) = ai_memory.as_deref_mut() {
                            memory.last_known_pos = Some(alert_pos);
                            memory.last_seen_turn = current_turn;
                            reset_search_state(memory);
                        }
                    }
                }
            }
        }

        let mut current_style = aiming_style.copied();

        if let Some(target) = target {
            let target_changed = current_target_entity != Some(target.entity);
            current_style = Some(ensure_aiming_style(
                &mut commands,
                entity,
                current_style,
                current_turn,
                personality,
                target_changed,
            ));
            let _ = ensure_cursor_origin(&mut commands, entity, &mut cursor, my_pos, target_changed);

            let last_seen = if target.visible {
                current_turn
            } else {
                ai_target
                    .map(|current| current.last_seen)
                    .unwrap_or(current_turn)
            };
            commands.entity(entity).insert(AiTarget {
                entity: target.entity,
                last_pos: target.pos,
                last_seen,
                locked: target.locked,
            });

            if let Some(memory) = ai_memory.as_deref_mut() {
                memory.last_known_pos = Some(target.pos);
                if target.visible {
                    memory.last_seen_turn = current_turn;
                    reset_search_state(memory);
                }
            }

            if target.visible {
                commands.entity(entity).insert(AiPursuitBoost {
                    extra_range: PURSUIT_AWARENESS_BOOST,
                    last_spotted_turn: current_turn,
                });
            }
        } else if let Some(goal) = objective {
            let _ = ensure_cursor_origin(&mut commands, entity, &mut cursor, my_pos, false);
            track_unseen_objective_cursor(
                &mut commands,
                entity,
                &mut cursor,
                my_pos,
                goal,
                current_turn,
                &mut viewshed,
            );
            commands.entity(entity).remove::<AiTarget>();
            commands.entity(entity).remove::<AiPursuitBoost>();
        } else {
            clear_engagement(&mut commands, entity);
        }

        if let Some(fire_center) = nearest_fire_threat(my_pos, &game_map) {
            if let Some(dir) =
                flee_direction(
                    my_pos,
                    fire_center,
                    entity,
                    my_faction,
                    &game_map,
                    &spatial,
                    &blockers,
                    &npc_overview,
                    ai_memory.as_deref(),
                    current_turn,
                )
            {
                move_intents.write(MoveIntent {
                    entity,
                    dx: dir.x,
                    dy: dir.y,
                });
                record_move(&mut ai_memory, dir, current_turn);
                orient_cursor(
                    &mut commands,
                    entity,
                    &mut cursor,
                    my_pos,
                    my_pos + dir,
                    &mut viewshed,
                );
                energy.spend_action();
                continue;
            }
        }

        let health_ref = health.as_deref();
        if let (Some(health), Some(heal)) = (health_ref, tactics.heal) {
            if should_attempt_heal(entity, current_turn, health) {
                let _ = heal.amount;
                use_item_intents.write(UseItemIntent {
                    user: entity,
                    item_index: heal.index,
                });
                energy.spend_action();
                continue;
            }
        }

        let should_retreat = should_flee(health_ref, has_heal, personality);
        let keep_fleeing = matches!(*ai_state, AiState::Fleeing)
            && objective.is_some()
            && health_ref.is_some_and(|health| health.fraction() < FLEE_RECOVERY_FRACTION);
        if (should_retreat || keep_fleeing) && objective.is_some() {
            *ai_state = AiState::Fleeing;
        } else if objective.is_some() {
            *ai_state = AiState::Chasing;
        } else if matches!(*ai_state, AiState::Idle) {
            *ai_state = AiState::Idle;
        } else {
            *ai_state = AiState::Patrolling;
        }

        if inventory_view.is_some_and(|inv| inv.items.len() < 9) && objective.is_none() {
            if spatial
                .entities_at(&my_pos)
                .iter()
                .any(|&item| floor_items.get(item).is_ok())
            {
                pickup_intents.write(PickupItemIntent { picker: entity });
                energy.spend_action();
                continue;
            }
        }

        match *ai_state {
            AiState::Fleeing => {
                if let Some(goal) = visible_threat_pos.or(objective) {
                    let direct_away = cardinal_step_toward(my_pos - goal);
                    if !direct_away.is_zero()
                        && tile_cost_for_ai(
                            my_pos + direct_away,
                            entity,
                            my_faction,
                            &game_map,
                            &spatial,
                            &blockers,
                            &npc_overview,
                        )
                        .is_some()
                    {
                        move_intents.write(MoveIntent {
                            entity,
                            dx: direct_away.x,
                            dy: direct_away.y,
                        });
                        record_move(&mut ai_memory, direct_away, current_turn);
                        orient_cursor_to_move(
                            &mut commands,
                            entity,
                            &mut cursor,
                            my_pos,
                            direct_away,
                            &mut viewshed,
                        );
                        energy.spend_action();
                        continue;
                    }
                    if let Some(dir) =
                        flee_direction(
                            my_pos,
                            goal,
                            entity,
                            my_faction,
                            &game_map,
                            &spatial,
                            &blockers,
                            &npc_overview,
                            ai_memory.as_deref(),
                            current_turn,
                        )
                    {
                        move_intents.write(MoveIntent {
                            entity,
                            dx: dir.x,
                            dy: dir.y,
                        });
                        record_move(&mut ai_memory, dir, current_turn);
                        orient_cursor_to_move(
                            &mut commands,
                            entity,
                            &mut cursor,
                            my_pos,
                            dir,
                            &mut viewshed,
                        );
                        energy.spend_action();
                        continue;
                    }
                }

                *ai_state = AiState::Patrolling;
            }
            AiState::Chasing => {
                if let Some(target) = target {
                    let mut target_pos = target.pos;
                    if target.visible {
                        target_pos = target.pos;
                    }
                    let distance = my_pos.chebyshev_distance(target_pos);

                    if distance <= 1 && target.visible {
                        attack_intents.write(AttackIntent {
                            attacker: entity,
                            target: target.entity,
                        });
                        commands.entity(entity).insert(AiTarget {
                            entity: target.entity,
                            last_pos: target_pos,
                            last_seen: current_turn,
                            locked: true,
                        });
                        energy.spend_action();
                        continue;
                    }

                    let style = current_style;
                    let gun = tactics.loaded_gun.map(|choice| GunChoice {
                        entity: choice.entity,
                        profile: adjust_profile_for_style(choice.profile, style, personality),
                    });
                    let bow = tactics.bow.map(|choice| BowChoice {
                        attack: choice.attack,
                        profile: adjust_profile_for_style(choice.profile, style, personality),
                    });

                    let stamina_ok = stamina
                        .as_deref()
                        .is_some_and(|stamina| stamina.current >= SPELL_STAMINA_COST);
                    let can_aim = gun.is_some()
                        || bow.is_some()
                        || (target.visible
                            && stamina_ok
                            && (tactics.grenade.is_some() || tactics.molotov.is_some()));
                    let cursor_pos =
                        ensure_cursor_origin(&mut commands, entity, &mut cursor, my_pos, false);
                    let cursor_to_target = target_pos - cursor_pos;
                    if can_aim && target.visible {
                        orient_cursor(
                            &mut commands,
                            entity,
                            &mut cursor,
                            my_pos,
                            target_pos,
                            &mut viewshed,
                        );
                    } else if !target.visible {
                        track_unseen_objective_cursor(
                            &mut commands,
                            entity,
                            &mut cursor,
                            my_pos,
                            target_pos,
                            current_turn,
                            &mut viewshed,
                        );
                    }
                    if let Some(memory) = ai_memory.as_deref_mut() {
                        memory.cursor_steps = if can_aim && target.visible && !cursor_to_target.is_zero()
                        {
                            memory.cursor_steps.saturating_add(1)
                        } else {
                            0
                        };
                    }
                    let aim_origin = cursor.as_deref().map(|cursor| cursor.pos).unwrap_or_else(|| {
                        if can_aim && target.visible {
                            target_pos
                        } else {
                            cursor_pos
                        }
                    });
                    let aim_delta = aim_origin - my_pos;
                    let aim_endpoint = if target.visible { target_pos } else { aim_origin };
                    let aim_has_los =
                        has_clear_line_of_sight(my_pos, aim_endpoint, &game_map, &sand_cloud_tiles);
                    let aim_friendly_blocked = has_friendly_in_path(
                        my_pos,
                        aim_endpoint,
                        my_faction,
                        entity,
                        &spatial,
                        &npc_overview,
                    );
                    let cluster = count_hostiles_near(
                        target_pos,
                        tactics
                            .grenade
                            .map(|choice| choice.radius)
                            .or_else(|| tactics.molotov.map(|choice| choice.radius))
                            .unwrap_or(2),
                        my_faction,
                        player_info,
                        &npc_overview,
                    );

                    if target.visible && stamina_ok && aim_has_los {
                        if let Some(grenade) = tactics.grenade {
                            if (EXPLOSIVE_MIN_RANGE..=EXPLOSIVE_MAX_RANGE).contains(&distance)
                                && cluster >= 2
                                && !blast_hits_friendlies(
                                    target_pos,
                                    grenade.radius,
                                    my_pos,
                                    my_faction,
                                    entity,
                                    &npc_overview,
                                )
                            {
                                let _ = grenade.damage;
                                spell_intents.write(SpellCastIntent {
                                    caster: entity,
                                    radius: grenade.radius,
                                    target: target_pos,
                                    grenade_index: grenade.index,
                                });
                                commands.entity(entity).insert(AiTarget {
                                    entity: target.entity,
                                    last_pos: target_pos,
                                    last_seen: current_turn,
                                    locked: true,
                                });
                                energy.spend_action();
                                continue;
                            }
                        } else if let Some(molotov) = tactics.molotov {
                            if (EXPLOSIVE_MIN_RANGE..=EXPLOSIVE_MAX_RANGE).contains(&distance)
                                && cluster >= 2
                                && !blast_hits_friendlies(
                                    target_pos,
                                    molotov.radius,
                                    my_pos,
                                    my_faction,
                                    entity,
                                    &npc_overview,
                                )
                            {
                                molotov_intents.write(MolotovCastIntent {
                                    caster: entity,
                                    radius: molotov.radius,
                                    damage: molotov.damage,
                                    target: target_pos,
                                    item_index: molotov.index,
                                });
                                commands.entity(entity).insert(AiTarget {
                                    entity: target.entity,
                                    last_pos: target_pos,
                                    last_seen: current_turn,
                                    locked: true,
                                });
                                energy.spend_action();
                                continue;
                            }
                        }
                    }

                    if let Some(gun) = gun {
                        let in_range =
                            distance >= gun.profile.min_range && distance <= gun.profile.max_range;
                        let can_blind_fire = matches!(style, Some(AimingStyle::Suppression))
                            && !target.visible
                            && ai_target.is_some_and(|current| {
                                current.entity == target.entity
                                    && current.locked
                                    && current_turn.saturating_sub(current.last_seen) <= 4
                            });
                        let blind_fire_aligned =
                            can_blind_fire && cursor_aligned_with_target(aim_delta, target_pos - my_pos);
                        if in_range
                            && aim_has_los
                            && !aim_friendly_blocked
                            && (target.visible || blind_fire_aligned)
                        {
                            ranged_intents.write(RangedAttackIntent {
                                attacker: entity,
                                range: gun.profile.max_range,
                                dx: aim_delta.x,
                                dy: aim_delta.y,
                                gun_item: Some(gun.entity),
                            });
                            commands.entity(entity).insert(AiTarget {
                                entity: target.entity,
                                last_pos: target_pos,
                                last_seen: current_turn,
                                locked: true,
                            });
                            energy.spend_action();
                            continue;
                        }
                    }

                    if let Some(bow) = bow {
                        let in_range = target.visible
                            && aim_has_los
                            && !aim_friendly_blocked
                            && distance >= bow.profile.min_range
                            && distance <= bow.profile.max_range;
                        if in_range {
                            let max_component = aim_delta.x.abs().max(aim_delta.y.abs()).max(1);
                            let scale = bow.profile.max_range.div_euclid(max_component).max(1);
                            let endpoint =
                                my_pos + GridVec::new(aim_delta.x * scale, aim_delta.y * scale);
                            crate::systems::projectile::spawn_arrow(
                                &mut commands,
                                my_pos,
                                endpoint,
                                bow.attack,
                                entity,
                            );
                            commands.entity(entity).insert(AiTarget {
                                entity: target.entity,
                                last_pos: target_pos,
                                last_seen: current_turn,
                                locked: true,
                            });
                            energy.spend_action();
                            continue;
                        }
                    }

                    if tactics.loaded_gun.is_none()
                        && tactics.reloadable_gun.is_some()
                        && distance > 2
                        && (!target.visible || distance >= 4)
                        && maybe_reload_gun(tactics.reloadable_gun, &mut item_kinds)
                    {
                        energy.spend_action();
                        continue;
                    }

                    if personality.aggression < 0.55
                        && let Some(profile) = gun
                            .map(|gun| gun.profile)
                            .or_else(|| bow.map(|bow| bow.profile))
                    {
                        if distance < profile.min_range && distance > 1 {
                            if let Some(dir) = retreat_direction(
                                my_pos,
                                target_pos,
                                entity,
                                my_faction,
                                &game_map,
                                &spatial,
                                &blockers,
                                &npc_overview,
                                &sand_cloud_tiles,
                                target.visible,
                                ai_memory.as_deref(),
                                current_turn,
                            ) {
                                move_intents.write(MoveIntent {
                                    entity,
                                    dx: dir.x,
                                    dy: dir.y,
                                });
                                record_move(&mut ai_memory, dir, current_turn);
                                orient_cursor_to_move(
                                    &mut commands,
                                    entity,
                                    &mut cursor,
                                    my_pos,
                                    dir,
                                    &mut viewshed,
                                );
                                energy.spend_action();
                                continue;
                            }
                        }
                    }

                    let flank_now = ai_memory
                        .as_deref()
                        .is_some_and(|memory| memory.stationary_turns >= STUCK_FLANK_TURNS + 1);
                    let should_flank = aim_friendly_blocked
                        || flank_now
                        || (!target.visible && distance <= 12)
                        || (!aim_has_los && distance <= 10);
                    let goal = if should_flank {
                        flank_goal(entity, current_turn, my_pos, target_pos)
                    } else {
                        target_pos
                    };

                    if let Some(dir) = first_step_toward(
                        my_pos,
                        goal,
                        entity,
                        my_faction,
                        &game_map,
                        &spatial,
                        &blockers,
                        &npc_overview,
                        true,
                        ai_memory.as_deref(),
                        current_turn,
                    ) {
                        move_intents.write(MoveIntent {
                            entity,
                            dx: dir.x,
                            dy: dir.y,
                        });
                        record_move(&mut ai_memory, dir, current_turn);
                        orient_cursor_to_move(
                            &mut commands,
                            entity,
                            &mut cursor,
                            my_pos,
                            dir,
                            &mut viewshed,
                        );
                        energy.spend_action();
                        continue;
                    }

                    energy.spend_action();
                    continue;
                }

                if let Some(goal) = objective {
                    if my_pos == goal {
                        let search_rotation_steps = ai_memory
                            .as_deref()
                            .map(|memory| memory.search_rotation_steps)
                            .unwrap_or(0);
                        if search_rotation_steps > 0 {
                            if let Some(dir) = rotation_probe_step(
                                my_pos,
                                entity,
                                my_faction,
                                &cursor,
                                search_rotation_steps,
                                &game_map,
                                &spatial,
                                &blockers,
                                &npc_overview,
                            ) {
                                move_intents.write(MoveIntent {
                                    entity,
                                    dx: dir.x,
                                    dy: dir.y,
                                });
                                record_move(&mut ai_memory, dir, current_turn);
                            }
                            if scan_rotation_due(entity, current_turn)
                                && let Some(memory) = ai_memory.as_deref_mut()
                            {
                                rotate_cursor(
                                    &mut commands,
                                    entity,
                                    &mut cursor,
                                    my_pos,
                                    &mut viewshed,
                                    &mut memory.search_rotation_steps,
                                );
                            }
                            energy.spend_action();
                            continue;
                        }

                        if let Some(memory) = ai_memory.as_deref_mut() {
                            if memory.search_attempts < MAX_SEARCH_SWEEPS {
                                memory.search_attempts += 1;
                                memory.search_rotation_steps = FULL_ROTATION_STEPS;
                                prime_scan_cursor(
                                    &mut commands,
                                    entity,
                                    &mut cursor,
                                    my_pos,
                                    Some(goal),
                                    &mut viewshed,
                                );
                                rotate_cursor(
                                    &mut commands,
                                    entity,
                                    &mut cursor,
                                    my_pos,
                                    &mut viewshed,
                                    &mut memory.search_rotation_steps,
                                );
                                energy.spend_action();
                                continue;
                            }

                            memory.last_known_pos = None;
                            memory.search_attempts = 0;
                        }

                        clear_engagement(&mut commands, entity);
                        *ai_state = AiState::Patrolling;
                    } else if let Some(dir) = first_step_toward(
                        my_pos,
                        goal,
                        entity,
                        my_faction,
                        &game_map,
                        &spatial,
                        &blockers,
                        &npc_overview,
                        true,
                        ai_memory.as_deref(),
                        current_turn,
                    ) {
                        move_intents.write(MoveIntent {
                            entity,
                            dx: dir.x,
                            dy: dir.y,
                        });
                        record_move(&mut ai_memory, dir, current_turn);
                        orient_cursor_to_move(
                            &mut commands,
                            entity,
                            &mut cursor,
                            my_pos,
                            dir,
                            &mut viewshed,
                        );
                    }

                    energy.spend_action();
                    continue;
                }

                *ai_state = AiState::Patrolling;
            }
            AiState::Idle | AiState::Patrolling => {}
        }

        if tactics.reloadable_gun.is_some()
            && objective.is_none()
            && maybe_reload_gun(tactics.reloadable_gun, &mut item_kinds)
        {
            energy.spend_action();
            continue;
        }

        if objective.is_none() && inventory_view.is_some_and(|inv| inv.items.len() < 9) {
            if let Some(item_pos) = floor_items
                .iter()
                .map(|(_, position)| position.as_grid_vec())
                .filter(|item_pos| has_visibility(viewshed.as_deref(), *item_pos))
                .filter(|item_pos| my_pos.chebyshev_distance(*item_pos) <= ITEM_INTEREST_RANGE)
                .min_by_key(|item_pos| my_pos.chebyshev_distance(*item_pos))
            {
                if item_pos == my_pos {
                    pickup_intents.write(PickupItemIntent { picker: entity });
                } else if let Some(dir) = first_step_toward(
                    my_pos,
                    item_pos,
                    entity,
                    my_faction,
                    &game_map,
                    &spatial,
                    &blockers,
                    &npc_overview,
                    false,
                    ai_memory.as_deref(),
                    current_turn,
                ) {
                    move_intents.write(MoveIntent {
                        entity,
                        dx: dir.x,
                        dy: dir.y,
                    });
                    record_move(&mut ai_memory, dir, current_turn);
                    orient_cursor(
                        &mut commands,
                        entity,
                        &mut cursor,
                        my_pos,
                        my_pos + dir,
                        &mut viewshed,
                    );
                }
                energy.spend_action();
                continue;
            }
        }

        let search_rotation_steps = ai_memory
            .as_deref()
            .map(|memory| memory.search_rotation_steps)
            .unwrap_or(0);
        if search_rotation_steps > 0 {
            if let Some(dir) = rotation_probe_step(
                my_pos,
                entity,
                my_faction,
                &cursor,
                search_rotation_steps,
                &game_map,
                &spatial,
                &blockers,
                &npc_overview,
            ) {
                move_intents.write(MoveIntent {
                    entity,
                    dx: dir.x,
                    dy: dir.y,
                });
                record_move(&mut ai_memory, dir, current_turn);
            }
            if scan_rotation_due(entity, current_turn) && let Some(memory) = ai_memory.as_deref_mut() {
                rotate_cursor(
                    &mut commands,
                    entity,
                    &mut cursor,
                    my_pos,
                    &mut viewshed,
                    &mut memory.search_rotation_steps,
                );
            }
            energy.spend_action();
            continue;
        }

        if current_turn != 0 && current_turn % look_interval(entity) == 0 {
            let _ = ensure_cursor_origin(&mut commands, entity, &mut cursor, my_pos, false);
            let scan_interest = objective.or_else(|| patrol_origin.map(|origin| origin.0));
            prime_scan_cursor(
                &mut commands,
                entity,
                &mut cursor,
                my_pos,
                scan_interest,
                &mut viewshed,
            );
            if let Some(memory) = ai_memory.as_deref_mut() {
                let mut remaining_steps = 1;
                rotate_cursor(
                    &mut commands,
                    entity,
                    &mut cursor,
                    my_pos,
                    &mut viewshed,
                    &mut remaining_steps,
                );
                memory.search_rotation_steps = 0;
            } else {
                let mut remaining_steps = 1;
                rotate_cursor(
                    &mut commands,
                    entity,
                    &mut cursor,
                    my_pos,
                    &mut viewshed,
                    &mut remaining_steps,
                );
            }
            energy.spend_action();
            continue;
        }

        if let Some(origin) = patrol_origin.map(|origin| origin.0) {
            if let Some(dir) = patrol_direction(
                entity,
                my_pos,
                origin,
                my_faction,
                &game_map,
                &spatial,
                &blockers,
                &npc_overview,
                ai_memory.as_deref(),
                current_turn,
            ) {
                move_intents.write(MoveIntent {
                    entity,
                    dx: dir.x,
                    dy: dir.y,
                });
                record_move(&mut ai_memory, dir, current_turn);
                orient_cursor(
                    &mut commands,
                    entity,
                    &mut cursor,
                    my_pos,
                    my_pos + dir,
                    &mut viewshed,
                );
                energy.spend_action();
                continue;
            }
        }

        energy.spend_action();
    }
}

pub fn energy_accumulate_system(mut query: Query<(&Speed, &mut Energy)>) {
    for (speed, mut energy) in &mut query {
        energy.accumulate(speed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walkable(_: GridVec) -> bool {
        true
    }

    #[test]
    fn a_star_returns_adjacent_step() {
        let start = GridVec::new(4, 4);
        let goal = GridVec::new(5, 5);
        assert_eq!(
            a_star_first_step_pub(start, goal, walkable),
            Some(GridVec::new(0, 1))
        );
    }

    #[test]
    fn a_star_routes_around_wall() {
        let start = GridVec::new(0, 0);
        let goal = GridVec::new(3, 0);
        let walls = HashSet::from([GridVec::new(1, 0), GridVec::new(1, 1)]);
        let step = a_star_first_step_pub(start, goal, |pos| !walls.contains(&pos));
        assert_eq!(step, Some(GridVec::new(0, -1)));
    }

    #[test]
    fn pursuit_boost_decays_over_time() {
        let boost = AiPursuitBoost {
            extra_range: 12,
            last_spotted_turn: 10,
        };
        let personality = AiPersonality {
            aggression: 0.5,
            courage: 0.5,
        };
        assert_eq!(
            effective_awareness_range(12, Some(&boost), personality, 10),
            29
        );
        assert_eq!(
            effective_awareness_range(12, Some(&boost), personality, 18),
            27
        );
        assert_eq!(
            effective_awareness_range(12, Some(&boost), personality, 58),
            17
        );
    }

    #[test]
    fn faction_hostility_is_strictly_cross_faction() {
        assert!(factions_are_hostile(Faction::Outlaws, Faction::Lawmen));
        assert!(!factions_are_hostile(Faction::Police, Faction::Police));
    }

    #[test]
    fn flee_triggers_without_healing() {
        let health = Health {
            current: 5,
            max: 20,
        };
        let personality = AiPersonality {
            aggression: 0.9,
            courage: 0.2,
        };
        assert!(should_flee(Some(&health), false, personality));
        assert!(!should_flee(Some(&health), true, personality));
    }

    #[test]
    fn heal_check_requires_below_half_hp() {
        let entity = Entity::from_bits(1234);
        let trigger_turn =
            entity.to_bits() as u32 % HEAL_CHECK_INTERVAL
                + (((entity.to_bits() >> 8) as u32) % HEAL_CHECK_PERIOD) * HEAL_CHECK_INTERVAL;
        let exactly_half = Health {
            current: 10,
            max: 20,
        };
        let wounded = Health { current: 9, max: 20 };

        assert!(!should_attempt_heal(entity, trigger_turn, &exactly_half));
        assert!(should_attempt_heal(entity, trigger_turn, &wounded));
    }

    #[test]
    fn heal_check_is_sparse_and_cadenced() {
        let entity = Entity::from_bits(4321);
        let trigger_turn =
            entity.to_bits() as u32 % HEAL_CHECK_INTERVAL
                + (((entity.to_bits() >> 8) as u32) % HEAL_CHECK_PERIOD) * HEAL_CHECK_INTERVAL;
        let wounded = Health { current: 6, max: 20 };

        assert!(should_attempt_heal(entity, trigger_turn, &wounded));
        assert!(!should_attempt_heal(entity, trigger_turn + 1, &wounded));
        assert!(!should_attempt_heal(
            entity,
            trigger_turn + HEAL_CHECK_INTERVAL,
            &wounded
        ));
    }

    #[test]
    fn closest_visible_target_wins_priority() {
        let threats = vec![
            Threat {
                entity: Entity::from_bits(1),
                pos: GridVec::new(10, 10),
                distance: 5,
                score: 60,
            },
            Threat {
                entity: Entity::from_bits(2),
                pos: GridVec::new(7, 10),
                distance: 2,
                score: 4,
            },
            Threat {
                entity: Entity::from_bits(3),
                pos: GridVec::new(8, 10),
                distance: 3,
                score: 120,
            },
        ];

        let best = select_priority_target(&threats).expect("target");
        assert_eq!(best.entity, Entity::from_bits(2));
    }

    #[test]
    fn target_priority_breaks_ties_with_pressure() {
        let threats = vec![
            Threat {
                entity: Entity::from_bits(11),
                pos: GridVec::new(10, 10),
                distance: 3,
                score: 10,
            },
            Threat {
                entity: Entity::from_bits(12),
                pos: GridVec::new(11, 10),
                distance: 3,
                score: 42,
            },
        ];

        let best = select_priority_target(&threats).expect("target");
        assert_eq!(best.entity, Entity::from_bits(12));
    }

    #[test]
    fn moving_scan_cursor_fans_around_interest() {
        let origin = GridVec::new(10, 10);
        let interest = GridVec::new(20, 10);
        let entity = Entity::from_bits(99);

        let expected = [
            GridVec::new(1, 0).rotate_90_ccw(),
            GridVec::new(1, 0),
            GridVec::new(1, 0).rotate_90_cw(),
        ];
        let mut actual = HashSet::new();
        for turn in [0, 40, 80, 120, 160, 200] {
            let dir =
                cardinal_step_toward(moving_scan_cursor_target(origin, interest, entity, turn) - origin);
            actual.insert(dir);
            assert!(expected.contains(&dir), "unexpected scan direction: {:?}", dir);
        }
        assert_eq!(actual.len(), 3, "scan should cover left, forward, and right");
    }
}
