use bevy::prelude::*;

use crate::components::{
    CollectibleKind, Dead, Health, Inventory, Item, ItemKind, Name, PlayerControlled, Position,
    Stamina, item_display_name,
};
use crate::events::{PickupItemIntent, ThrowItemIntent, UseItemIntent};
use crate::grid_vec::GridVec;
use crate::resources::{Collectibles, CombatLog, InputState, SpatialIndex};
use crate::systems::input::THROW_ITEM_STAMINA_COST;

/// Maximum inventory capacity (unified for players and NPCs).
pub const MAX_INVENTORY_SLOTS: usize = 9;

/// Processes pickup intents: any entity picks up an item on the ground at their position.
/// Works for both the player and NPCs — the picker entity is taken from the intent.
pub fn pickup_system(
    mut intents: MessageReader<PickupItemIntent>,
    mut commands: Commands,
    position_query: Query<&Position>,
    items_query: Query<(Entity, &Position, Option<&Name>), With<Item>>,
    spatial: Res<SpatialIndex>,
    mut inventory_query: Query<&mut Inventory>,
    mut combat_log: ResMut<CombatLog>,
) {
    for intent in intents.read() {
        let Ok(picker_pos) = position_query.get(intent.picker) else {
            continue;
        };
        let picker_vec = picker_pos.as_grid_vec();

        // Find items at the picker's position using the spatial index.
        let entities_here = spatial.entities_at(&picker_vec);
        let mut picked_up = false;

        for &ent in entities_here {
            if let Ok((_, _, name)) = items_query.get(ent) {
                let item_name = item_display_name(name);

                // Add to inventory.
                if let Ok(mut inv) = inventory_query.get_mut(intent.picker) {
                    if inv.items.len() < MAX_INVENTORY_SLOTS {
                        // Remove position so it's no longer on the map.
                        commands.entity(ent).remove::<Position>();
                        inv.items.push(ent);
                        combat_log.push(format!("Picked up {item_name}"));
                        picked_up = true;
                        break; // Pick up one item at a time.
                    } else {
                        combat_log.push("Inventory full!".into());
                    }
                }
            }
        }

        if !picked_up {
            // Silently ignore — no message when there's nothing to pick up.
        }
    }
}

/// Processes use-item intents: consumes an item from any entity's inventory.
/// Works for both the player and NPCs — the user entity is taken from the intent.
pub fn use_item_system(
    mut intents: MessageReader<UseItemIntent>,
    mut commands: Commands,
    mut inventory_query: Query<&mut Inventory>,
    mut health_query: Query<&mut Health>,
    mut item_kind_query: Query<(&mut ItemKind, Option<&Name>)>,
    user_names: Query<Option<&Name>>,
    mut combat_log: ResMut<CombatLog>,
    mut collectibles: ResMut<Collectibles>,
) {
    for intent in intents.read() {
        let Ok(mut inv) = inventory_query.get_mut(intent.user) else {
            continue;
        };

        let Some(&item_entity) = inv.items.get(intent.item_index) else {
            combat_log.push("No item in that slot.".into());
            continue;
        };

        let Ok((kind, name)) = item_kind_query.get(item_entity) else {
            combat_log.push("Invalid item.".into());
            continue;
        };

        let item_name = item_display_name(name);
        let user_name = user_names
            .get(intent.user)
            .ok()
            .flatten()
            .map(|name| name.0.as_str())
            .unwrap_or("Someone");

        // Dereference Bevy's `Mut<ItemKind>` wrapper to pattern match.
        // This borrows the inner value immutably first; if we need to mutate
        // (e.g. increment loaded rounds), we call `get_mut` on a second query.
        match kind {
            ItemKind::Whiskey { heal, .. }
            | ItemKind::Beer { heal, .. }
            | ItemKind::Ale { heal, .. }
            | ItemKind::Stout { heal, .. }
            | ItemKind::Wine { heal, .. }
            | ItemKind::Rum { heal, .. } => {
                let heal = *heal;
                if let Ok(mut hp) = health_query.get_mut(intent.user) {
                    let healed = hp.heal(heal);
                    combat_log.push(format!("{user_name} used {item_name}, healed {healed} HP"));
                }
                inv.remove_at(intent.item_index);
                commands.entity(item_entity).despawn();
            }
            ItemKind::Grenade { .. } => {
                combat_log.push(format!("Used {item_name}!"));
                inv.remove_at(intent.item_index);
                commands.entity(item_entity).despawn();
            }
            ItemKind::Gun {
                loaded,
                capacity,
                caliber,
                name: gun_name,
                ..
            } => {
                let gun_name = gun_name.clone();
                let caliber = *caliber;
                let loaded = *loaded;
                let capacity = *capacity;

                if loaded >= capacity {
                    combat_log.push(format!(
                        "{gun_name} is fully loaded ({capacity}/{capacity})"
                    ));
                } else if collectibles.can_reload(caliber) {
                    collectibles.consume_reload(caliber);
                    if let Ok((mut kind_mut, _)) = item_kind_query.get_mut(item_entity)
                        && let ItemKind::Gun { ref mut loaded, .. } = *kind_mut
                    {
                        *loaded += 1;
                        combat_log.push(format!(
                            "Loaded 1 round into {gun_name} ({}/{capacity})",
                            *loaded
                        ));
                    }
                } else {
                    combat_log.push(format!(
                        "Need: 1 {caliber} bullet, 1 cap, 1 powder to reload {gun_name}"
                    ));
                }
            }
            ItemKind::Knife { .. } => {
                combat_log.push("Readied knife".into());
            }
            ItemKind::Tomahawk { .. } => {
                combat_log.push("Readied tomahawk".into());
            }
            ItemKind::Molotov { .. } => {
                combat_log.push("Readied molotov — aim and press slot key to throw".into());
            }
            ItemKind::Bow { .. } => {}
        }
    }
}

/// Reload system: finds the first gun in inventory that is not fully loaded
/// and can be reloaded with available collectibles, then reloads one round.
/// If the first gun's caliber is unavailable, tries other guns before failing.
pub fn reload_system(
    player_query: Query<&Inventory, With<PlayerControlled>>,
    mut item_kind_query: Query<(&mut ItemKind, Option<&Name>)>,
    mut combat_log: ResMut<CombatLog>,
    mut input_state: ResMut<InputState>,
    mut collectibles: ResMut<Collectibles>,
) {
    if !input_state.reload_pending {
        return;
    }
    input_state.reload_pending = false;

    let Ok(inv) = player_query.single() else {
        return;
    };

    // Collect all guns that are not fully loaded.
    let gun_entities: Vec<Entity> = inv
        .items
        .iter()
        .copied()
        .filter(|&ent| {
            item_kind_query.get(ent).ok().is_some_and(
                |(k, _)| matches!(k, ItemKind::Gun { loaded, capacity, .. } if *loaded < *capacity),
            )
        })
        .collect();

    if gun_entities.is_empty() {
        combat_log.push("No guns need reloading.".into());
        return;
    }

    // Try each gun in order until we find one we can reload.
    for gun_ent in &gun_entities {
        let (caliber, gun_name) = {
            let Ok((ref kind, _)) = item_kind_query.get(*gun_ent) else {
                continue;
            };
            if let ItemKind::Gun { caliber, name, .. } = kind {
                (*caliber, name.clone())
            } else {
                continue;
            }
        };

        if collectibles.can_reload(caliber) {
            collectibles.consume_reload(caliber);
            if let Ok((mut kind_mut, _)) = item_kind_query.get_mut(*gun_ent)
                && let ItemKind::Gun {
                    ref mut loaded,
                    capacity,
                    ..
                } = *kind_mut
            {
                *loaded += 1;
                combat_log.push(format!(
                    "Loaded 1 round into {gun_name} ({}/{capacity})",
                    *loaded
                ));
            }
            return;
        }
    }

    // No gun could be reloaded — report the first gun's requirements.
    let first_gun = gun_entities[0];
    if let Ok((ref kind, _)) = item_kind_query.get(first_gun)
        && let ItemKind::Gun { caliber, name, .. } = kind
    {
        combat_log.push(format!(
            "Need: 1 {caliber} bullet, 1 cap, 1 powder to reload {name}"
        ));
    }
}

/// Auto-pickup system: automatically picks up any item when the player walks
/// over it. Runs after movement.
pub fn auto_pickup_system(
    mut commands: Commands,
    player_query: Query<&Position, (With<PlayerControlled>, Without<Dead>)>,
    items_query: Query<(Entity, &Position, Option<&Name>, Option<&CollectibleKind>), With<Item>>,
    spatial: Res<SpatialIndex>,
    mut inventory_query: Query<&mut Inventory, (With<PlayerControlled>, Without<Dead>)>,
    mut combat_log: ResMut<CombatLog>,
    mut collectibles: ResMut<Collectibles>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let player_vec = player_pos.as_grid_vec();

    let entities_here = spatial.entities_at(&player_vec);

    for &ent in entities_here {
        let Ok((item_entity, _pos, item_name, coll_kind)) = items_query.get(ent) else {
            continue;
        };

        let name_str = item_display_name(item_name);

        // Handle collectible items: add to Collectibles resource instead of inventory.
        if let Some(kind) = coll_kind {
            collectibles.collect(*kind);
            combat_log.push(format!("Picked up {name_str}"));
            commands.entity(item_entity).despawn();
            continue;
        }

        if let Ok(mut inv) = inventory_query.single_mut()
            && inv.items.len() < MAX_INVENTORY_SLOTS
        {
            commands.entity(item_entity).remove::<Position>();
            inv.items.push(item_entity);
            combat_log.push(format!("Picked up {name_str}"));
        }
    }
}

/// Processes throw-item intents: removes a knife/tomahawk from inventory and
/// spawns a projectile that travels toward the target using the unified
/// animation system. Damage is applied when the projectile hits a hostile,
/// and the item is placed at the landing position with a Thrown marker.
pub fn throw_system(
    mut intents: MessageReader<ThrowItemIntent>,
    mut commands: Commands,
    mut inventory_query: Query<(&mut Inventory, &Position, Option<&mut Stamina>), With<PlayerControlled>>,
    mut combat_log: ResMut<CombatLog>,
    name_query: Query<Option<&Name>>,
) {
    for intent in intents.read() {
        let Ok((mut inv, player_pos, stamina)) = inventory_query.single_mut() else {
            continue;
        };

        if let Some(mut stamina) = stamina
            && !stamina.spend(THROW_ITEM_STAMINA_COST)
        {
            combat_log.push("Not enough stamina to throw!".into());
            continue;
        }

        // Remove from inventory
        if let Some(idx) = inv.items.iter().position(|&e| e == intent.item_entity) {
            inv.items.remove(idx);
        } else {
            continue;
        }

        let item_name = item_display_name(name_query.get(intent.item_entity).ok().flatten());

        let origin = player_pos.as_grid_vec();
        let endpoint = origin + GridVec::new(intent.dx * intent.range, intent.dy * intent.range);

        combat_log.push(format!("Threw {item_name}!"));

        crate::systems::projectile::spawn_thrown_projectile(
            &mut commands,
            origin,
            endpoint,
            intent.damage,
            intent.thrower,
            intent.item_entity,
        );
    }
}
