use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_ratatui::RatatuiContext;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    components::{Glyph, Memory, MovePlan, Name, Npc, PendingMove, PendingReply, Player, Position},
    map::{Tile, TileMap},
    resources::{GameLog, PLAYER_VIEW_RADIUS, PlayerNeeds, UiMode, UiState, WorldClock},
    runtime::RigRuntime,
};

type PlayerPanelQuery<'w, 's> =
    Query<'w, 's, (&'static Position, &'static Glyph, &'static Name), With<Player>>;
type NpcPanelQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Position,
        &'static Glyph,
        &'static Name,
        &'static Memory,
        &'static MovePlan,
        &'static PendingMove,
        &'static PendingReply,
    ),
    With<Npc>,
>;

#[derive(Clone, Copy)]
struct CameraViewport {
    origin: Position,
    width: i32,
    height: i32,
}

pub fn draw_system(
    mut context: ResMut<RatatuiContext>,
    map: Res<TileMap>,
    ui: Res<UiState>,
    log: Res<GameLog>,
    clock: Res<WorldClock>,
    runtime: Res<RigRuntime>,
    needs: Res<PlayerNeeds>,
    player_query: PlayerPanelQuery,
    npc_query: NpcPanelQuery,
) -> Result {
    let Ok((player_pos, _, player_name)) = player_query.single() else {
        return Ok(());
    };
    let player_visible_tiles = map.visible_tiles(*player_pos, PLAYER_VIEW_RADIUS);

    context.draw(|frame| {
        let area = frame.area();
        let bottom_height = if ui.mode == UiMode::Talking { 13 } else { 12 };
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(bottom_height)])
            .split(area);

        let town_block = Block::default().borders(Borders::ALL).title("Town");
        let town_inner = town_block.inner(vertical[0]);
        let viewport = build_camera_viewport(&map, *player_pos, town_inner);
        let actor_map = build_actor_map(
            ui.selected_npc,
            ui.cursor,
            &player_query,
            &npc_query,
            &player_visible_tiles,
        );

        frame.render_widget(town_block, vertical[0]);
        frame.render_widget(
            Paragraph::new(build_map_lines(
                &map,
                viewport,
                &player_visible_tiles,
                &actor_map,
                ui.cursor,
            ))
            .wrap(Wrap { trim: false }),
            town_inner,
        );

        let bottom_block = Block::default()
            .borders(Borders::ALL)
            .title("Tavern Interface");
        let bottom_inner = bottom_block.inner(vertical[1]);
        frame.render_widget(bottom_block, vertical[1]);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(34),
                Constraint::Min(36),
                Constraint::Length(36),
            ])
            .split(bottom_inner);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Min(5)])
            .split(columns[2]);

        frame.render_widget(
            Paragraph::new(build_status_input_panel(
                &ui,
                *player_pos,
                player_name,
                &runtime,
                &clock,
                &npc_query,
            ))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Status / Input"),
            )
            .wrap(Wrap { trim: false }),
            columns[0],
        );

        frame.render_widget(
            Paragraph::new(build_history_panel(&log))
                .block(Block::default().borders(Borders::ALL).title("History"))
                .wrap(Wrap { trim: false }),
            columns[1],
        );

        frame.render_widget(
            Paragraph::new(build_stats_panel(
                &needs,
                *player_pos,
                ui.selected_npc,
                &npc_query,
            ))
            .block(Block::default().borders(Borders::ALL).title("Stats"))
            .wrap(Wrap { trim: false }),
            right[0],
        );

        frame.render_widget(
            Paragraph::new(build_cursor_panel(
                &map,
                ui.cursor,
                *player_pos,
                &player_visible_tiles,
                &player_query,
                &npc_query,
            ))
            .block(Block::default().borders(Borders::ALL).title("Cursor"))
            .wrap(Wrap { trim: false }),
            right[1],
        );
    })?;

    Ok(())
}

fn build_camera_viewport(map: &TileMap, player_pos: Position, area: Rect) -> CameraViewport {
    let width = i32::from(area.width.max(1));
    let height = i32::from(area.height.max(1));
    let clamped_width = width.min(map.width.max(1));
    let clamped_height = height.min(map.height.max(1));
    let max_x = (map.width - clamped_width).max(0);
    let max_y = (map.height - clamped_height).max(0);

    let origin_x = (player_pos.x - clamped_width / 2).clamp(0, max_x);
    let origin_y = (player_pos.y - clamped_height / 2).clamp(0, max_y);

    CameraViewport {
        origin: Position::new(origin_x, origin_y),
        width: clamped_width,
        height: clamped_height,
    }
}

fn build_actor_map(
    selected: Option<Entity>,
    cursor: Position,
    player_query: &PlayerPanelQuery,
    npc_query: &NpcPanelQuery,
    visible_tiles: &HashSet<Position>,
) -> HashMap<(i32, i32), (char, Style)> {
    let mut actor_map = HashMap::new();

    if let Ok((pos, glyph, _)) = player_query.single() {
        let mut style = Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD);
        if *pos == cursor && visible_tiles.contains(pos) {
            style = style.bg(Color::Yellow).fg(Color::Black);
        }
        actor_map.insert((pos.x, pos.y), (glyph.0, style));
    }

    for (entity, pos, glyph, _, _, move_plan, pending_move, pending_reply) in npc_query.iter() {
        if !visible_tiles.contains(pos) {
            continue;
        }

        let mut style = if Some(entity) == selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if pending_reply.waiting() {
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD)
        } else if pending_move.waiting() {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if move_plan.has_steps() {
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        };

        if *pos == cursor {
            style = style.bg(Color::Yellow).fg(Color::Black);
        }

        actor_map.insert((pos.x, pos.y), (glyph.0, style));
    }

    actor_map
}

fn build_map_lines(
    map: &TileMap,
    viewport: CameraViewport,
    visible_tiles: &HashSet<Position>,
    actor_map: &HashMap<(i32, i32), (char, Style)>,
    cursor: Position,
) -> Text<'static> {
    let mut lines = Vec::with_capacity(viewport.height as usize);

    for y in viewport.origin.y..(viewport.origin.y + viewport.height) {
        let mut spans = Vec::with_capacity(viewport.width as usize);
        for x in viewport.origin.x..(viewport.origin.x + viewport.width) {
            let pos = Position::new(x, y);
            if !visible_tiles.contains(&pos) {
                spans.push(Span::raw(" "));
                continue;
            }

            if let Some((glyph, style)) = actor_map.get(&(x, y)) {
                spans.push(Span::styled(glyph.to_string(), *style));
                continue;
            }

            if pos == cursor {
                spans.push(Span::styled(
                    "x",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
                continue;
            }

            match map.tile(x, y) {
                Tile::Floor => spans.push(Span::styled(".", Style::default().fg(Color::DarkGray))),
                Tile::Wall => spans.push(Span::styled("#", Style::default().fg(Color::Gray))),
            }
        }
        lines.push(Line::from(spans));
    }

    Text::from(lines)
}

fn build_history_panel(log: &GameLog) -> Text<'static> {
    if log.lines.is_empty() {
        return Text::from("The tavern is quiet.");
    }

    let lines = log
        .lines
        .iter()
        .rev()
        .take(32)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(Line::from)
        .collect::<Vec<_>>();

    Text::from(lines)
}

fn build_status_input_panel(
    ui: &UiState,
    player_pos: Position,
    player_name: &Name,
    runtime: &RigRuntime,
    clock: &WorldClock,
    npc_query: &NpcPanelQuery,
) -> Text<'static> {
    let provider = runtime.current_provider();
    let readiness = if provider.ready { "ready" } else { "offline" };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "rig roguelike",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {}  time={:.1}s  events={}  frame={}",
                match ui.mode {
                    UiMode::Explore => "explore",
                    UiMode::Talking => "talk",
                },
                clock.elapsed_seconds,
                clock.turn,
                clock.frame
            )),
        ]),
        Line::from(format!(
            "route={} / {} [{}]",
            provider.label, provider.default_model, readiness
        )),
        Line::from(format!(
            "player={} @ ({}, {})",
            player_name.0, player_pos.x, player_pos.y
        )),
        Line::from(format!(
            "cursor=({}, {})  focus={}",
            ui.cursor.x,
            ui.cursor.y,
            selected_name(ui.selected_npc, npc_query)
        )),
        Line::from("move=WASD/arrows  cursor=IJKL  talk=T/Enter"),
        Line::from("cursor center=C  provider=[ ]  focus=Tab  wait=Space  quit=Q/Esc"),
    ];

    match ui.mode {
        UiMode::Explore => lines.push(Line::from(
            "Say something out loud and anyone in earshot with clear line of sight may react.",
        )),
        UiMode::Talking => lines.push(Line::from(if ui.draft.is_empty() {
            "draft: type a line to speak aloud, then press Enter. Esc exits talk.".to_string()
        } else {
            format!("draft: {}", ui.draft)
        })),
    }

    Text::from(lines)
}

fn build_stats_panel(
    needs: &PlayerNeeds,
    player_pos: Position,
    selected: Option<Entity>,
    npc_query: &NpcPanelQuery,
) -> Text<'static> {
    let mut lines = vec![
        meter_line("Hunger", needs.hunger, Color::Yellow),
        meter_line("Thirst", needs.thirst, Color::LightBlue),
        Line::from(""),
        Line::from(format!(
            "Seat: tavern floor @ {},{}",
            player_pos.x, player_pos.y
        )),
    ];

    if let Some(entity) = selected
        && let Ok((_, pos, _, name, memory, move_plan, pending_move, pending_reply)) =
            npc_query.get(entity)
    {
        lines.push(Line::from(format!(
            "Focus: {} ({},{})",
            name.0, pos.x, pos.y
        )));
        lines.push(Line::from(format!(
            "State: {}",
            movement_status(move_plan, *pending_move, *pending_reply)
        )));
        if !memory.notes.is_empty() {
            lines.push(Line::from(format!("Notes: {}", memory.notes.join(" | "))));
        }
    }

    Text::from(lines)
}

fn build_cursor_panel(
    map: &TileMap,
    cursor: Position,
    player_pos: Position,
    visible_tiles: &HashSet<Position>,
    player_query: &PlayerPanelQuery,
    npc_query: &NpcPanelQuery,
) -> Text<'static> {
    let mut lines = vec![
        Line::from(format!("Position: ({}, {})", cursor.x, cursor.y)),
        Line::from(format!(
            "Distance: {}",
            player_pos.chebyshev_distance(cursor)
        )),
    ];

    if !map.in_bounds(cursor.x, cursor.y) {
        lines.push(Line::from("Ground: out of bounds"));
        lines.push(Line::from("Actor: none"));
        lines.push(Line::from("Object: none"));
        return Text::from(lines);
    }

    let visible = visible_tiles.contains(&cursor);
    lines.push(Line::from(format!(
        "Visible: {}",
        if visible { "yes" } else { "no" }
    )));

    if visible {
        lines.push(Line::from(format!(
            "Ground: {}",
            tile_label(map.tile(cursor.x, cursor.y))
        )));
        lines.push(Line::from(format!(
            "Actor: {}",
            actor_at(cursor, player_query, npc_query).unwrap_or_else(|| "none".to_string())
        )));
        lines.push(Line::from("Object: none"));

        if let Some(entity) = npc_query
            .iter()
            .find(|(_, pos, _, _, _, _, _, _)| **pos == cursor)
            .map(|(entity, ..)| entity)
            && let Ok((_, _, _, _, memory, move_plan, pending_move, pending_reply)) =
                npc_query.get(entity)
        {
            lines.push(Line::from(format!(
                "State: {}",
                movement_status(move_plan, *pending_move, *pending_reply)
            )));
            if !memory.notes.is_empty() {
                lines.push(Line::from(format!("Notes: {}", memory.notes.join(" | "))));
            }
        }
    } else {
        lines.push(Line::from("Ground: unknown"));
        lines.push(Line::from("Actor: unseen"));
        lines.push(Line::from("Object: unknown"));
    }

    Text::from(lines)
}

fn actor_at(
    cursor: Position,
    player_query: &PlayerPanelQuery,
    npc_query: &NpcPanelQuery,
) -> Option<String> {
    if let Ok((pos, _, name)) = player_query.single()
        && *pos == cursor
    {
        return Some(name.0.clone());
    }

    npc_query
        .iter()
        .find(|(_, pos, _, _, _, _, _, _)| **pos == cursor)
        .map(|(_, _, _, name, _, _, _, _)| name.0.clone())
}

fn tile_label(tile: Tile) -> &'static str {
    match tile {
        Tile::Floor => "floorboards",
        Tile::Wall => "wall",
    }
}

fn meter_line(label: &str, value: f32, color: Color) -> Line<'static> {
    let clamped = value.clamp(0.0, 100.0);
    let filled = ((clamped / 10.0).round() as usize).min(10);
    let empty = 10usize.saturating_sub(filled);
    let bar = format!("{}{}", "#".repeat(filled), "-".repeat(empty));
    Line::from(vec![
        Span::raw(format!("{label:<6} [")),
        Span::styled(bar, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::raw(format!("] {:>3.0}", clamped)),
    ])
}

fn selected_name(selected: Option<Entity>, npc_query: &NpcPanelQuery) -> String {
    selected
        .and_then(|entity| npc_query.get(entity).ok())
        .map(|(_, _, _, name, _, _, _, _)| name.0.clone())
        .unwrap_or_else(|| "none".to_string())
}

fn movement_status(
    move_plan: &MovePlan,
    pending_move: PendingMove,
    pending_reply: PendingReply,
) -> &'static str {
    if pending_reply.waiting() {
        "replying"
    } else if pending_move.waiting() {
        "planning"
    } else if move_plan.has_steps() {
        "walking"
    } else {
        "idle"
    }
}
