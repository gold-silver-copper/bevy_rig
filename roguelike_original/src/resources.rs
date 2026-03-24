use std::collections::{HashMap, VecDeque};
#[cfg(not(feature = "windowed"))]
use std::collections::HashSet;
#[cfg(feature = "windowed")]
use std::time::Duration;

use bevy::prelude::*;

use crate::components::{Caliber, CollectibleKind};
use crate::gamemap::GameMap;
use crate::grid_vec::GridVec;
use crate::noise::NoiseSeed;
use crate::typedefs::{MyPoint, SPAWN_X, SPAWN_Y};
use crate::typeenums::{Floor, Props};

/// Explosion radius for gunpowder barrels (Chebyshev distance).
pub const GUNPOWDER_BARREL_EXPLOSION_RADIUS: i32 = 3;

/// Bevy resource wrapping the game map for ECS access.
#[derive(Resource)]
pub struct GameMapResource(pub GameMap);

impl GameMapResource {
    /// Places persistent SandCloud floor tiles in a directionally-biased area.
    ///
    /// Used by gun smoke, sand throws, and similar effects. Tiles within
    /// `scan_radius` of `origin` are converted to `SandCloud` if they pass
    /// the directional bias check:
    ///   `effective_radius = base_radius + max(0, dot) × directional_scale`
    /// where `dot` is the cosine of the angle between the tile offset and
    /// the given `direction` unit vector.
    pub fn place_sand_cloud(
        &mut self,
        origin: GridVec,
        turn: u32,
        direction: (f64, f64),
        scan_radius: i32,
        base_radius: f64,
        directional_scale: f64,
    ) {
        use crate::typeenums::{Floor, Props};

        let mut tiles_to_cloud: Vec<(GridVec, Option<Floor>)> = Vec::new();
        for dx in -scan_radius..=scan_radius {
            for dy in -scan_radius..=scan_radius {
                let fx = dx as f64;
                let fy = dy as f64;
                let dist = (fx * fx + fy * fy).sqrt();
                let dot = if dist > 0.01 {
                    (fx * direction.0 + fy * direction.1) / dist
                } else {
                    0.0
                };
                let effective_radius = base_radius + dot.max(0.0) * directional_scale;
                if dist > effective_radius {
                    continue;
                }
                let pos = origin + GridVec::new(dx, dy);
                if let Some(voxel) = self.0.get_voxel_at(&pos)
                    && !matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall))
                    && !matches!(
                        voxel.floor,
                        Some(Floor::ShallowWater) | Some(Floor::DeepWater)
                    )
                {
                    tiles_to_cloud.push((pos, voxel.floor.clone()));
                }
            }
        }
        for (pos, prev_floor) in tiles_to_cloud {
            self.0
                .sand_cloud_previous_floor
                .entry(pos)
                .or_insert(prev_floor);
            if let Some(voxel) = self.0.get_voxel_at_mut(&pos) {
                voxel.floor = Some(Floor::SandCloud);
            }
            self.0.sand_cloud_turns.insert(pos, turn);
        }
    }

    /// Detonates a gunpowder barrel at the given position: sets fire in a radius,
    /// destroys flammable props, and tracks fire turns for burnout.
    /// Returns the number of props destroyed by the explosion.
    pub fn detonate_gunpowder_barrel(&mut self, origin: GridVec, turn: u32) -> i32 {
        let radius = GUNPOWDER_BARREL_EXPLOSION_RADIUS;
        let mut fire_count = 0;
        // Collect tiles to ignite so we don't hold mutable borrow during iteration.
        let mut tiles_to_ignite: Vec<(GridVec, Option<Floor>, bool)> = Vec::new();
        for gdx in -radius..=radius {
            for gdy in -radius..=radius {
                let gp_dist = gdx.abs().max(gdy.abs());
                if gp_dist > radius {
                    continue;
                }
                let fire_pos = origin + GridVec::new(gdx, gdy);
                if let Some(voxel) = self.0.get_voxel_at(&fire_pos) {
                    if matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall)) {
                        continue;
                    }
                    let is_water = matches!(
                        voxel.floor,
                        Some(Floor::ShallowWater) | Some(Floor::DeepWater)
                    );
                    let has_flammable = voxel.props.as_ref().is_some_and(|p| p.is_flammable());
                    let prev = voxel.floor.clone();
                    tiles_to_ignite.push((fire_pos, prev, has_flammable));
                    if is_water {
                        continue;
                    }
                }
            }
        }
        for (fire_pos, prev, has_flammable) in tiles_to_ignite {
            if has_flammable {
                if let Some(voxel) = self.0.get_voxel_at_mut(&fire_pos) {
                    voxel.props = None;
                }
                fire_count += 1;
            }
            // Save previous floor before setting fire (avoids double mutable borrow).
            let is_water = self.0.get_voxel_at(&fire_pos).is_some_and(|v| {
                matches!(v.floor, Some(Floor::ShallowWater) | Some(Floor::DeepWater))
            });
            if !is_water {
                self.0.fire_previous_floor.entry(fire_pos).or_insert(prev);
                self.0.fire_turns.entry(fire_pos).or_insert(turn);
                if let Some(voxel) = self.0.get_voxel_at_mut(&fire_pos) {
                    voxel.floor = Some(Floor::Fire);
                }
            }
        }
        fire_count
    }
}

/// Bevy resource holding the camera position (follows the tracked entity).
#[derive(Resource)]
pub struct CameraPosition(pub MyPoint);

/// Bevy resource holding the seed used for deterministic procedural generation.
///
/// Changing this seed produces a completely different but equally valid map.
/// Keeping the same seed always reproduces exactly the same world, which is
/// essential for debugging, replays, and multiplayer synchronization.
#[derive(Resource, Debug, Clone, Copy)]
pub struct MapSeed(pub NoiseSeed);

/// Top-level game state managed by Bevy's state machine.
/// Systems that should only run during gameplay use
/// `.run_if(in_state(GameState::Playing))`.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub enum GameState {
    #[default]
    Playing,
    Paused,
    Victory,
    Dead,
}

/// Tracks which input mode the game is in and inventory selection state.
/// In `Game` mode, normal movement/action keys are processed.
/// In `Inventory` mode, the inventory overlay is shown and
/// arrow/enter keys navigate and use items.
/// In `EscMenu` mode, the escape menu is shown with resume/restart/quit.
/// Also tracks help and welcome overlay visibility.
#[derive(Resource, Debug)]
pub struct InputState {
    pub mode: InputMode,
    pub inv_selection: usize,
    pub help_visible: bool,
    pub welcome_visible: bool,
    /// Set to true when the player requests a reload (R key).
    pub reload_pending: bool,
    /// Stamina cost for a pending dive action.
    pub dive_stamina_pending: i32,
    /// Stamina cost for a pending special ability action.
    pub ability_stamina_pending: i32,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            mode: InputMode::Game,
            inv_selection: 0,
            help_visible: false,
            welcome_visible: true, // shown on first launch
            reload_pending: false,
            dive_stamina_pending: 0,
            ability_stamina_pending: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Game,
    Inventory,
    EscMenu,
}

/// Turn-phase sub-state that controls the flow within `GameState::Playing`.
///
/// State machine:
///   AwaitingInput → PlayerTurn → WorldTurn → AwaitingInput
///
/// - **AwaitingInput** – input system is active; game waits for player action.
/// - **PlayerTurn** – player's action is resolved (movement, combat).
/// - **WorldTurn** – NPC AI and world-tick systems run; energy is accumulated.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, SubStates)]
#[source(GameState = GameState::Playing)]
pub enum TurnState {
    #[default]
    AwaitingInput,
    PlayerTurn,
    WorldTurn,
}

/// Spatial index for O(1) entity-at-position lookup.
///
/// Maintained by `spatial_index_system` which runs at the start of every
/// `Update` tick. Any system that needs to know "which entities are at
/// position X" reads this resource instead of iterating all entities.
#[derive(Resource, Debug, Default)]
pub struct SpatialIndex {
    pub map: HashMap<MyPoint, Vec<Entity>>,
}

impl SpatialIndex {
    /// Returns all entities occupying the given tile, or an empty slice.
    pub fn entities_at(&self, point: &MyPoint) -> &[Entity] {
        self.map.get(point).map_or(&[], |v| v.as_slice())
    }

    /// Removes an entity from a specific tile in the index.
    ///
    /// Used by the movement system to maintain the spatial index invariant
    /// inline when an entity moves: the entity is removed from its old tile
    /// before being added to the new one.
    pub fn remove_entity(&mut self, point: &MyPoint, entity: Entity) {
        if let Some(entities) = self.map.get_mut(point) {
            entities.retain(|&e| e != entity);
        }
    }

    /// Adds an entity to a specific tile in the index.
    pub fn add_entity(&mut self, point: MyPoint, entity: Entity) {
        self.map.entry(point).or_default().push(entity);
    }

    /// Atomically moves an entity from one tile to another in the index.
    ///
    /// This is the correct primitive for maintaining the spatial index
    /// invariant during movement: it ensures the entity is never present
    /// in both the old and new tile simultaneously, and never absent from
    /// both. This prevents the stale-read race condition where two entities
    /// could move to the same tile in the same frame.
    ///
    /// **Invariant**: after `move_entity(old, new, e)`, the entity `e` is
    /// present at `new` and absent from `old`.
    pub fn move_entity(&mut self, old: &MyPoint, new: MyPoint, entity: Entity) {
        self.remove_entity(old, entity);
        self.add_entity(new, entity);
    }
}

/// Counts elapsed world turns. Used by the wave spawning system to
/// determine how many and how often new enemies appear.
#[derive(Resource, Debug, Default)]
pub struct TurnCounter(pub u32);

/// Tracks the total number of hostile entities killed by the player.
/// Displayed in the status bar as the player's score.
#[derive(Resource, Debug, Default)]
pub struct KillCount(pub u32);

/// Accumulator for combat log messages displayed in the status bar.
/// Maintains a rolling history of recent messages using a bounded
/// ring buffer (`VecDeque`) for O(1) enqueue/dequeue — the correct
/// data structure for a bounded FIFO queue.
///
/// Each message may optionally carry a world position so the render
/// system can filter to only show events visible to the player.
#[derive(Resource, Debug, Default)]
pub struct CombatLog {
    pub messages: VecDeque<String>,
    /// Parallel deque: the world position associated with each message.
    /// `None` means the message is always shown (e.g., level-up, UI feedback).
    positions: VecDeque<Option<GridVec>>,
}

/// Maximum number of messages retained in the combat log.
const MAX_COMBAT_LOG_MESSAGES: usize = 50;

/// Active combat particles for rendering grenade/bullet animations.
/// Each entry is (position, remaining_lifetime_frames, delay_before_visible, is_sand, velocity_x, velocity_y).
/// Particles with delay > 0 are not yet visible; they count down each tick.
/// `is_sand` particles are rendered as smoke/plume that drifts and dissipates.
/// Velocity fields allow particles to move each tick for visible motion.
///
/// Also stores pre-computed sound indicator positions for the render system.
#[derive(Resource, Debug, Default)]
pub struct SpellParticles {
    pub particles: Vec<(MyPoint, u32, u32, bool, i32, i32)>,
    /// World positions where "!" sound indicators should appear this frame.
    /// Computed by the particle tick system from SoundEvents + player viewshed.
    pub sound_indicators: Vec<MyPoint>,
    /// Frame accumulator for frame-rate-independent particle ticking.
    /// Particles advance once every `PARTICLE_TICK_INTERVAL` frames so that
    /// animations remain readable at high frame rates (e.g., 60 FPS).
    frame_accumulator: u32,
}

/// Maximum number of active combat particles to prevent unbounded growth.
const MAX_PARTICLES: usize = 1200;

/// Particles advance once every this many render frames to stay readable
/// at high frame rates. At 60 FPS, 3 → particles tick at ~20 Hz.
const PARTICLE_TICK_INTERVAL: u32 = 3;

impl SpellParticles {
    /// Adds an expanding ring of particles for a grenade blast.
    /// Particles at greater distances from the origin appear later, creating
    /// an outward-traveling shrapnel wave effect. Particles drift outward.
    pub fn add_aoe(&mut self, origin: MyPoint, lifetime: u32) {
        let radius = 3i32; // visual radius of the particle ring
        let frames_per_ring = 2u32; // ticks of delay per distance unit

        for r in 1..=radius {
            // Generate a ring of particles at Chebyshev distance r
            for dx in -r..=r {
                for dy in -r..=r {
                    // Only the perimeter of the Chebyshev ring
                    if dx.abs().max(dy.abs()) != r {
                        continue;
                    }
                    if self.particles.len() >= MAX_PARTICLES {
                        return;
                    }
                    let pos = origin + MyPoint::new(dx, dy);
                    let delay = (r as u32 - 1) * frames_per_ring;
                    // Particles drift outward from origin
                    let vx = dx.signum();
                    let vy = dy.signum();
                    self.particles.push((pos, lifetime, delay, false, vx, vy));
                }
            }
        }
    }

    /// Ticks all particles: counts down delays, then lifetimes and moves particles.
    /// Uses a frame accumulator so particles advance at a fixed rate (~20 Hz)
    /// regardless of the render frame rate, keeping animations readable.
    pub fn tick(&mut self) {
        self.frame_accumulator += 1;
        if self.frame_accumulator < PARTICLE_TICK_INTERVAL {
            return; // not time to advance particles yet
        }
        self.frame_accumulator = 0;

        self.particles
            .retain_mut(|(pos, life, delay, _is_sand, vx, vy)| {
                if *delay > 0 {
                    *delay -= 1;
                    true // still waiting to appear
                } else {
                    // Move the particle for visible motion
                    if *life > 0 {
                        pos.x += *vx;
                        pos.y += *vy;
                    }
                    *life = life.saturating_sub(1);
                    // Slow down particles as they age (plume dissipation)
                    if *life < 3 {
                        *vx = 0;
                        *vy = 0;
                    }
                    *life > 0
                }
            });
    }
}

impl CombatLog {
    /// Internal: appends a message with an optional position and trims oldest entry.
    fn push_inner(&mut self, message: String, pos: Option<GridVec>) {
        self.messages.push_back(message);
        self.positions.push_back(pos);
        if self.messages.len() > MAX_COMBAT_LOG_MESSAGES {
            self.messages.pop_front();
            self.positions.pop_front();
        }
    }

    /// Adds a message (always visible) and trims the oldest entry.
    /// O(1) amortised via `VecDeque::pop_front` (no element shifting).
    pub fn push(&mut self, message: String) {
        self.push_inner(message, None);
    }

    /// Adds a message tagged with a world position for visibility filtering.
    pub fn push_at(&mut self, message: String, pos: GridVec) {
        self.push_inner(message, Some(pos));
    }

    /// Adds a message with an optional position. Shorthand for the common
    /// pattern of `if pos { push_at } else { push }`.
    pub fn push_opt(&mut self, message: String, pos: Option<GridVec>) {
        self.push_inner(message, pos);
    }

    /// Clears all entries (messages and positions).
    pub fn clear(&mut self) {
        self.messages.clear();
        self.positions.clear();
    }

    /// Returns the most recent `n` messages as owned references.
    pub fn recent(&self, n: usize) -> Vec<&str> {
        let len = self.messages.len();
        let start = len.saturating_sub(n);
        self.messages
            .iter()
            .skip(start)
            .map(|s| s.as_str())
            .collect()
    }

    /// Returns the most recent `n` messages that are either untagged
    /// (always visible) or tagged with a position inside `visible`.
    pub fn recent_visible(
        &self,
        n: usize,
        visible: &std::collections::HashSet<GridVec>,
    ) -> Vec<&str> {
        let visible_msgs: Vec<&str> = self
            .messages
            .iter()
            .zip(self.positions.iter())
            .filter(|(_, pos)| pos.is_none_or(|p| visible.contains(&p)))
            .map(|(msg, _)| msg.as_str())
            .collect();
        let start = visible_msgs.len().saturating_sub(n);
        visible_msgs[start..].to_vec()
    }

    /// Returns the most recent `n` messages tagged with positions inside `visible`.
    /// Untagged/global messages are excluded.
    pub fn recent_strictly_visible(
        &self,
        n: usize,
        visible: &std::collections::HashSet<GridVec>,
    ) -> Vec<&str> {
        let visible_msgs: Vec<&str> = self
            .messages
            .iter()
            .zip(self.positions.iter())
            .filter(|(_, pos)| pos.is_some_and(|p| visible.contains(&p)))
            .map(|(msg, _)| msg.as_str())
            .collect();
        let start = visible_msgs.len().saturating_sub(n);
        visible_msgs[start..].to_vec()
    }
}

/// Tracks blood splatters left by wounded entities when they move.
/// Maps world positions to the turn number when blood was placed,
/// allowing the renderer to darken blood over time.
#[derive(Resource, Debug, Default)]
pub struct BloodMap {
    pub stains: HashMap<GridVec, u32>,
}

/// Blood stains older than this many turns are removed to prevent unbounded growth.
const BLOOD_MAX_AGE: u32 = 20;

impl BloodMap {
    /// Removes blood stains older than `BLOOD_MAX_AGE` turns.
    pub fn prune(&mut self, current_turn: u32) {
        self.stains
            .retain(|_, &mut turn| current_turn.saturating_sub(turn) <= BLOOD_MAX_AGE);
    }
}

/// Marker resource indicating a restart has been requested.
#[derive(Resource, Debug, Default)]
pub struct RestartRequested(pub bool);

/// Wall tiles destroyed to create the player's starting breach. The opening
/// effect is deferred until the welcome overlay is dismissed.
#[derive(Resource, Debug)]
pub struct StartupExplosionPending {
    pub breach_tiles: Vec<GridVec>,
}

impl Default for StartupExplosionPending {
    fn default() -> Self {
        Self {
            breach_tiles: Vec::new(),
        }
    }
}

/// Tracks held keys in windowed mode so repeated input can emulate terminal
/// autorepeat: immediate first press, short delay, then steady repeats.
#[derive(Resource, Debug)]
pub struct WindowedKeyRepeat {
    #[cfg(feature = "windowed")]
    pub held: HashMap<bevy::input::keyboard::KeyCode, Duration>,
    #[cfg(not(feature = "windowed"))]
    pub held: HashSet<()>,
}

impl Default for WindowedKeyRepeat {
    fn default() -> Self {
        Self {
            #[cfg(feature = "windowed")]
            held: HashMap::default(),
            #[cfg(not(feature = "windowed"))]
            held: HashSet::default(),
        }
    }
}

/// When true, the player is dead but watching the game continue.
/// Set by pressing "." on the death screen. The `end_world_turn_system` system
/// transitions back to Dead state after each spectated world turn.
#[derive(Resource, Debug, Default)]
pub struct SpectatingAfterDeath(pub bool);

/// Tracks how many render frames the player has been dead, used to
/// gradually fade in the red death overlay.
#[derive(Resource, Debug, Default)]
pub struct DeathFade {
    /// Number of render frames since the player died.
    pub frames: u32,
}

/// God mode: when true, the player cannot take damage. Toggled with Shift+G.
#[derive(Resource, Debug, Default)]
pub struct GodMode(pub bool);

/// Prop health map: tracks damage dealt to props.
/// When accumulated damage exceeds the prop's max HP, it is destroyed.
#[derive(Resource, Debug, Default)]
pub struct PropHealth {
    /// Maps world position → current remaining HP for the prop at that tile.
    pub hp: HashMap<GridVec, i32>,
}

/// Extra world ticks remaining after a player action. Physical movement sets
/// this to 1 so that the world turn cycles twice (2 total ticks), making
/// physical movement slower than cursor movement (1 tick). The `end_world_turn_system`
/// system decrements this and stays in `WorldTurn` while it is positive.
#[derive(Resource, Debug, Default)]
pub struct ExtraWorldTicks(pub i32);

/// Collectible supplies stored separately from inventory slots.
/// These don't occupy inventory slots and are tracked by quantity.
#[derive(Resource, Debug, Clone)]
pub struct Collectibles {
    /// Percussion caps (needed for reloading: 1 per round)
    pub caps: i32,
    /// Arrows for bows.
    pub arrows: i32,
    /// .31 caliber lead bullets
    pub bullets_31: i32,
    /// .36 caliber lead bullets
    pub bullets_36: i32,
    /// .44 caliber lead bullets
    pub bullets_44: i32,
    /// .50 caliber lead bullets
    pub bullets_50: i32,
    /// .58 caliber lead bullets
    pub bullets_58: i32,
    /// .577 caliber lead bullets
    pub bullets_577: i32,
    /// .69 caliber lead bullets
    pub bullets_69: i32,
    /// Black powder charges (needed for reloading: 1 per round)
    pub powder: i32,
}

impl Default for Collectibles {
    fn default() -> Self {
        Self {
            caps: 10,
            arrows: 0,
            bullets_31: 0,
            bullets_36: 0,
            bullets_44: 0,
            bullets_50: 0,
            bullets_58: 0,
            bullets_577: 0,
            bullets_69: 0,
            powder: 10,
        }
    }
}

impl Collectibles {
    /// Creates a new `Collectibles` pool with 10 starting bullets of the
    /// given caliber (matching the player's starting gun).
    pub fn for_starting_caliber(caliber: Caliber) -> Self {
        let mut c = Self::default();
        *c.bullets_mut(caliber) = 10;
        c
    }
}

impl Collectibles {
    /// Returns an immutable reference to the bullet count for the given caliber.
    pub fn bullets(&self, caliber: Caliber) -> i32 {
        match caliber {
            Caliber::Cal31 => self.bullets_31,
            Caliber::Cal36 => self.bullets_36,
            Caliber::Cal44 => self.bullets_44,
            Caliber::Cal50 => self.bullets_50,
            Caliber::Cal58 => self.bullets_58,
            Caliber::Cal577 => self.bullets_577,
            Caliber::Cal69 => self.bullets_69,
        }
    }

    /// Returns a mutable reference to the bullet count for the given caliber.
    pub fn bullets_mut(&mut self, caliber: Caliber) -> &mut i32 {
        match caliber {
            Caliber::Cal31 => &mut self.bullets_31,
            Caliber::Cal36 => &mut self.bullets_36,
            Caliber::Cal44 => &mut self.bullets_44,
            Caliber::Cal50 => &mut self.bullets_50,
            Caliber::Cal58 => &mut self.bullets_58,
            Caliber::Cal577 => &mut self.bullets_577,
            Caliber::Cal69 => &mut self.bullets_69,
        }
    }

    /// Returns `true` if the pool has enough supplies to reload one round
    /// of the given caliber (1 matching bullet + 1 cap + 1 powder).
    pub fn can_reload(&self, caliber: Caliber) -> bool {
        self.bullets(caliber) > 0 && self.caps > 0 && self.powder > 0
    }

    /// Consumes one round of reload supplies (1 matching bullet + 1 cap + 1 powder).
    ///
    /// **Pre-condition**: `self.can_reload(caliber)` is `true`.
    pub fn consume_reload(&mut self, caliber: Caliber) {
        *self.bullets_mut(caliber) -= 1;
        self.caps -= 1;
        self.powder -= 1;
    }

    /// Adds a collectible pickup to the resource pool.
    pub fn collect(&mut self, kind: CollectibleKind) {
        match kind {
            CollectibleKind::Caps(n) => self.caps += n,
            CollectibleKind::Arrows(n) => self.arrows += n,
            CollectibleKind::Bullets31(n) => self.bullets_31 += n,
            CollectibleKind::Bullets36(n) => self.bullets_36 += n,
            CollectibleKind::Bullets44(n) => self.bullets_44 += n,
            CollectibleKind::Bullets50(n) => self.bullets_50 += n,
            CollectibleKind::Bullets58(n) => self.bullets_58 += n,
            CollectibleKind::Bullets577(n) => self.bullets_577 += n,
            CollectibleKind::Bullets69(n) => self.bullets_69 += n,
            CollectibleKind::Powder(n) => self.powder += n,
        }
    }
}

/// Dynamic RNG resource seeded through map seed + tick count.
/// Use this for all gameplay randomness to ensure deterministic behavior
/// that varies each tick.
#[derive(Resource, Debug, Default)]
pub struct DynamicRng {
    /// Monotonically increasing counter, incremented each world turn.
    pub tick: u64,
}

impl DynamicRng {
    /// Returns a deterministic pseudo-random value in [0.0, 1.0) for the
    /// given key, using map_seed + tick as the seed.
    pub fn roll(&self, map_seed: u64, key: u64) -> f64 {
        // LCG constant from Knuth (MMIX) for good bit-mixing properties
        let seed = map_seed
            .wrapping_add(self.tick)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(key);
        // Squirrel3-like hash for good distribution
        let mut h = seed;
        h ^= h >> 16;
        h = h.wrapping_mul(0x45d9f3b);
        h ^= h >> 16;
        h = h.wrapping_mul(0x45d9f3b);
        h ^= h >> 16;
        (h as u32) as f64 / u32::MAX as f64
    }

    /// Returns a random index in `[0, len)` using the dynamic RNG.
    pub fn random_index(&self, map_seed: u64, key: u64, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        (self.roll(map_seed, key) * len as f64) as usize % len
    }

    /// Advance the tick counter by one.
    pub fn advance(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Reset the tick counter (on game restart).
    pub fn reset(&mut self) {
        self.tick = 0;
    }
}

/// Tracks audible events (gunshots, explosions) that should produce "!" sound
/// indicators on the map in areas not visible to the player.
#[derive(Resource, Debug, Default)]
pub struct SoundEvents {
    /// (position, remaining_ticks)
    pub events: Vec<(GridVec, u32)>,
}

/// Maximum audible range (Chebyshev distance) for sound indicators.
pub const SOUND_RANGE: i32 = 20;

impl SoundEvents {
    /// Records an audible event at the given position.
    pub fn add(&mut self, pos: GridVec) {
        self.events.push((pos, 3));
    }

    /// Ticks down lifetimes and removes expired events.
    pub fn tick(&mut self) {
        self.events.retain_mut(|(_, life)| {
            *life = life.saturating_sub(1);
            *life > 0
        });
    }
}

/// The cursor position in world coordinates.
/// Moved with IJKL keys. Used for aiming and directional actions.
/// Always visible on the map.
/// Also tracks blink state for cursor rendering.
#[derive(Resource, Debug, Clone)]
pub struct CursorPosition {
    pub pos: MyPoint,
    /// Frame counter for cursor blink animation.
    blink_frame: u32,
    /// Number of frames per half-blink cycle.
    blink_half_period: u32,
}

impl Default for CursorPosition {
    fn default() -> Self {
        Self {
            pos: GridVec::new(SPAWN_X, SPAWN_Y),
            blink_frame: 0,
            blink_half_period: 24, // At 60 FPS: toggles every 24 frames → ~1.25 blinks/sec
        }
    }
}

impl CursorPosition {
    /// Creates a cursor positioned at the given world coordinate.
    pub fn at(pos: MyPoint) -> Self {
        Self {
            pos,
            blink_frame: 0,
            blink_half_period: 24,
        }
    }

    /// Returns true when the cursor should be visible (inverted colors).
    pub fn blink_visible(&self) -> bool {
        (self.blink_frame / self.blink_half_period).is_multiple_of(2)
    }

    /// Returns the raw blink frame counter.
    pub fn blink_frame(&self) -> u32 {
        self.blink_frame
    }

    /// Advance the blink counter by one frame.
    pub fn tick_blink(&mut self) {
        self.blink_frame = self.blink_frame.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid_vec::GridVec;

    // ─── CombatLog tests ─────────────────────────────────────────

    #[test]
    fn combat_log_default_is_empty() {
        let log = CombatLog::default();
        assert!(log.messages.is_empty());
    }

    #[test]
    fn combat_log_push_adds_message() {
        let mut log = CombatLog::default();
        log.push("Hello".into());
        assert_eq!(log.messages.len(), 1);
        assert_eq!(log.messages[0], "Hello");
    }

    #[test]
    fn combat_log_push_multiple_preserves_order() {
        let mut log = CombatLog::default();
        log.push("First".into());
        log.push("Second".into());
        log.push("Third".into());
        let msgs: Vec<&str> = log.messages.iter().map(|s| s.as_str()).collect();
        assert_eq!(msgs, vec!["First", "Second", "Third"]);
    }

    #[test]
    fn combat_log_rolling_window_trims_oldest() {
        let mut log = CombatLog::default();
        for i in 0..60 {
            log.push(format!("msg-{i}"));
        }
        assert_eq!(log.messages.len(), MAX_COMBAT_LOG_MESSAGES);
        // Oldest messages (0..9) were trimmed; first remaining is msg-10.
        assert_eq!(log.messages[0], "msg-10");
        assert_eq!(log.messages[MAX_COMBAT_LOG_MESSAGES - 1], "msg-59");
    }

    #[test]
    fn combat_log_recent_returns_last_n() {
        let mut log = CombatLog::default();
        log.push("A".into());
        log.push("B".into());
        log.push("C".into());
        log.push("D".into());
        let recent = log.recent(2);
        assert_eq!(recent, vec!["C", "D"]);
    }

    #[test]
    fn combat_log_recent_more_than_available() {
        let mut log = CombatLog::default();
        log.push("Only".into());
        let recent = log.recent(10);
        assert_eq!(recent, vec!["Only"]);
    }

    #[test]
    fn combat_log_recent_zero() {
        let mut log = CombatLog::default();
        log.push("Something".into());
        let recent = log.recent(0);
        assert!(recent.is_empty());
    }

    #[test]
    fn combat_log_recent_on_empty() {
        let log = CombatLog::default();
        let recent = log.recent(5);
        assert!(recent.is_empty());
    }

    // ─── SpatialIndex tests ──────────────────────────────────────

    #[test]
    fn spatial_index_default_empty() {
        let index = SpatialIndex::default();
        assert!(index.map.is_empty());
    }

    #[test]
    fn spatial_index_entities_at_empty_tile() {
        let index = SpatialIndex::default();
        let point = GridVec::new(5, 5);
        assert!(index.entities_at(&point).is_empty());
    }

    #[test]
    fn spatial_index_entities_at_populated_tile() {
        let mut index = SpatialIndex::default();
        let point = GridVec::new(3, 7);
        let entity = Entity::from_bits(42);
        index.map.entry(point).or_default().push(entity);
        let result = index.entities_at(&point);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], entity);
    }

    #[test]
    fn spatial_index_multiple_entities_at_same_tile() {
        let mut index = SpatialIndex::default();
        let point = GridVec::new(0, 0);
        let e1 = Entity::from_bits(1);
        let e2 = Entity::from_bits(2);
        index.map.entry(point).or_default().push(e1);
        index.map.entry(point).or_default().push(e2);
        let result = index.entities_at(&point);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&e1));
        assert!(result.contains(&e2));
    }

    #[test]
    fn spatial_index_remove_entity() {
        let mut index = SpatialIndex::default();
        let point = GridVec::new(5, 5);
        let e1 = Entity::from_bits(10);
        let e2 = Entity::from_bits(20);
        index.add_entity(point, e1);
        index.add_entity(point, e2);
        assert_eq!(index.entities_at(&point).len(), 2);
        index.remove_entity(&point, e1);
        let result = index.entities_at(&point);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], e2);
    }

    #[test]
    fn spatial_index_remove_entity_from_empty_tile() {
        let mut index = SpatialIndex::default();
        let point = GridVec::new(1, 1);
        let entity = Entity::from_bits(42);
        // Should not panic when removing from a tile with no entries.
        index.remove_entity(&point, entity);
        assert!(index.entities_at(&point).is_empty());
    }

    #[test]
    fn spatial_index_add_entity() {
        let mut index = SpatialIndex::default();
        let point = GridVec::new(7, 3);
        let entity = Entity::from_bits(99);
        index.add_entity(point, entity);
        let result = index.entities_at(&point);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], entity);
    }

    #[test]
    fn spatial_index_move_entity_atomicity() {
        let mut index = SpatialIndex::default();
        let old = GridVec::new(0, 0);
        let new = GridVec::new(1, 0);
        let entity = Entity::from_bits(50);
        index.add_entity(old, entity);
        assert_eq!(index.entities_at(&old).len(), 1);
        // Move the entity atomically.
        index.move_entity(&old, new, entity);
        // Old tile should be empty, new tile should have the entity.
        assert!(index.entities_at(&old).is_empty());
        assert_eq!(index.entities_at(&new).len(), 1);
        assert_eq!(index.entities_at(&new)[0], entity);
    }

    // ─── Collectibles default tests ─────────────────────────────────

    #[test]
    fn collectibles_default_has_starting_supplies() {
        let c = Collectibles::default();
        assert_eq!(c.caps, 10);
        assert_eq!(
            c.bullets_31, 0,
            "Default has no .31 bullets — use for_starting_caliber()"
        );
        assert_eq!(c.arrows, 0);
        assert_eq!(c.bullets_36, 0);
        assert_eq!(c.bullets_44, 0);
        assert_eq!(c.powder, 10);
    }

    // ─── CursorPosition default tests ───────────────────────────────

    #[test]
    fn cursor_default_at_spawn() {
        let cursor = CursorPosition::default();
        assert_eq!(cursor.pos.x, SPAWN_X);
        assert_eq!(cursor.pos.y, SPAWN_Y);
    }

    #[test]
    fn cursor_blink_visible_toggles() {
        let mut cursor = CursorPosition::default();
        // Initially visible (frame 0)
        assert!(cursor.blink_visible());
        // Advance past half_period (24 frames at 60 FPS)
        for _ in 0..24 {
            cursor.tick_blink();
        }
        // Should now be invisible
        assert!(!cursor.blink_visible());
        // Advance another half_period
        for _ in 0..24 {
            cursor.tick_blink();
        }
        // Should be visible again (full cycle)
        assert!(cursor.blink_visible());
    }
}
