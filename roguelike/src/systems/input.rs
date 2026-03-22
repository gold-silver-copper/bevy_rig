use bevy::{app::AppExit, prelude::*};
use bevy_ratatui::event::KeyMessage;
use ratatui::crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use std::collections::HashSet;

use crate::{
    components::{Name, Npc, PendingReply, Player, Position},
    events::{MoveIntent, TalkIntent},
    map::TileMap,
    resources::{PLAYER_VIEW_RADIUS, UiMode, UiState},
    runtime::RigRuntime,
};

pub fn input_system(
    mut key_messages: MessageReader<KeyMessage>,
    mut exit: MessageWriter<AppExit>,
    map: Res<TileMap>,
    player_query: Query<(Entity, &Position), With<Player>>,
    npc_query: Query<(Entity, &Position, &Name, &PendingReply), With<Npc>>,
    mut ui: ResMut<UiState>,
    mut runtime: ResMut<RigRuntime>,
    mut move_intents: MessageWriter<MoveIntent>,
    mut talk_intents: MessageWriter<TalkIntent>,
) {
    let Ok((player_entity, player_pos)) = player_query.single() else {
        return;
    };
    let player_visible_tiles = map.visible_tiles(*player_pos, PLAYER_VIEW_RADIUS);

    for key in key_messages.read() {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            continue;
        }

        match ui.mode {
            UiMode::Explore => {
                if handle_explore_input(
                    key,
                    player_entity,
                    *player_pos,
                    &map,
                    &player_visible_tiles,
                    &npc_query,
                    &mut ui,
                    &mut runtime,
                    &mut move_intents,
                    &mut exit,
                ) {
                    break;
                }
            }
            UiMode::Talking => {
                handle_talk_input(key, &npc_query, &mut ui, &mut talk_intents);
            }
        }
    }
}

fn handle_explore_input(
    key: &KeyMessage,
    player_entity: Entity,
    player_pos: Position,
    map: &TileMap,
    visible_tiles: &HashSet<Position>,
    npc_query: &Query<(Entity, &Position, &Name, &PendingReply), With<Npc>>,
    ui: &mut UiState,
    runtime: &mut RigRuntime,
    move_intents: &mut MessageWriter<MoveIntent>,
    exit: &mut MessageWriter<AppExit>,
) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            exit.write_default();
            true
        }
        KeyCode::Char('i') => {
            move_cursor(ui, npc_query, visible_tiles, map, 0, -1);
            false
        }
        KeyCode::Char('k') => {
            move_cursor(ui, npc_query, visible_tiles, map, 0, 1);
            false
        }
        KeyCode::Char('j') => {
            move_cursor(ui, npc_query, visible_tiles, map, -1, 0);
            false
        }
        KeyCode::Char('l') => {
            move_cursor(ui, npc_query, visible_tiles, map, 1, 0);
            false
        }
        KeyCode::Char('c') => {
            ui.cursor = player_pos;
            sync_selected_npc_to_cursor(visible_tiles, npc_query, ui);
            false
        }
        KeyCode::Up | KeyCode::Char('w') => {
            emit_player_move(move_intents, player_entity, 0, -1);
            false
        }
        KeyCode::Down | KeyCode::Char('s') => {
            emit_player_move(move_intents, player_entity, 0, 1);
            false
        }
        KeyCode::Left | KeyCode::Char('a') => {
            emit_player_move(move_intents, player_entity, -1, 0);
            false
        }
        KeyCode::Right | KeyCode::Char('d') => {
            emit_player_move(move_intents, player_entity, 1, 0);
            false
        }
        KeyCode::Tab => {
            cycle_selected_npc(player_pos, visible_tiles, npc_query, ui);
            false
        }
        KeyCode::Enter | KeyCode::Char('t') => {
            ui.mode = UiMode::Talking;
            ui.draft.clear();
            false
        }
        KeyCode::Char('[') => {
            runtime.cycle_provider(-1);
            false
        }
        KeyCode::Char(']') => {
            runtime.cycle_provider(1);
            false
        }
        KeyCode::Char(' ') => {
            move_intents.write(MoveIntent {
                entity: player_entity,
                dx: 0,
                dy: 0,
            });
            false
        }
        _ => false,
    }
}

fn emit_player_move(
    move_intents: &mut MessageWriter<MoveIntent>,
    entity: Entity,
    dx: i32,
    dy: i32,
) {
    move_intents.write(MoveIntent { entity, dx, dy });
}

fn handle_talk_input(
    key: &KeyMessage,
    _npc_query: &Query<(Entity, &Position, &Name, &PendingReply), With<Npc>>,
    ui: &mut UiState,
    talk_intents: &mut MessageWriter<TalkIntent>,
) {
    match key.code {
        KeyCode::Esc => {
            ui.mode = UiMode::Explore;
            ui.draft.clear();
        }
        KeyCode::Enter => {
            let draft = ui.draft.trim().to_string();
            if draft.is_empty() {
                return;
            }

            talk_intents.write(TalkIntent { prompt: draft });
            ui.draft.clear();
        }
        KeyCode::Backspace => {
            ui.draft.pop();
        }
        KeyCode::Char(c)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            ui.draft.push(c);
        }
        KeyCode::Tab => {}
        _ => {}
    }
}

fn cycle_selected_npc(
    player_pos: Position,
    visible_tiles: &HashSet<Position>,
    npc_query: &Query<(Entity, &Position, &Name, &PendingReply), With<Npc>>,
    ui: &mut UiState,
) {
    let mut npcs = npc_query
        .iter()
        .filter(|(_, pos, _, _)| visible_tiles.contains(pos))
        .map(|(entity, pos, name, _)| {
            (
                entity,
                player_pos.chebyshev_distance(*pos),
                name.0.clone(),
                *pos,
            )
        })
        .collect::<Vec<_>>();

    npcs.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.x.cmp(&right.3.x))
            .then_with(|| left.3.y.cmp(&right.3.y))
    });

    if npcs.is_empty() {
        ui.selected_npc = None;
        return;
    }

    let current = ui
        .selected_npc
        .and_then(|selected| npcs.iter().position(|(entity, ..)| *entity == selected));

    let next_index = current.map(|index| (index + 1) % npcs.len()).unwrap_or(0);
    ui.selected_npc = Some(npcs[next_index].0);
}

fn move_cursor(
    ui: &mut UiState,
    npc_query: &Query<(Entity, &Position, &Name, &PendingReply), With<Npc>>,
    visible_tiles: &HashSet<Position>,
    map: &TileMap,
    dx: i32,
    dy: i32,
) {
    let next = ui.cursor.offset(dx, dy);
    if !map.in_bounds(next.x, next.y) {
        return;
    }

    ui.cursor = next;
    sync_selected_npc_to_cursor(visible_tiles, npc_query, ui);
}

fn sync_selected_npc_to_cursor(
    visible_tiles: &HashSet<Position>,
    npc_query: &Query<(Entity, &Position, &Name, &PendingReply), With<Npc>>,
    ui: &mut UiState,
) {
    if !visible_tiles.contains(&ui.cursor) {
        return;
    }

    ui.selected_npc = npc_query
        .iter()
        .find(|(_, pos, _, _)| **pos == ui.cursor)
        .map(|(entity, _, _, _)| entity);
}
