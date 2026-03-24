use bevy::{ecs::system::SystemParam, prelude::*};
#[cfg(not(feature = "windowed"))]
use bevy_ratatui::event::KeyMessage;
#[cfg(not(feature = "windowed"))]
use ratatui::crossterm::event::KeyCode;
use crate::components::{
    Dead, Faction, Health, Inventory, ItemKind, PlayerControlled, Position, SPELL_STAMINA_COST,
    Stamina, Viewshed,
};
use crate::events::{
    MeleeWideIntent, MolotovCastIntent, MoveIntent, RangedAttackIntent, SpellCastIntent,
    ThrowItemIntent, UseItemIntent,
};
use crate::resources::{
    CombatLog, CursorPosition, DynamicRng, ExtraWorldTicks, GameState, InputMode, InputState,
    MapSeed, RestartRequested, SpectatingAfterDeath, TurnState, WindowedKeyRepeat,
};

#[cfg(feature = "windowed")]
use bevy::input::keyboard::KeyCode;

/// Bundles all intent MessageWriters to stay under Bevy's 16-param system limit.
#[derive(SystemParam)]
pub struct IntentWriters<'w> {
    move_intents: MessageWriter<'w, MoveIntent>,
    spell_intents: MessageWriter<'w, SpellCastIntent>,
    molotov_intents: MessageWriter<'w, MolotovCastIntent>,
    use_item_intents: MessageWriter<'w, UseItemIntent>,
    ranged_intents: MessageWriter<'w, RangedAttackIntent>,
    melee_wide_intents: MessageWriter<'w, MeleeWideIntent>,
    throw_item_intents: MessageWriter<'w, ThrowItemIntent>,
}

#[derive(SystemParam)]
pub(crate) struct InputResources<'w> {
    game_state: Res<'w, State<GameState>>,
    next_game_state: ResMut<'w, NextState<GameState>>,
    turn_state: Option<Res<'w, State<TurnState>>>,
    next_turn_state: Option<ResMut<'w, NextState<TurnState>>>,
    combat_log: ResMut<'w, CombatLog>,
    input_state: ResMut<'w, InputState>,
    restart_requested: ResMut<'w, RestartRequested>,
    cursor: ResMut<'w, CursorPosition>,
    extra_world_ticks: ResMut<'w, ExtraWorldTicks>,
    spectating: ResMut<'w, SpectatingAfterDeath>,
    dynamic_rng: Res<'w, DynamicRng>,
    seed: Res<'w, MapSeed>,
    god_mode: ResMut<'w, crate::resources::GodMode>,
}

#[cfg(feature = "windowed")]
#[derive(SystemParam)]
pub(crate) struct WindowedInputRepeat<'w> {
    time: Res<'w, Time>,
    key_repeat: ResMut<'w, WindowedKeyRepeat>,
}

/// Default radius for the player's grenade blast.
const SPELL_RADIUS: i32 = 3;

/// Range for the targeted ranged attack (bullet max travel distance).
const RANGED_ATTACK_RANGE: i32 = 100;

/// Maximum inventory slots for the player.
pub const MAX_INVENTORY_SIZE: usize = 6;

/// Stamina cost for the Throw Sand ability (G key).
pub(crate) const SAND_STAMINA_COST: i32 = 8;

/// Stamina cost for the Throw Item ability (E key).
pub(crate) const THROW_ITEM_STAMINA_COST: i32 = 15;

/// Stamina cost for the Roundhouse ability (F key).
const ROUNDHOUSE_STAMINA_COST: i32 = 10;

/// Stamina cost per tile of movement (WASD / arrows).
const MOVE_STAMINA_COST: i32 = 2;

#[cfg(feature = "windowed")]
const WINDOWED_REPEAT_DELAY: f32 = 0.22;

#[cfg(feature = "windowed")]
const WINDOWED_REPEAT_INTERVAL: f32 = 0.045;

/// A single command binding entry: the key(s) that trigger it, a short name, documentation,
/// and a category for UI grouping.
pub struct CommandBinding {
    /// Key combination string shown in the help/welcome screen.
    pub key: &'static str,
    /// Short action name.
    pub name: &'static str,
    /// Longer description / documentation for the command.
    pub docs: &'static str,
    /// Category for grouping in the Q menu: "Movement", "Combat", "Inventory", or "Other".
    pub category: &'static str,
}

/// All keybindings, generated from the exhaustive match arms below.
/// Used by the `?` help overlay to display available commands.
/// Related keys are grouped (WASD, IJKL) to reduce visual clutter.
pub const KEYBINDINGS: &[CommandBinding] = &[
    CommandBinding {
        key: "WASD / ↑↓←→",
        name: "Move",
        docs: "Move the player one tile. Physical movement also costs stamina.",
        category: "Movement",
    },
    CommandBinding {
        key: "IJKL",
        name: "Cursor",
        docs: "Move the cursor one tile for aiming.",
        category: "Other",
    },
    CommandBinding {
        key: "1-0",
        name: "Fire/Use",
        docs: "Primary action: fire, drink, throw, or use the item in that slot toward the cursor.",
        category: "Inventory",
    },
    CommandBinding {
        key: "C",
        name: "Center cursor",
        docs: "Snap cursor onto your position.",
        category: "Other",
    },
    CommandBinding {
        key: "V",
        name: "Auto-aim",
        docs: "Cursor steps toward nearest enemy.",
        category: "Other",
    },
    CommandBinding {
        key: "R",
        name: "Reload",
        docs: "Reload gun using ammo, caps, and powder.",
        category: "Combat",
    },
    CommandBinding {
        key: "F",
        name: "Roundhouse",
        docs: "Roundhouse kicks adjacent enemies and smashes nearby props. Costs stamina.",
        category: "Combat",
    },
    CommandBinding {
        key: "T",
        name: "Wait",
        docs: "Skip your turn.",
        category: "Movement",
    },
    CommandBinding {
        key: "G",
        name: "Throw sand (8 sta)",
        docs: "Create sand cloud blocking vision toward cursor.",
        category: "Combat",
    },
    CommandBinding {
        key: "E",
        name: "Throw item (15 sta)",
        docs: "Throw a random inventory item toward cursor.",
        category: "Inventory",
    },
    CommandBinding {
        key: "Q",
        name: "Menu",
        docs: "Toggle pause menu",
        category: "Other",
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameInput {
    ToggleMenu,
    CursorUp,
    CursorDown,
    CursorLeft,
    CursorRight,
    CenterCursor,
    AutoAim,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Wait,
    ReloadOrRestart,
    Roundhouse,
    ThrowSand,
    ThrowItem,
    ToggleGodMode,
    UseSlot(usize),
}

#[cfg_attr(feature = "windowed", allow(dead_code))]
#[inline]
fn inventory_slot_from_key(c: char) -> Option<usize> {
    match c {
        '1'..='9' => Some((c as usize) - ('1' as usize)),
        '0' => Some(9),
        _ => None,
    }
}

/// Reads keyboard input. Global keys (quit, pause, help) are always handled.
/// Movement keys are only processed while `TurnState::AwaitingInput`,
/// which transitions the game into `PlayerTurn` so that the action is
/// resolved before the next input is accepted.
///
/// When the game is in `GameState::Dead`, only quit (Q) and restart (R) work.
#[cfg(not(feature = "windowed"))]
pub(crate) fn input_system(
    mut messages: MessageReader<KeyMessage>,
    mut intents: IntentWriters,
    player_query: Query<
        (
            Entity,
            &Position,
            Option<&Stamina>,
            Option<&Inventory>,
            Option<&Dead>,
        ),
        With<PlayerControlled>,
    >,
    mut player_viewshed: Query<&mut Viewshed, With<PlayerControlled>>,
    item_kind_query: Query<&ItemKind>,
    (hostiles_query, _health_query): (
        Query<&Position, (With<Faction>, Without<PlayerControlled>)>,
        Query<Entity, With<Health>>,
    ),
    mut resources: InputResources,
) {
    let commands: Vec<_> = messages
        .read()
        .filter_map(|message| map_terminal_input(message.code))
        .collect();
    process_inputs(
        commands,
        &mut intents,
        player_query,
        &mut player_viewshed,
        item_kind_query,
        hostiles_query,
        resources.game_state,
        &mut resources.next_game_state,
        resources.turn_state,
        &mut resources.next_turn_state,
        &mut resources.combat_log,
        &mut resources.input_state,
        &mut resources.restart_requested,
        &mut resources.cursor,
        &mut resources.extra_world_ticks,
        &mut resources.spectating,
        resources.dynamic_rng,
        resources.seed,
        &mut resources.god_mode,
    );
}

#[cfg(feature = "windowed")]
pub(crate) fn input_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut repeat: WindowedInputRepeat,
    mut intents: IntentWriters,
    player_query: Query<
        (
            Entity,
            &Position,
            Option<&Stamina>,
            Option<&Inventory>,
            Option<&Dead>,
        ),
        With<PlayerControlled>,
    >,
    mut player_viewshed: Query<&mut Viewshed, With<PlayerControlled>>,
    item_kind_query: Query<&ItemKind>,
    (hostiles_query, _health_query): (
        Query<&Position, (With<Faction>, Without<PlayerControlled>)>,
        Query<Entity, With<Health>>,
    ),
    mut resources: InputResources,
) {
    let commands = collect_windowed_inputs(&keys, &repeat.time, &mut repeat.key_repeat);
    process_inputs(
        commands,
        &mut intents,
        player_query,
        &mut player_viewshed,
        item_kind_query,
        hostiles_query,
        resources.game_state,
        &mut resources.next_game_state,
        resources.turn_state,
        &mut resources.next_turn_state,
        &mut resources.combat_log,
        &mut resources.input_state,
        &mut resources.restart_requested,
        &mut resources.cursor,
        &mut resources.extra_world_ticks,
        &mut resources.spectating,
        resources.dynamic_rng,
        resources.seed,
        &mut resources.god_mode,
    );
}

fn process_inputs(
    commands: Vec<GameInput>,
    intents: &mut IntentWriters,
    player_query: Query<
        (
            Entity,
            &Position,
            Option<&Stamina>,
            Option<&Inventory>,
            Option<&Dead>,
        ),
        With<PlayerControlled>,
    >,
    player_viewshed: &mut Query<&mut Viewshed, With<PlayerControlled>>,
    item_kind_query: Query<&ItemKind>,
    hostiles_query: Query<&Position, (With<Faction>, Without<PlayerControlled>)>,
    game_state: Res<State<GameState>>,
    next_game_state: &mut ResMut<NextState<GameState>>,
    turn_state: Option<Res<State<TurnState>>>,
    next_turn_state: &mut Option<ResMut<NextState<TurnState>>>,
    combat_log: &mut ResMut<CombatLog>,
    input_state: &mut ResMut<InputState>,
    restart_requested: &mut ResMut<RestartRequested>,
    cursor: &mut ResMut<CursorPosition>,
    extra_world_ticks: &mut ResMut<ExtraWorldTicks>,
    spectating: &mut ResMut<SpectatingAfterDeath>,
    dynamic_rng: Res<DynamicRng>,
    seed: Res<MapSeed>,
    god_mode: &mut ResMut<crate::resources::GodMode>,
) {
    // Handle Dead and Victory states: R to restart, auto-advance turns when dead.
    if *game_state.get() == GameState::Dead || *game_state.get() == GameState::Victory {
        for command in commands {
            if command == GameInput::ReloadOrRestart {
                restart_requested.0 = true;
            }
        }
        return;
    }

    let Ok((player_entity, player_pos, player_stamina, player_inv, player_dead)) =
        player_query.single()
    else {
        // PlayerControlled entity is gone (should only happen transiently).
        return;
    };

    // If the player is dead, only allow restart and auto-advance time.
    if player_dead.is_some() {
        for command in commands {
            if command == GameInput::ReloadOrRestart {
                restart_requested.0 = true;
            }
        }
        // Auto-advance turns when spectating after death so the world keeps running.
        if spectating.0 {
            let awaiting = turn_state
                .as_ref()
                .is_some_and(|s| *s.get() == TurnState::AwaitingInput);
            if awaiting {
                if let Some(nts) = next_turn_state {
                    nts.set(TurnState::PlayerTurn);
                }
            }
        }
        return;
    }

    let awaiting_input = turn_state
        .as_ref()
        .is_some_and(|s| *s.get() == TurnState::AwaitingInput);

    // Spectating mode removed — death screen only offers restart.

    // ── ESC menu input mode ─────────────────────────────────────
    if input_state.mode == InputMode::EscMenu {
        for command in commands {
            match command {
                GameInput::ToggleMenu => {
                    input_state.mode = InputMode::Game;
                    if *game_state.get() == GameState::Paused {
                        next_game_state.set(GameState::Playing);
                    }
                }
                GameInput::ReloadOrRestart => {
                    input_state.mode = InputMode::Game;
                    restart_requested.0 = true;
                }
                _ => {}
            }
        }
        return;
    }

    if input_state.welcome_visible && !commands.is_empty() {
        input_state.welcome_visible = false;
        return;
    }

    // ── Normal game input mode ──────────────────────────────────
    for command in commands {
        // Exhaustive input handling — every arm here corresponds to a KEYBINDINGS entry.
        match command {
            // ── Q key: toggle ESC menu ──────────────────────────
            GameInput::ToggleMenu => {
                // Open ESC menu and pause the game.
                input_state.mode = InputMode::EscMenu;
                if *game_state.get() == GameState::Playing {
                    next_game_state.set(GameState::Paused);
                }
            }
            // ── Cursor movement (IJKL) — advances one tick ─────
            GameInput::CursorUp if awaiting_input => {
                move_cursor(
                    cursor,
                    0,
                    1,
                    player_viewshed,
                    next_turn_state,
                );
            }
            GameInput::CursorDown if awaiting_input => {
                move_cursor(
                    cursor,
                    0,
                    -1,
                    player_viewshed,
                    next_turn_state,
                );
            }
            GameInput::CursorLeft if awaiting_input => {
                move_cursor(
                    cursor,
                    -1,
                    0,
                    player_viewshed,
                    next_turn_state,
                );
            }
            GameInput::CursorRight if awaiting_input => {
                move_cursor(
                    cursor,
                    1,
                    0,
                    player_viewshed,
                    next_turn_state,
                );
            }
            // ── Center cursor on player (C) — advances one tick ──
            GameInput::CenterCursor if awaiting_input => {
                cursor.pos = player_pos.as_grid_vec();
                mark_viewshed_dirty(player_viewshed);
                advance_turn(next_turn_state);
            }
            // ── Auto-aim (V): move cursor one step toward nearest hostile — advances one tick ──
            GameInput::AutoAim if awaiting_input => {
                let player_vec = player_pos.as_grid_vec();
                let mut best_dist = i32::MAX;
                let mut best_pos = None;
                for hostile_pos in &hostiles_query {
                    let hv = hostile_pos.as_grid_vec();
                    let dist = player_vec.chebyshev_distance(hv);
                    if dist < best_dist {
                        best_dist = dist;
                        best_pos = Some(hv);
                    }
                }
                if let Some(target) = best_pos {
                    let step = (target - cursor.pos).king_step();
                    cursor.pos += step;
                    mark_viewshed_dirty(player_viewshed);
                    advance_turn(next_turn_state);
                } else {
                    combat_log.push("No enemies visible.".into());
                }
            }
            // ── Movement keys (only while awaiting input) ───────
            // Normal movement — costs 3 ticks and 2 stamina
            GameInput::MoveUp if awaiting_input => {
                extra_world_ticks.0 = 2;
                input_state.ability_stamina_pending = MOVE_STAMINA_COST;
                emit_move(
                    &mut intents.move_intents,
                    next_turn_state,
                    player_entity,
                    0,
                    1,
                );
            }
            GameInput::MoveDown if awaiting_input => {
                extra_world_ticks.0 = 2;
                input_state.ability_stamina_pending = MOVE_STAMINA_COST;
                emit_move(
                    &mut intents.move_intents,
                    next_turn_state,
                    player_entity,
                    0,
                    -1,
                );
            }
            GameInput::MoveLeft if awaiting_input => {
                extra_world_ticks.0 = 2;
                input_state.ability_stamina_pending = MOVE_STAMINA_COST;
                emit_move(
                    &mut intents.move_intents,
                    next_turn_state,
                    player_entity,
                    -1,
                    0,
                );
            }
            GameInput::MoveRight if awaiting_input => {
                extra_world_ticks.0 = 2;
                input_state.ability_stamina_pending = MOVE_STAMINA_COST;
                emit_move(
                    &mut intents.move_intents,
                    next_turn_state,
                    player_entity,
                    1,
                    0,
                );
            }
            // ── Wait / skip turn (T) ────────────────────────────
            GameInput::Wait if awaiting_input => {
                combat_log.push("You wait...".into());
                advance_turn(next_turn_state);
            }
            // ── Reload weapon from inventory magazine — costs 10 ticks ──
            GameInput::ReloadOrRestart if awaiting_input => {
                extra_world_ticks.0 = 9;
                input_state.reload_pending = true;
                advance_turn(next_turn_state);
            }
            // ── Melee wide (roundhouse) attack — costs 2 ticks + stamina (F key) ────
            GameInput::Roundhouse if awaiting_input => {
                let has_stamina = player_stamina
                    .map(|m| m.current >= ROUNDHOUSE_STAMINA_COST)
                    .unwrap_or(false);
                if !has_stamina {
                    combat_log.push("Not enough stamina for roundhouse!".into());
                } else {
                    extra_world_ticks.0 = 1;
                    input_state.ability_stamina_pending = ROUNDHOUSE_STAMINA_COST;
                    intents.melee_wide_intents.write(MeleeWideIntent {
                        attacker: player_entity,
                    });
                    advance_turn(next_turn_state);
                }
            }
            // ── Throw sand (G key): create sand cloud blocking vision (costs stamina) ──
            GameInput::ThrowSand if awaiting_input => {
                let has_stamina = player_stamina
                    .map(|m| m.current >= SAND_STAMINA_COST)
                    .unwrap_or(false);
                if !has_stamina {
                    combat_log.push("Not enough stamina!".into());
                } else {
                    let delta = cursor.pos - player_pos.as_grid_vec();
                    if delta == crate::grid_vec::GridVec::ZERO {
                        combat_log.push("Cursor is on your position!".into());
                    } else {
                        let step = delta.king_step();
                        let sand_center = player_pos.as_grid_vec() + step * 2;
                        // Create sand cloud as spell particles (visual obstruction)
                        intents.spell_intents.write(SpellCastIntent {
                            caster: player_entity,
                            radius: 2,
                            target: sand_center,
                            grenade_index: usize::MAX, // sentinel: no grenade consumed
                        });
                        input_state.ability_stamina_pending = SAND_STAMINA_COST;
                        combat_log.push("You throw a handful of sand!".into());
                        extra_world_ticks.0 = 0;
                        advance_turn(next_turn_state);
                    }
                }
            }
            // ── Throw random item (E key): throw inventory item toward cursor (costs stamina) ──
            GameInput::ThrowItem if awaiting_input => {
                let has_stamina = player_stamina
                    .map(|m| m.current >= THROW_ITEM_STAMINA_COST)
                    .unwrap_or(false);
                if !has_stamina {
                    combat_log.push("Not enough stamina to throw!".into());
                } else {
                    input_state.ability_stamina_pending = THROW_ITEM_STAMINA_COST;
                    handle_throw_random(
                        player_entity,
                        player_pos,
                        player_inv,
                        &item_kind_query,
                        cursor,
                        intents,
                        extra_world_ticks,
                        next_turn_state,
                        combat_log,
                        &dynamic_rng,
                        &seed,
                    );
                }
            }
            // ── Toggle God Mode (Shift+G) ───────────────────────
            GameInput::ToggleGodMode if awaiting_input => {
                god_mode.0 = !god_mode.0;
                if god_mode.0 {
                    combat_log.push("God mode ENABLED — you are invincible.".into());
                } else {
                    combat_log.push("God mode DISABLED.".into());
                }
            }
            // ── Use inventory item by slot (1-0) / Fire gun toward cursor / Throw / Grenade ──
            // Combat actions cost 2 ticks.
            GameInput::UseSlot(idx) if awaiting_input => {
                let mut handled = false;
                if let Some(inv) = player_inv
                    && let Some(&item_entity) = inv.items.get(idx)
                    && let Ok(kind) = item_kind_query.get(item_entity)
                {
                    if let ItemKind::Gun { loaded, name, .. } = kind {
                        if *loaded > 0 {
                            let delta = cursor.pos - player_pos.as_grid_vec();
                            if delta != crate::grid_vec::GridVec::ZERO {
                                let _is_starr = name.contains("Starr");
                                extra_world_ticks.0 = 1;
                                intents.ranged_intents.write(RangedAttackIntent {
                                    attacker: player_entity,
                                    range: RANGED_ATTACK_RANGE,
                                    dx: delta.x,
                                    dy: delta.y,
                                    gun_item: Some(item_entity),
                                });
                                advance_turn(next_turn_state);
                                handled = true;
                            } else {
                                combat_log.push("Cursor is on your position!".into());
                                handled = true;
                            }
                        } else {
                            combat_log.push("Gun is empty! Press R to reload.".into());
                            handled = true;
                        }
                    } else if let ItemKind::Bow { .. } = kind {
                        let delta = cursor.pos - player_pos.as_grid_vec();
                        if delta != crate::grid_vec::GridVec::ZERO {
                            extra_world_ticks.0 = 1;
                            intents.ranged_intents.write(RangedAttackIntent {
                                attacker: player_entity,
                                range: 28,
                                dx: delta.x,
                                dy: delta.y,
                                gun_item: Some(item_entity),
                            });
                            advance_turn(next_turn_state);
                            handled = true;
                        } else {
                            combat_log.push("Cursor is on your position!".into());
                            handled = true;
                        }
                    } else if let ItemKind::Knife { attack, .. }
                    | ItemKind::Tomahawk { attack, .. } = kind
                    {
                        let delta = cursor.pos - player_pos.as_grid_vec();
                        if delta != crate::grid_vec::GridVec::ZERO {
                            extra_world_ticks.0 = 1;
                            intents.throw_item_intents.write(ThrowItemIntent {
                                thrower: player_entity,
                                item_entity,
                                item_index: idx,
                                dx: delta.x,
                                dy: delta.y,
                                range: crate::systems::projectile::THROWN_RANGE,
                                damage: *attack,
                            });
                            advance_turn(next_turn_state);
                            handled = true;
                        } else {
                            combat_log.push("Cursor is on your position!".into());
                            handled = true;
                        }
                    } else if matches!(kind, ItemKind::Grenade { .. }) {
                        // Throw grenade from this inventory slot toward the cursor.
                        let has_stamina = player_stamina
                            .map(|m| m.current >= SPELL_STAMINA_COST)
                            .unwrap_or(false);
                        if !has_stamina {
                            combat_log.push("Not enough stamina!".into());
                        } else {
                            extra_world_ticks.0 = 1;
                            intents.spell_intents.write(SpellCastIntent {
                                caster: player_entity,
                                radius: SPELL_RADIUS,
                                target: cursor.pos,
                                grenade_index: idx,
                            });
                            advance_turn(next_turn_state);
                        }
                        handled = true;
                    } else if let ItemKind::Molotov { damage, radius, .. } = kind {
                        // Throw molotov from this inventory slot toward the cursor.
                        let has_stamina = player_stamina
                            .map(|m| m.current >= SPELL_STAMINA_COST)
                            .unwrap_or(false);
                        if !has_stamina {
                            combat_log.push("Not enough stamina!".into());
                        } else {
                            extra_world_ticks.0 = 1;
                            intents.molotov_intents.write(MolotovCastIntent {
                                caster: player_entity,
                                radius: *radius,
                                damage: *damage,
                                target: cursor.pos,
                                item_index: idx,
                            });
                            advance_turn(next_turn_state);
                        }
                        handled = true;
                    }
                }
                if !handled {
                    // Non-gun items: use normally.
                    intents.use_item_intents.write(UseItemIntent {
                        user: player_entity,
                        item_index: idx,
                    });
                    advance_turn(next_turn_state);
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(feature = "windowed"))]
fn map_terminal_input(key: KeyCode) -> Option<GameInput> {
    match key {
        KeyCode::Char('q') => Some(GameInput::ToggleMenu),
        KeyCode::Char('i') => Some(GameInput::CursorUp),
        KeyCode::Char('k') => Some(GameInput::CursorDown),
        KeyCode::Char('j') => Some(GameInput::CursorLeft),
        KeyCode::Char('l') => Some(GameInput::CursorRight),
        KeyCode::Char('c') => Some(GameInput::CenterCursor),
        KeyCode::Char('v') => Some(GameInput::AutoAim),
        KeyCode::Char('w') | KeyCode::Up => Some(GameInput::MoveUp),
        KeyCode::Char('s') | KeyCode::Down => Some(GameInput::MoveDown),
        KeyCode::Char('a') | KeyCode::Left => Some(GameInput::MoveLeft),
        KeyCode::Char('d') | KeyCode::Right => Some(GameInput::MoveRight),
        KeyCode::Char('t') => Some(GameInput::Wait),
        KeyCode::Char('r') => Some(GameInput::ReloadOrRestart),
        KeyCode::Char('f') => Some(GameInput::Roundhouse),
        KeyCode::Char('g') => Some(GameInput::ThrowSand),
        KeyCode::Char('e') => Some(GameInput::ThrowItem),
        KeyCode::Char('G') => Some(GameInput::ToggleGodMode),
        KeyCode::Char(c) => inventory_slot_from_key(c).map(GameInput::UseSlot),
        _ => None,
    }
}

#[cfg(feature = "windowed")]
fn collect_windowed_inputs(
    keys: &ButtonInput<KeyCode>,
    time: &Time,
    key_repeat: &mut WindowedKeyRepeat,
) -> Vec<GameInput> {
    let mut commands = Vec::new();
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyQ,
        GameInput::ToggleMenu,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyI,
        GameInput::CursorUp,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyK,
        GameInput::CursorDown,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyJ,
        GameInput::CursorLeft,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyL,
        GameInput::CursorRight,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyC,
        GameInput::CenterCursor,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyV,
        GameInput::AutoAim,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyW,
        GameInput::MoveUp,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::ArrowUp,
        GameInput::MoveUp,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyS,
        GameInput::MoveDown,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::ArrowDown,
        GameInput::MoveDown,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyA,
        GameInput::MoveLeft,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::ArrowLeft,
        GameInput::MoveLeft,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyD,
        GameInput::MoveRight,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::ArrowRight,
        GameInput::MoveRight,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyT,
        GameInput::Wait,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyR,
        GameInput::ReloadOrRestart,
    );
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyF,
        GameInput::Roundhouse,
    );
    if consume_windowed_repeat(keys, time, key_repeat, KeyCode::KeyG) {
        commands.push(if shift {
            GameInput::ToggleGodMode
        } else {
            GameInput::ThrowSand
        });
    }
    update_windowed_repeat(
        &mut commands,
        keys,
        time,
        key_repeat,
        KeyCode::KeyE,
        GameInput::ThrowItem,
    );

    for (key, slot) in [
        (KeyCode::Digit1, 0),
        (KeyCode::Digit2, 1),
        (KeyCode::Digit3, 2),
        (KeyCode::Digit4, 3),
        (KeyCode::Digit5, 4),
        (KeyCode::Digit6, 5),
        (KeyCode::Digit7, 6),
        (KeyCode::Digit8, 7),
        (KeyCode::Digit9, 8),
        (KeyCode::Digit0, 9),
    ] {
        if consume_windowed_repeat(keys, time, key_repeat, key) {
            commands.push(GameInput::UseSlot(slot));
        }
    }

    commands
}

#[cfg(feature = "windowed")]
fn update_windowed_repeat(
    commands: &mut Vec<GameInput>,
    keys: &ButtonInput<KeyCode>,
    time: &Time,
    key_repeat: &mut WindowedKeyRepeat,
    key: KeyCode,
    command: GameInput,
) {
    if consume_windowed_repeat(keys, time, key_repeat, key) {
        commands.push(command);
    }
}

#[cfg(feature = "windowed")]
fn consume_windowed_repeat(
    keys: &ButtonInput<KeyCode>,
    time: &Time,
    key_repeat: &mut WindowedKeyRepeat,
    key: KeyCode,
) -> bool {
    if keys.just_pressed(key) {
        key_repeat
            .held
            .insert(key, std::time::Duration::from_secs_f32(WINDOWED_REPEAT_DELAY));
        return true;
    }

    if !keys.pressed(key) {
        key_repeat.held.remove(&key);
        return false;
    }

    let Some(remaining) = key_repeat.held.get_mut(&key) else {
        key_repeat
            .held
            .insert(key, std::time::Duration::from_secs_f32(WINDOWED_REPEAT_DELAY));
        return false;
    };

    let delta = time.delta();
    if *remaining > delta {
        *remaining -= delta;
        return false;
    }

    let overshoot = delta.saturating_sub(*remaining).as_secs_f32();
    let interval = WINDOWED_REPEAT_INTERVAL;
    let next_remaining = if overshoot <= 0.0 {
        interval
    } else {
        let remainder = overshoot % interval;
        if remainder == 0.0 {
            interval
        } else {
            interval - remainder
        }
    };
    *remaining = std::time::Duration::from_secs_f32(next_remaining);
    true
}

/// Special ability: Throw random inventory item toward cursor.
fn handle_throw_random(
    player_entity: Entity,
    player_pos: &Position,
    player_inv: Option<&Inventory>,
    item_kind_query: &Query<&ItemKind>,
    cursor: &CursorPosition,
    intents: &mut IntentWriters,
    extra_world_ticks: &mut ExtraWorldTicks,
    next_turn_state: &mut Option<ResMut<NextState<TurnState>>>,
    combat_log: &mut CombatLog,
    dynamic_rng: &DynamicRng,
    seed: &MapSeed,
) {
    let delta = cursor.pos - player_pos.as_grid_vec();
    if delta == crate::grid_vec::GridVec::ZERO {
        combat_log.push("Cursor is on your position!".into());
        return;
    }

    let Some(inv) = player_inv else {
        combat_log.push("No inventory!".into());
        return;
    };

    if inv.items.is_empty() {
        combat_log.push("Inventory is empty!".into());
        return;
    }

    // Pick random item
    let idx = dynamic_rng.random_index(seed.0, 0x7000, inv.items.len());
    let item_entity = inv.items[idx];

    // Determine damage based on item type
    let damage = item_kind_query
        .get(item_entity)
        .ok()
        .map_or(2, |k| match k {
            ItemKind::Knife { attack, .. } | ItemKind::Tomahawk { attack, .. } => *attack,
            ItemKind::Gun { attack, .. } => *attack / 2,
            _ => 2,
        });

    intents.throw_item_intents.write(ThrowItemIntent {
        thrower: player_entity,
        item_entity,
        item_index: idx,
        dx: delta.x,
        dy: delta.y,
        range: crate::systems::projectile::THROWN_RANGE,
        damage,
    });
    extra_world_ticks.0 = 1;
    advance_turn(next_turn_state);
}

/// Helper: emits a `MoveIntent` and advances the turn state to `PlayerTurn`.
fn emit_move(
    move_intents: &mut MessageWriter<MoveIntent>,
    next_turn_state: &mut Option<ResMut<NextState<TurnState>>>,
    entity: Entity,
    dx: i32,
    dy: i32,
) {
    move_intents.write(MoveIntent { entity, dx, dy });
    advance_turn(next_turn_state);
}

/// Helper: transitions to `PlayerTurn`, ending the input phase.
#[inline]
fn advance_turn(next_turn_state: &mut Option<ResMut<NextState<TurnState>>>) {
    if let Some(next) = next_turn_state {
        next.set(TurnState::PlayerTurn);
    }
}

/// Helper: marks the player's viewshed as dirty so FOV is recalculated.
#[inline]
fn mark_viewshed_dirty(player_viewshed: &mut Query<&mut Viewshed, With<PlayerControlled>>) {
    if let Ok(mut vs) = player_viewshed.single_mut() {
        vs.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::inventory_slot_from_key;

    #[test]
    fn inventory_keys_cover_one_through_zero() {
        assert_eq!(inventory_slot_from_key('1'), Some(0));
        assert_eq!(inventory_slot_from_key('5'), Some(4));
        assert_eq!(inventory_slot_from_key('9'), Some(8));
        assert_eq!(inventory_slot_from_key('0'), Some(9));
        assert_eq!(inventory_slot_from_key('x'), None);
    }
}

/// Helper: moves the cursor by `(dx, dy)`, marks viewshed dirty, and advances the turn.
#[inline]
fn move_cursor(
    cursor: &mut ResMut<CursorPosition>,
    dx: i32,
    dy: i32,
    player_viewshed: &mut Query<&mut Viewshed, With<PlayerControlled>>,
    next_turn_state: &mut Option<ResMut<NextState<TurnState>>>,
) {
    cursor.pos.x += dx;
    cursor.pos.y += dy;
    mark_viewshed_dirty(player_viewshed);
    advance_turn(next_turn_state);
}
