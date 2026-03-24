use bevy::prelude::*;

use crate::typedefs::CoordinateUnit;

/// Fired when an entity intends to move by a relative offset.
#[derive(Message, Debug)]
pub struct MoveIntent {
    pub entity: Entity,
    pub dx: CoordinateUnit,
    pub dy: CoordinateUnit,
}

/// Fired when an entity intends to attack another entity.
#[derive(Message, Debug)]
pub struct AttackIntent {
    pub attacker: Entity,
    pub target: Entity,
}

/// Fired after damage has been resolved and applied to an entity.
#[derive(Message, Debug)]
pub struct DamageEvent {
    pub target: Entity,
    pub amount: CoordinateUnit,
    /// Entity that dealt this damage. Used to attribute kills for EXP.
    pub source: Option<Entity>,
}

/// Fired when the player throws a grenade (area-of-effect attack).
/// Damages all hostile entities within `radius` tiles of the target.
#[derive(Message, Debug)]
pub struct SpellCastIntent {
    pub caster: Entity,
    pub radius: CoordinateUnit,
    /// World position where the grenade detonates (cursor position).
    pub target: crate::grid_vec::GridVec,
    /// Inventory index of the grenade item to consume.
    pub grenade_index: usize,
}

/// Fired when an entity wants to use an inventory item.
#[derive(Message, Debug)]
pub struct UseItemIntent {
    pub user: Entity,
    pub item_index: usize,
}

/// Fired when an entity wants to pick up an item on the ground.
#[derive(Message, Debug)]
pub struct PickupItemIntent {
    pub picker: Entity,
}

/// Fired when the player uses a targeted ranged attack in a chosen direction.
/// The bullet travels along (dx, dy) direction for up to `range` tiles.
#[derive(Message, Debug)]
pub struct RangedAttackIntent {
    pub attacker: Entity,
    pub range: CoordinateUnit,
    /// Trajectory direction (normalized to -1/0/1 per axis).
    pub dx: CoordinateUnit,
    pub dy: CoordinateUnit,
    /// Optional gun item entity. If present, decrements its loaded rounds.
    pub gun_item: Option<Entity>,
}

/// Fired when an AI entity performs a ranged attack toward a target position.
#[derive(Message, Debug)]
pub struct AiRangedAttackIntent {
    pub attacker: Entity,
    pub target: Entity,
    pub range: CoordinateUnit,
}

/// Fired when the player performs a melee wide (cleave) attack hitting all adjacent enemies.
#[derive(Message, Debug)]
pub struct MeleeWideIntent {
    pub attacker: Entity,
}

/// Fired when the player throws a knife or tomahawk toward the cursor.
#[derive(Message, Debug)]
pub struct ThrowItemIntent {
    pub thrower: Entity,
    pub item_entity: Entity,
    pub item_index: usize,
    pub dx: CoordinateUnit,
    pub dy: CoordinateUnit,
    pub range: CoordinateUnit,
    pub damage: CoordinateUnit,
}

/// Fired when the player throws a molotov cocktail.
/// Ignites all flammable props within `radius` tiles of the target.
#[derive(Message, Debug)]
pub struct MolotovCastIntent {
    pub caster: Entity,
    pub radius: CoordinateUnit,
    pub damage: CoordinateUnit,
    /// World position where the molotov detonates.
    pub target: crate::grid_vec::GridVec,
    /// Inventory index of the molotov item to consume.
    pub item_index: usize,
}
