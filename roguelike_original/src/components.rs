use std::collections::HashSet;

use bevy::prelude::*;

use crate::grid_vec::GridVec;
use crate::typedefs::{CoordinateUnit, MyPoint, RatColor};

/// World-grid position for any entity.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub x: CoordinateUnit,
    pub y: CoordinateUnit,
}

impl Position {
    /// Convert to a `GridVec` for vector arithmetic and distance calculations.
    #[inline]
    pub fn as_grid_vec(self) -> GridVec {
        GridVec::new(self.x, self.y)
    }
}

impl From<GridVec> for Position {
    #[inline]
    fn from(v: GridVec) -> Self {
        Self { x: v.x, y: v.y }
    }
}

impl From<Position> for GridVec {
    #[inline]
    fn from(p: Position) -> Self {
        GridVec::new(p.x, p.y)
    }
}

/// Marker component: tags the player-controlled entity.
#[derive(Component, Debug)]
pub struct PlayerControlled;

/// Marker component: tags an entity that has died.
/// Dead entities are excluded from most gameplay systems (pickup, healing,
/// combat actions, etc.) to prevent stale interactions.
#[derive(Component, Debug)]
pub struct Dead;

/// Visual representation used when rendering an entity on the grid.
#[derive(Component, Clone, Debug)]
pub struct Renderable {
    pub symbol: String,
    pub fg: RatColor,
    pub bg: RatColor,
}

/// Marker component: the camera will follow entities that have this.
#[derive(Component, Debug)]
pub struct CameraFollow;

/// Marker component: entity occupies its tile and blocks movement.
#[derive(Component, Debug)]
pub struct BlocksMovement;

/// Field-of-view component. Attached to entities that "see" the world.
/// `visible_tiles` is recomputed by `visibility_system` when dirty.
/// `revealed_tiles` accumulates all tiles ever seen (fog of war memory).
#[derive(Component, Debug)]
pub struct Viewshed {
    /// Maximum sight range (in tiles).
    pub range: CoordinateUnit,
    /// Set of world-grid coordinates currently visible.
    pub visible_tiles: HashSet<MyPoint>,
    /// Set of world-grid coordinates that have been seen at least once.
    /// Used for fog-of-war: revealed tiles are drawn dimmed when not visible.
    pub revealed_tiles: HashSet<MyPoint>,
    /// Whether the viewshed needs recalculation (dirty flag).
    pub dirty: bool,
}

/// Health pool for any entity that can take damage or be healed.
///
/// **Invariant**: `0 ≤ current ≤ max` is maintained by all mutating methods.
/// Direct field mutation is allowed for compatibility but callers should
/// prefer the provided methods to guarantee correctness.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Health {
    pub current: CoordinateUnit,
    pub max: CoordinateUnit,
}

impl Health {
    /// Applies `amount` damage, clamping `current` to `[0, max]`.
    /// Returns the *actual* damage dealt (may be less if current < amount).
    ///
    /// **Post-condition**: `0 ≤ self.current ≤ self.max`.
    #[inline]
    pub fn apply_damage(&mut self, amount: CoordinateUnit) -> CoordinateUnit {
        let actual = amount.min(self.current).max(0);
        self.current -= actual;
        actual
    }

    /// Heals by `amount`, clamping `current` to `[0, max]`.
    /// Returns the *actual* HP restored (may be less if near max).
    ///
    /// **Post-condition**: `0 ≤ self.current ≤ self.max`.
    #[inline]
    pub fn heal(&mut self, amount: CoordinateUnit) -> CoordinateUnit {
        let actual = amount.min(self.max - self.current).max(0);
        self.current += actual;
        actual
    }

    /// Returns `true` when the entity should be considered dead.
    #[inline]
    pub fn is_dead(&self) -> bool {
        self.current <= 0
    }

    /// Returns the health fraction in `[0.0, 1.0]`.
    /// Returns `0.0` if `max` is zero (avoids division by zero).
    #[inline]
    pub fn fraction(&self) -> f64 {
        if self.max <= 0 {
            return 0.0;
        }
        (self.current as f64 / self.max as f64).clamp(0.0, 1.0)
    }
}

/// Combat statistics used by the combat system to resolve attacks.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct CombatStats {
    pub attack: CoordinateUnit,
}

impl CombatStats {
    /// Computes the raw damage this attacker deals.
    #[inline]
    pub fn damage_against(&self) -> CoordinateUnit {
        self.attack.max(0)
    }
}

/// Display name for any entity. Used in combat messages, UI, and logs.
#[derive(Component, Clone, Debug)]
pub struct Name(pub String);

/// Returns the display string of an optional `Name`, falling back to `"???"`.
#[inline]
pub fn display_name(name: Option<&Name>) -> &str {
    name.map_or("???", |n| &n.0)
}

/// Returns the display string of an optional `Name`, falling back to `"item"`.
/// Used for item pickup/drop/use messages where "item" is the natural default.
#[inline]
pub fn item_display_name(name: Option<&Name>) -> &str {
    name.map_or("item", |n| &n.0)
}

/// Movement speed: determines how much energy an entity gains each world tick.
///
/// In the energy-based turn model, an entity acts when its accumulated energy
/// reaches `ACTION_COST`. Higher speed → more energy per tick → more frequent
/// actions. A speed of 100 is the "normal" baseline (one action per tick).
///
/// The energy model is a discrete event scheduler:
///   turns_between_actions = ⌈ACTION_COST / speed⌉
///
/// This is the standard roguelike scheduling algorithm used by Angband, DCSS,
/// and Cogmind. It avoids floating-point entirely and provides exact fairness.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Speed(pub CoordinateUnit);

/// Accumulated action energy. When `energy >= ACTION_COST`, the entity may act.
///
/// After acting, energy is reduced by `ACTION_COST`. Excess energy carries
/// over, ensuring long-run fairness: over N ticks, an entity with speed S
/// takes exactly ⌊N × S / ACTION_COST⌋ actions.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Energy(pub CoordinateUnit);

impl Energy {
    /// Returns `true` when this entity has accumulated enough energy to act.
    #[inline]
    pub fn can_act(&self) -> bool {
        self.0 >= ACTION_COST
    }

    /// Deducts `ACTION_COST`, leaving any excess for the next tick.
    ///
    /// **Pre-condition**: `self.can_act()` should be true.
    /// **Post-condition**: `self.0 = old − ACTION_COST`.
    #[inline]
    pub fn spend_action(&mut self) {
        self.0 -= ACTION_COST;
    }

    /// Accumulates energy from the entity's speed.
    ///
    /// This is one tick of the discrete energy scheduler:
    ///   `energy += speed`
    #[inline]
    pub fn accumulate(&mut self, speed: &Speed) {
        self.0 += speed.0;
    }
}

/// The energy threshold required to perform one action.
/// Entities accumulate energy each tick equal to their `Speed` value.
/// When energy ≥ ACTION_COST, they may act and energy is reduced by ACTION_COST.
pub const ACTION_COST: CoordinateUnit = 100;

/// Stamina cost for casting AoE grenades and molotov cocktails.
/// Note: sand throwing has its own separate cost (5 stamina).
pub const SPELL_STAMINA_COST: CoordinateUnit = 10;

/// AI behaviour state for non-player entities.
///
/// The AI system reads this to decide what action to emit:
/// - `Idle`: stand still, wait for the player to enter sight range.
/// - `Chasing`: move toward the last known player position.
/// - `Patrolling`: move along a patrol route or wander randomly.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum AiState {
    /// Entity is stationary — has not seen the player yet.
    Idle,
    /// Entity is actively pursuing the player.
    Chasing,
    /// Entity is patrolling — moving along a route or wandering.
    Patrolling,
    /// Entity is retreating — health is critical and no healing items available.
    Fleeing,
}

/// Patrol origin: the position this NPC considers "home". It will patrol
/// around this position when not chasing the player.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct PatrolOrigin(pub GridVec);

/// Memory component for NPC AI — remembers the last known target position.
/// When an NPC loses sight of its target, it navigates to the remembered
/// position before returning to patrol/idle.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct AiMemory {
    /// Last known position of the chase target.
    pub last_known_pos: Option<GridVec>,
    /// Turn number when the target was last seen.
    pub last_seen_turn: u32,
    /// Number of failed 360° search sweeps at the last-known position.
    /// After 2 failed sweeps the NPC gives up and returns to patrol.
    pub search_attempts: u8,
    /// Cursor steps taken since last fire (for blind-fire after 4 steps).
    pub cursor_steps: u8,
    /// Remaining cursor rotation steps while searching at a last-known position.
    pub search_rotation_steps: u8,
    /// Consecutive turns the NPC has been stationary (same tile).
    pub stationary_turns: u8,
    /// Previous position, used to detect stationarity.
    pub prev_pos: Option<GridVec>,
    /// Last movement direction taken by the NPC.
    pub last_move_dir: Option<GridVec>,
    /// Turn when `last_move_dir` was recorded.
    pub last_move_turn: u32,
}

impl Default for AiMemory {
    fn default() -> Self {
        Self {
            last_known_pos: None,
            last_seen_turn: 0,
            search_attempts: 0,
            cursor_steps: 0,
            search_rotation_steps: 0,
            stationary_turns: 0,
            prev_pos: None,
            last_move_dir: None,
            last_move_turn: 0,
        }
    }
}

/// Personality traits that modulate NPC AI behavior.
/// Different NPCs exhibit different combat styles based on these parameters.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct AiPersonality {
    /// How aggressively the NPC pursues targets (0.0 = passive, 1.0 = berserker).
    pub aggression: f64,
    /// How willing the NPC is to stay in fights when wounded (0.0 = coward, 1.0 = fearless).
    pub courage: f64,
}

impl Default for AiPersonality {
    fn default() -> Self {
        Self {
            aggression: 0.5,
            courage: 0.5,
        }
    }
}

/// Aiming style assigned randomly when an NPC acquires a target.
/// Determines how the NPC approaches ranged combat.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum AimingStyle {
    /// Takes extra turns tracking the cursor onto the target before firing.
    CarefulAim,
    /// Fires quickly with reduced accuracy.
    SnapShot,
    /// Fires vaguely in the target's direction without precise aim.
    Suppression,
}

/// Shared cursor component for aiming. Both the player and NPCs use this same
/// component. NPC AI drives the cursor position directly and may never center
/// it on their own tile.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    /// Current aim cursor position in world coordinates.
    pub pos: GridVec,
}

/// Persistent target tracking for NPC AI.
/// Once an NPC acquires a target, it pursues and attacks that target
/// until the target is dead or has been fully out of awareness range
/// for at least `TARGET_LOCK_TIMEOUT` turns with no new sightings.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct AiTarget {
    /// The entity being pursued.
    pub entity: Entity,
    /// Last known position of the target.
    pub last_pos: GridVec,
    /// Turn number when target was last seen.
    pub last_seen: u32,
    /// Hard lock: set when NPC has fired at or taken fire from this target.
    /// When locked, only death or `TARGET_LOCK_TIMEOUT` turns unseen can break it.
    pub locked: bool,
}

/// Extended awareness range during active pursuit.
/// Decays gradually back to baseline once the target has not been re-spotted.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct AiPursuitBoost {
    /// Extra awareness range (in tiles) added during pursuit.
    pub extra_range: i32,
    /// Turn when the target was last spotted (for decay calculation).
    pub last_spotted_turn: u32,
}

/// Faction affiliation for group-based spawning.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Faction {
    Wildlife,
    Outlaws,
    Lawmen,
    Vaqueros,
    /// Town civilians — shopkeepers, townsfolk. No allies.
    Civilians,
    /// Native American faction. No allies.
    Apache,
    /// Police officers and deputies. No allies.
    Police,
}

/// Bullet caliber for period-accurate cap-and-ball revolvers and rifles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Caliber {
    /// .31 caliber (Colt Pocket)
    Cal31,
    /// .36 caliber (Colt Navy, Colt Sheriff, Savage 1856)
    Cal36,
    /// .44 caliber (Colt Army, Remington New Model Army, Starr 1858, Adams)
    Cal44,
    /// .50 caliber (Hawken Rifle)
    Cal50,
    /// .58 caliber (Springfield Model 1855)
    Cal58,
    /// .577 caliber (Enfield Pattern 1853)
    Cal577,
    /// .69 caliber (Springfield Model 1842)
    Cal69,
}

impl Caliber {
    /// Returns the base damage for this caliber.
    /// Damage is equivalent to the caliber number: .31 = 31, .44 = 44, etc.
    /// For .577, damage is truncated to 57 (hundredths) to stay in scale.
    #[inline]
    pub fn damage(&self) -> i32 {
        match self {
            Caliber::Cal31 => 31,
            Caliber::Cal36 => 36,
            Caliber::Cal44 => 44,
            Caliber::Cal50 => 50,
            Caliber::Cal58 => 58,
            Caliber::Cal577 => 57,
            Caliber::Cal69 => 69,
        }
    }
}

impl std::fmt::Display for Caliber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Caliber::Cal31 => write!(f, ".31"),
            Caliber::Cal36 => write!(f, ".36"),
            Caliber::Cal44 => write!(f, ".44"),
            Caliber::Cal50 => write!(f, ".50"),
            Caliber::Cal58 => write!(f, ".58"),
            Caliber::Cal577 => write!(f, ".577"),
            Caliber::Cal69 => write!(f, ".69"),
        }
    }
}

/// Stamina pool for entities that can perform special actions.
/// Special actions consume stamina; stamina regenerates slowly each turn.
///
/// **Invariant**: `0 ≤ current ≤ max`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Stamina {
    pub current: CoordinateUnit,
    pub max: CoordinateUnit,
}

impl Stamina {
    /// Attempts to spend `cost` stamina. Returns `true` and deducts if
    /// sufficient stamina is available, `false` otherwise (no mutation).
    ///
    /// **Post-condition**: if `true`, `self.current` decreased by `cost`.
    #[inline]
    pub fn spend(&mut self, cost: CoordinateUnit) -> bool {
        if self.current >= cost {
            self.current -= cost;
            true
        } else {
            false
        }
    }

    /// Recovers `amount` stamina, clamped to `max`.
    ///
    /// **Post-condition**: `self.current ≤ self.max`.
    #[inline]
    pub fn recover(&mut self, amount: CoordinateUnit) {
        self.current = (self.current + amount).min(self.max);
    }
}

/// Visual style for projectile rendering.
/// The animation system is generic over visual: each projectile type carries
/// its own `ProjectileVisual` so the renderer can pick the correct symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectileVisual {
    /// Bullets and arrows: center dot head with a trailing dot tail.
    BulletTrail,
    /// Explosion fragments: hot spark / shard visuals distinct from normal shots.
    ShrapnelTrail,
    /// Tomahawks and knives: spinning slashes and dashes (`/`, `—`, `\`, `|`).
    SpinningBlade,
    /// Everything else (dynamite, molotov, generic): asterisk (`*`).
    Asterisk,
}

/// A projectile entity that travels along a path over ticks.
/// Unified animation system: bullets, shrapnel, arrows, thrown knives/tomahawks,
/// and explosives all use this component with tunable speed and visual style.
/// Each tick the projectile advances `tiles_per_tick` steps along its precomputed
/// Bresenham path. When it reaches a hostile entity, it applies damage and despawns.
///
/// Each tick the projectile advances one tile every 10ms along its path,
/// applying damage and collision at each step.
#[derive(Component, Debug)]
pub struct Projectile {
    /// Precomputed path tiles (Bresenham line from origin to endpoint).
    pub path: Vec<GridVec>,
    /// Current index along the path (game-logic and display combined).
    pub path_index: usize,
    /// Number of tiles the projectile advances per tick.
    pub tiles_per_tick: usize,
    /// Damage to apply on hit.
    pub damage: i32,
    /// Remaining penetration power.
    pub penetration: i32,
    /// Entity that fired the projectile (to avoid self-damage for bullets).
    pub source: Entity,
    /// Previous display position for rendering a trailing tail.
    pub tail_pos: Option<GridVec>,
    /// Visual style used by the renderer (dots, spinning slashes, asterisk).
    pub visual: ProjectileVisual,
    /// Whether this is a firearm bullet (uses hit-chance / headshot rolls).
    pub is_bullet: bool,
    /// Accumulated real time (seconds) since the last tile step.
    pub tile_timer: f32,
}

/// A thrown explosive (dynamite or molotov) traveling through the air.
/// When this projectile hits something (entity, wall) or reaches its target,
/// it detonates, spawning the appropriate explosion/fire effect.
#[derive(Component, Debug)]
pub enum ThrownExplosive {
    /// Dynamite: spawns shrapnel and environmental destruction on detonation.
    Dynamite {
        damage: i32,
        radius: i32,
        grenade_index: usize,
    },
    /// Molotov: sets area on fire and generates smoke on detonation.
    Molotov {
        damage: i32,
        radius: i32,
        item_index: usize,
    },
}

// ─── Inventory & Item system ─────────────────────────────────────

/// Marker component: tags an entity as an item that can be picked up.
#[derive(Component, Debug)]
pub struct Item;

/// The kind of item and its associated effect.
#[derive(Component, Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// A cap-and-ball revolver. Tracks loaded rounds and caliber.
    Gun {
        loaded: i32,
        capacity: i32,
        caliber: Caliber,
        attack: i32,
        name: String,
        /// Blunt/thrown damage when used as a melee weapon or thrown.
        blunt_damage: i32,
    },
    /// A throwing knife. Can be recovered after landing.
    Knife { attack: i32, blunt_damage: i32 },
    /// A throwing tomahawk. Can be recovered after landing.
    Tomahawk { attack: i32, blunt_damage: i32 },
    /// A grenade (dynamite stick). Deals area damage.
    Grenade {
        damage: i32,
        radius: i32,
        blunt_damage: i32,
    },
    /// Whiskey bottle. Restores health when consumed.
    Whiskey { heal: i32, blunt_damage: i32 },
    /// A molotov cocktail. Thrown toward cursor; sets a large area on fire.
    Molotov {
        damage: i32,
        radius: i32,
        blunt_damage: i32,
    },
    /// A bow. Fires arrows. Used by Apache.
    Bow { attack: i32, blunt_damage: i32 },
    /// Beer. Restores a small amount of health when consumed.
    Beer { heal: i32, blunt_damage: i32 },
    /// Ale. Restores health when consumed.
    Ale { heal: i32, blunt_damage: i32 },
    /// Stout. Restores a moderate amount of health when consumed.
    Stout { heal: i32, blunt_damage: i32 },
    /// Wine. Restores health when consumed.
    Wine { heal: i32, blunt_damage: i32 },
    /// Rum. Restores a large amount of health when consumed.
    Rum { heal: i32, blunt_damage: i32 },
}

impl ItemKind {
    /// Returns the blunt/thrown damage for this item.
    pub fn blunt_damage(&self) -> i32 {
        match self {
            ItemKind::Gun { blunt_damage, .. } => *blunt_damage,
            ItemKind::Knife { blunt_damage, .. } => *blunt_damage,
            ItemKind::Tomahawk { blunt_damage, .. } => *blunt_damage,
            ItemKind::Grenade { blunt_damage, .. } => *blunt_damage,
            ItemKind::Whiskey { blunt_damage, .. } => *blunt_damage,
            ItemKind::Molotov { blunt_damage, .. } => *blunt_damage,
            ItemKind::Bow { blunt_damage, .. } => *blunt_damage,
            ItemKind::Beer { blunt_damage, .. } => *blunt_damage,
            ItemKind::Ale { blunt_damage, .. } => *blunt_damage,
            ItemKind::Stout { blunt_damage, .. } => *blunt_damage,
            ItemKind::Wine { blunt_damage, .. } => *blunt_damage,
            ItemKind::Rum { blunt_damage, .. } => *blunt_damage,
        }
    }

    /// Returns a human-readable display name for this item kind.
    pub fn display_name(&self) -> String {
        match self {
            ItemKind::Gun { name, .. } => name.clone(),
            ItemKind::Knife { .. } => "Knife".into(),
            ItemKind::Tomahawk { .. } => "Tomahawk".into(),
            ItemKind::Grenade { .. } => "Dynamite".into(),
            ItemKind::Whiskey { .. } => "Whiskey".into(),
            ItemKind::Molotov { .. } => "Molotov".into(),
            ItemKind::Bow { .. } => "Bow".into(),
            ItemKind::Beer { .. } => "Beer".into(),
            ItemKind::Ale { .. } => "Ale".into(),
            ItemKind::Stout { .. } => "Stout".into(),
            ItemKind::Wine { .. } => "Wine".into(),
            ItemKind::Rum { .. } => "Rum".into(),
        }
    }
}

/// Attached to a projectile that carries a thrown item (knife/tomahawk).
/// When the projectile lands or hits, the item entity is placed at the
/// landing position with a `Thrown` marker so it can be recovered.
#[derive(Component, Debug)]
pub struct ThrownItemProjectile {
    /// The item entity being thrown.
    pub item_entity: Entity,
}

/// Inventory component: holds item entities belonging to an entity.
#[derive(Component, Debug, Default)]
pub struct Inventory {
    pub items: Vec<Entity>,
}

impl Inventory {
    /// Removes and returns the item at `index`, or `None` if out of bounds.
    #[inline]
    pub fn remove_at(&mut self, index: usize) -> Option<Entity> {
        if index < self.items.len() {
            Some(self.items.remove(index))
        } else {
            None
        }
    }
}

/// Marker component: when this entity dies, it may drop items.
/// The death system uses noise-based probability to determine drops.
#[derive(Component, Debug)]
pub struct LootTable;

/// Type of collectible supply drop.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum CollectibleKind {
    Caps(i32),
    Arrows(i32),
    Bullets31(i32),
    Bullets36(i32),
    Bullets44(i32),
    Bullets50(i32),
    Bullets58(i32),
    Bullets577(i32),
    Bullets69(i32),
    Powder(i32),
}

/// Tracks which entity last dealt damage to this entity.
/// Used to attribute the killing blow for kill counting.
#[derive(Component, Clone, Copy, Debug)]
pub struct LastDamageSource(pub Entity);

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Position / GridVec conversion tests ─────────────────────

    #[test]
    fn position_as_grid_vec() {
        let pos = Position { x: 10, y: 20 };
        let gv = pos.as_grid_vec();
        assert_eq!(gv, GridVec::new(10, 20));
    }

    #[test]
    fn position_from_grid_vec() {
        let gv = GridVec::new(5, -3);
        let pos = Position::from(gv);
        assert_eq!(pos.x, 5);
        assert_eq!(pos.y, -3);
    }

    #[test]
    fn grid_vec_from_position() {
        let pos = Position { x: 7, y: 13 };
        let gv: GridVec = pos.into();
        assert_eq!(gv, GridVec::new(7, 13));
    }

    #[test]
    fn position_round_trip() {
        let original = Position { x: -42, y: 99 };
        let gv = original.as_grid_vec();
        let back = Position::from(gv);
        assert_eq!(original, back);
    }

    // ─── Health tests ────────────────────────────────────────────

    #[test]
    fn health_full() {
        let h = Health {
            current: 30,
            max: 30,
        };
        assert_eq!(h.current, h.max);
    }

    #[test]
    fn health_damage_reduces_current() {
        let mut h = Health {
            current: 30,
            max: 30,
        };
        let actual = h.apply_damage(5);
        assert_eq!(actual, 5);
        assert_eq!(h.current, 25);
        assert_eq!(h.max, 30);
    }

    #[test]
    fn health_damage_clamps_to_zero() {
        let mut h = Health {
            current: 3,
            max: 30,
        };
        let actual = h.apply_damage(10);
        assert_eq!(actual, 3, "Actual damage should be clamped to current HP");
        assert_eq!(h.current, 0);
    }

    #[test]
    fn health_apply_damage_returns_actual() {
        let mut h = Health {
            current: 5,
            max: 30,
        };
        assert_eq!(h.apply_damage(3), 3);
        assert_eq!(h.current, 2);
        assert_eq!(h.apply_damage(10), 2);
        assert_eq!(h.current, 0);
    }

    #[test]
    fn health_heal_clamps_to_max() {
        let mut h = Health {
            current: 25,
            max: 30,
        };
        let healed = h.heal(10);
        assert_eq!(healed, 5, "Should only heal the deficit");
        assert_eq!(h.current, 30);
    }

    #[test]
    fn health_heal_returns_actual() {
        let mut h = Health {
            current: 20,
            max: 30,
        };
        assert_eq!(h.heal(5), 5);
        assert_eq!(h.current, 25);
    }

    #[test]
    fn health_heal_at_full_returns_zero() {
        let mut h = Health {
            current: 30,
            max: 30,
        };
        assert_eq!(h.heal(5), 0);
        assert_eq!(h.current, 30);
    }

    #[test]
    fn health_is_dead() {
        assert!(
            Health {
                current: 0,
                max: 30
            }
            .is_dead()
        );
        assert!(
            !Health {
                current: 1,
                max: 30
            }
            .is_dead()
        );
    }

    #[test]
    fn health_fraction() {
        let h = Health {
            current: 15,
            max: 30,
        };
        assert!((h.fraction() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn health_fraction_zero_max() {
        let h = Health { current: 0, max: 0 };
        assert_eq!(h.fraction(), 0.0);
    }

    #[test]
    fn health_invariant_maintained() {
        let mut h = Health {
            current: 10,
            max: 30,
        };
        h.apply_damage(100);
        assert!(h.current >= 0 && h.current <= h.max);
        h.heal(100);
        assert!(h.current >= 0 && h.current <= h.max);
    }

    // ─── CombatStats damage formula tests ────────────────────────

    #[test]
    fn damage_formula_positive() {
        let attacker = CombatStats { attack: 5 };
        assert_eq!(attacker.damage_against(), 5);
    }

    #[test]
    fn damage_formula_equals_attack() {
        let attacker = CombatStats { attack: 3 };
        assert_eq!(attacker.damage_against(), 3);
    }

    #[test]
    fn damage_formula_zero_attack() {
        let attacker = CombatStats { attack: 2 };
        assert_eq!(attacker.damage_against(), 2);
    }

    #[test]
    fn damage_against_non_negative() {
        // Property: ∀ atk: damage_against(atk) ≥ 0
        for atk in 0..20 {
            assert!(CombatStats { attack: atk }.damage_against() >= 0);
        }
    }

    #[test]
    fn damage_against_monotone_in_attack() {
        // Property: atk₁ ≤ atk₂ ⟹ damage(atk₁) ≤ damage(atk₂)
        for atk in 0..19 {
            let d1 = CombatStats { attack: atk }.damage_against();
            let d2 = CombatStats { attack: atk + 1 }.damage_against();
            assert!(d1 <= d2);
        }
    }

    #[test]
    fn damage_against_zero_only_for_zero_attack() {
        // damage = max(0, atk), so it's 0 only when atk <= 0
        for atk in 0..15 {
            let d = CombatStats { attack: atk }.damage_against();
            if atk <= 0 {
                assert_eq!(d, 0);
            } else {
                assert!(d > 0);
            }
        }
    }

    // ─── Energy / Speed tests ────────────────────────────────────

    #[test]
    fn action_cost_is_100() {
        assert_eq!(ACTION_COST, 100);
    }

    #[test]
    fn energy_accumulation_normal_speed() {
        let speed = Speed(100);
        let mut energy = Energy(0);
        energy.accumulate(&speed);
        assert_eq!(energy.0, 100);
        assert!(energy.can_act());
    }

    #[test]
    fn energy_accumulation_slow_speed() {
        let speed = Speed(50);
        let mut energy = Energy(0);
        // After 1 tick: not enough to act
        energy.accumulate(&speed);
        assert_eq!(energy.0, 50);
        assert!(!energy.can_act());
        // After 2 ticks: enough to act
        energy.accumulate(&speed);
        assert_eq!(energy.0, 100);
        assert!(energy.can_act());
    }

    #[test]
    fn energy_accumulation_fast_speed() {
        let speed = Speed(200);
        let mut energy = Energy(0);
        energy.accumulate(&speed);
        assert_eq!(energy.0, 200);
        // Fast entity can act twice
        assert!(energy.can_act());
        energy.spend_action();
        assert!(energy.can_act());
    }

    #[test]
    fn energy_deduction_leaves_excess() {
        let speed = Speed(120);
        let mut energy = Energy(0);
        energy.accumulate(&speed);
        energy.spend_action();
        assert_eq!(energy.0, 20); // Excess carries over
    }

    #[test]
    fn energy_scheduling_fairness() {
        // Property: over N ticks, entity with speed S takes
        // exactly ⌊N × S / ACTION_COST⌋ actions.
        for &spd in &[50, 75, 100, 120, 150, 200] {
            let speed = Speed(spd);
            let mut energy = Energy(0);
            let mut actions = 0i64;
            let n = 100;
            for _ in 0..n {
                energy.accumulate(&speed);
                while energy.can_act() {
                    energy.spend_action();
                    actions += 1;
                }
            }
            let expected = (n as i64 * spd as i64) / ACTION_COST as i64;
            assert_eq!(
                actions, expected,
                "Speed {spd}: expected {expected} actions over {n} ticks, got {actions}"
            );
        }
    }

    // ─── AiState tests ──────────────────────────────────────────

    #[test]
    fn ai_state_idle_default() {
        let state = AiState::Idle;
        assert_eq!(state, AiState::Idle);
    }

    #[test]
    fn ai_state_transitions() {
        let state = AiState::Idle;
        assert_eq!(state, AiState::Idle);
        let state = AiState::Chasing;
        assert_eq!(state, AiState::Chasing);
    }

    // ─── Viewshed tests ─────────────────────────────────────────

    #[test]
    fn viewshed_dirty_flag() {
        let mut vs = Viewshed {
            range: 10,
            visible_tiles: HashSet::new(),
            revealed_tiles: HashSet::new(),
            dirty: true,
        };
        assert!(vs.dirty);
        vs.dirty = false;
        assert!(!vs.dirty);
    }

    #[test]
    fn viewshed_visible_tiles_insert() {
        let mut vs = Viewshed {
            range: 10,
            visible_tiles: HashSet::new(),
            revealed_tiles: HashSet::new(),
            dirty: true,
        };
        let point = GridVec::new(5, 5);
        vs.visible_tiles.insert(point);
        assert!(vs.visible_tiles.contains(&point));
    }

    #[test]
    fn viewshed_revealed_accumulates() {
        let mut vs = Viewshed {
            range: 10,
            visible_tiles: HashSet::new(),
            revealed_tiles: HashSet::new(),
            dirty: true,
        };
        let p1 = GridVec::new(1, 1);
        let p2 = GridVec::new(2, 2);
        vs.revealed_tiles.insert(p1);
        vs.revealed_tiles.insert(p2);
        // Clearing visible doesn't affect revealed
        vs.visible_tiles.clear();
        assert!(vs.revealed_tiles.contains(&p1));
        assert!(vs.revealed_tiles.contains(&p2));
    }

    // ─── Caliber display tests ──────────────────────────────────────

    #[test]
    fn caliber_display_formatting() {
        assert_eq!(format!("{}", Caliber::Cal31), ".31");
        assert_eq!(format!("{}", Caliber::Cal36), ".36");
        assert_eq!(format!("{}", Caliber::Cal44), ".44");
    }

    // ─── Gun ItemKind tests ─────────────────────────────────────────

    #[test]
    fn gun_loaded_rounds_decrement() {
        let mut gun = ItemKind::Gun {
            loaded: 6,
            capacity: 6,
            caliber: Caliber::Cal36,
            attack: 5,
            name: "Test Gun".into(),
            blunt_damage: 5,
        };
        if let ItemKind::Gun { ref mut loaded, .. } = gun {
            *loaded -= 1;
            assert_eq!(*loaded, 5);
        }
    }

    #[test]
    fn gun_cannot_fire_when_empty() {
        let gun = ItemKind::Gun {
            loaded: 0,
            capacity: 6,
            caliber: Caliber::Cal36,
            attack: 5,
            name: "Test Gun".into(),
            blunt_damage: 5,
        };
        if let ItemKind::Gun { loaded, .. } = gun {
            assert_eq!(loaded, 0);
            assert!(loaded <= 0, "Gun should not be able to fire");
        }
    }

    // ─── Stamina pool tests ─────────────────────────────────────────

    #[test]
    fn stamina_spend_success() {
        let mut s = Stamina {
            current: 50,
            max: 50,
        };
        assert!(s.spend(10));
        assert_eq!(s.current, 40);
    }

    #[test]
    fn stamina_spend_insufficient() {
        let mut s = Stamina {
            current: 5,
            max: 50,
        };
        assert!(!s.spend(10));
        assert_eq!(s.current, 5, "Should not mutate on failed spend");
    }

    #[test]
    fn stamina_spend_exact() {
        let mut s = Stamina {
            current: 10,
            max: 50,
        };
        assert!(s.spend(10));
        assert_eq!(s.current, 0);
    }

    #[test]
    fn stamina_recover_clamps_to_max() {
        let mut s = Stamina {
            current: 45,
            max: 50,
        };
        s.recover(10);
        assert_eq!(s.current, 50);
    }

    #[test]
    fn stamina_recover_partial() {
        let mut s = Stamina {
            current: 30,
            max: 50,
        };
        s.recover(5);
        assert_eq!(s.current, 35);
    }

    #[test]
    fn stamina_invariant_maintained() {
        let mut s = Stamina {
            current: 25,
            max: 50,
        };
        s.spend(100); // Should fail (not enough), no mutation
        assert!(s.current >= 0 && s.current <= s.max);
        s.recover(100);
        assert!(s.current >= 0 && s.current <= s.max);
    }
}
