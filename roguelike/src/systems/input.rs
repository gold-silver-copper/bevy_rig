use bevy::{app::AppExit, prelude::*};
use bevy_ratatui::event::KeyMessage;
use ratatui::crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use std::collections::HashSet;

use crate::{
    components::{Name, Npc, PendingReply, Player, Position},
    events::{InteractIntent, MoveIntent, TalkIntent},
    map::TileMap,
    resources::{PLAYER_VIEW_MAX_RANGE, PLAYER_VIEW_RADIUS, UiMode, UiState},
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
    mut interact_intents: MessageWriter<InteractIntent>,
) {
    let Ok((player_entity, player_pos)) = player_query.single() else {
        return;
    };
    let player_visible_tiles = map.player_visible_tiles(
        *player_pos,
        ui.cursor,
        PLAYER_VIEW_RADIUS,
        PLAYER_VIEW_MAX_RANGE,
    );

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
                    &mut interact_intents,
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
    _visible_tiles: &HashSet<Position>,
    _npc_query: &Query<(Entity, &Position, &Name, &PendingReply), With<Npc>>,
    ui: &mut UiState,
    runtime: &mut RigRuntime,
    move_intents: &mut MessageWriter<MoveIntent>,
    interact_intents: &mut MessageWriter<InteractIntent>,
    exit: &mut MessageWriter<AppExit>,
) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            exit.write_default();
            true
        }
        KeyCode::Char('i') => {
            move_cursor(ui, map, 0, -1);
            false
        }
        KeyCode::Char('k') => {
            move_cursor(ui, map, 0, 1);
            false
        }
        KeyCode::Char('j') => {
            move_cursor(ui, map, -1, 0);
            false
        }
        KeyCode::Char('l') => {
            move_cursor(ui, map, 1, 0);
            false
        }
        KeyCode::Char('c') => {
            ui.cursor = player_pos;
            false
        }
        KeyCode::F(2) | KeyCode::Char('`') => {
            ui.show_debug = !ui.show_debug;
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
        KeyCode::Enter | KeyCode::Char('t') => {
            ui.mode = UiMode::Talking;
            ui.draft.clear();
            false
        }
        KeyCode::Char('e') => {
            interact_intents.write(InteractIntent {
                position: ui.cursor,
            });
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

fn move_cursor(ui: &mut UiState, map: &TileMap, dx: i32, dy: i32) {
    let next = ui.cursor.offset(dx, dy);
    if !map.in_bounds(next.x, next.y) {
        return;
    }

    ui.cursor = next;
}
