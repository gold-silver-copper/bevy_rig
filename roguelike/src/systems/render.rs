use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use bevy_ratatui::RatatuiContext;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    components::{
        Glyph, Memory, MovePlan, Name, Npc, PendingAction, PendingReply, Player, Position,
    },
    map::{PropKind, Tile, TileMap},
    resources::{
        GameLog, PLAYER_VIEW_MAX_RANGE, PLAYER_VIEW_RADIUS, PlayerNeeds, UiMode, UiState,
        WorldClock,
    },
    runtime::RigRuntime,
};

const CURSOR_BLINK_HALF_PERIOD: u64 = 24;

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
        &'static PendingAction,
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
    let player_visible_tiles = map.player_visible_tiles(
        *player_pos,
        ui.cursor,
        PLAYER_VIEW_RADIUS,
        PLAYER_VIEW_MAX_RANGE,
    );
    let cursor_is_visible = cursor_blink_visible(clock.frame);

    context.draw(|frame| {
        let area = frame.area();
        let bottom_height = if ui.mode == UiMode::Talking { 18 } else { 17 };
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(bottom_height)])
            .split(area);

        let town_block = Block::default()
            .borders(Borders::ALL)
            .title("Stonehall Floor");
        let town_inner = town_block.inner(vertical[0]);
        let viewport = build_camera_viewport(&map, *player_pos, town_inner);
        let actor_map = build_actor_map(&player_query, &npc_query, &player_visible_tiles);

        frame.render_widget(town_block, vertical[0]);
        frame.render_widget(
            Paragraph::new(build_map_lines(
                &map,
                viewport,
                &player_visible_tiles,
                &actor_map,
                ui.cursor,
                cursor_is_visible,
            ))
            .wrap(Wrap { trim: false }),
            town_inner,
        );

        let bottom_block = Block::default()
            .borders(Borders::ALL)
            .title("Stonehall Interface");
        let bottom_inner = bottom_block.inner(vertical[1]);
        frame.render_widget(bottom_block, vertical[1]);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(26),
                Constraint::Min(34),
                Constraint::Length(52),
            ])
            .split(bottom_inner);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(6), Constraint::Min(11)])
            .split(columns[2]);

        frame.render_widget(
            Paragraph::new(build_status_input_panel(&ui, *player_pos, player_name))
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
                ui.cursor,
                &player_visible_tiles,
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

        if ui.show_debug {
            let debug_area = centered_rect(64, 42, vertical[0]);
            let debug_block = Block::default()
                .borders(Borders::ALL)
                .title("Debug")
                .style(Style::default().bg(Color::Black));

            frame.render_widget(Clear, debug_area);
            frame.render_widget(debug_block.clone(), debug_area);
            frame.render_widget(
                Paragraph::new(build_debug_panel(
                    &ui,
                    *player_pos,
                    player_name,
                    &player_visible_tiles,
                    &runtime,
                    &clock,
                    &npc_query,
                ))
                .block(debug_block)
                .style(Style::default().bg(Color::Black))
                .wrap(Wrap { trim: false }),
                debug_area,
            );
        }
    })?;

    Ok(())
}

fn build_camera_viewport(_map: &TileMap, player_pos: Position, area: Rect) -> CameraViewport {
    let width = i32::from(area.width.max(1));
    let height = i32::from(area.height.max(1));
    let origin_x = player_pos.x - width / 2;
    let origin_y = player_pos.y - height / 2;

    CameraViewport {
        origin: Position::new(origin_x, origin_y),
        width,
        height,
    }
}

fn build_actor_map(
    player_query: &PlayerPanelQuery,
    npc_query: &NpcPanelQuery,
    visible_tiles: &HashSet<Position>,
) -> HashMap<(i32, i32), (char, Style)> {
    let mut actor_map = HashMap::new();

    if let Ok((pos, glyph, _)) = player_query.single() {
        let style = Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD);
        actor_map.insert((pos.x, pos.y), (glyph.0, style));
    }

    for (_entity, pos, glyph, _, _, move_plan, pending_action, pending_reply) in npc_query.iter() {
        if !visible_tiles.contains(pos) {
            continue;
        }

        let style = if pending_reply.waiting() {
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD)
        } else if pending_action.waiting() {
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
    cursor_is_visible: bool,
) -> Text<'static> {
    let mut lines = Vec::with_capacity(viewport.height as usize);

    for y in viewport.origin.y..(viewport.origin.y + viewport.height) {
        let mut spans = Vec::with_capacity(viewport.width as usize);
        for x in viewport.origin.x..(viewport.origin.x + viewport.width) {
            let pos = Position::new(x, y);
            if !map.in_bounds(x, y) {
                spans.push(Span::raw(" "));
                continue;
            }

            if let Some((glyph, style)) = actor_map.get(&(x, y)) {
                let style = if pos == cursor && cursor_is_visible {
                    invert_cursor_style(*style)
                } else {
                    *style
                };
                spans.push(Span::styled(glyph.to_string(), style));
                continue;
            }

            let visible = visible_tiles.contains(&pos);
            let (glyph, mut style) = if let Some(prop) = map.prop(x, y) {
                (prop.glyph(), prop_style(prop, visible))
            } else {
                let tile = map.tile(x, y);
                (tile.glyph(), tile_style(tile, visible))
            };

            if pos == cursor && cursor_is_visible {
                style = invert_cursor_style(style);
            }

            spans.push(Span::styled(glyph.to_string(), style));
        }
        lines.push(Line::from(spans));
    }

    Text::from(lines)
}

fn build_history_panel(log: &GameLog) -> Text<'static> {
    if log.lines.is_empty() {
        return Text::from("The alehall hums low, but no one has spoken yet.");
    }

    let lines = log
        .lines
        .iter()
        .rev()
        .take(32)
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();

    Text::from(lines)
}

fn build_status_input_panel(
    ui: &UiState,
    player_pos: Position,
    player_name: &Name,
) -> Text<'static> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "rig roguelike",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {}",
                match ui.mode {
                    UiMode::Explore => "explore",
                    UiMode::Talking => "talk",
                },
            )),
        ]),
        Line::from(format!(
            "player={} @ ({}, {})",
            player_name.0, player_pos.x, player_pos.y
        )),
        Line::from(format!("cursor=({}, {})", ui.cursor.x, ui.cursor.y)),
        Line::from("move=WASD/arrows  cursor=IJKL"),
        Line::from("interact=E  talk=T/Enter"),
        Line::from("cursor center=C  wait=Space  debug=F2"),
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

fn build_debug_panel(
    ui: &UiState,
    player_pos: Position,
    player_name: &Name,
    visible_tiles: &HashSet<Position>,
    runtime: &RigRuntime,
    clock: &WorldClock,
    npc_query: &NpcPanelQuery,
) -> Text<'static> {
    let provider = runtime.current_provider();
    let readiness = if provider.ready { "ready" } else { "offline" };
    let mut lines = vec![
        Line::from(format!(
            "mode={}  time={:.1}s  turns={}  frame={}",
            match ui.mode {
                UiMode::Explore => "explore",
                UiMode::Talking => "talk",
            },
            clock.elapsed_seconds,
            clock.turn,
            clock.frame
        )),
        Line::from(format!(
            "route={} / {} [{}]",
            provider.label, provider.default_model, readiness
        )),
        Line::from(format!("provider detail={}", provider.detail)),
        Line::from(format!(
            "player={} @ ({}, {})  cursor=({}, {})",
            player_name.0, player_pos.x, player_pos.y, ui.cursor.x, ui.cursor.y
        )),
        Line::from(format!(
            "hover={}  debug={}  npc_count={}",
            hovered_name(ui.cursor, visible_tiles, npc_query),
            if ui.show_debug { "on" } else { "off" },
            npc_query.iter().count()
        )),
        Line::from("provider cycle=[ ]  quit=Q/Esc"),
    ];

    if let Some(entity) = npc_under_cursor(ui.cursor, visible_tiles, npc_query)
        && let Ok((_, pos, _, name, memory, move_plan, pending_action, pending_reply)) =
            npc_query.get(entity)
    {
        lines.push(Line::from(format!(
            "hover_pos=({}, {})  state={}  path_steps={}  target={}",
            pos.x,
            pos.y,
            movement_status(move_plan, *pending_action, *pending_reply),
            move_plan.steps.len(),
            move_plan
                .target
                .map(|target| format!("{},{}", target.x, target.y))
                .unwrap_or_else(|| "none".to_string())
        )));
        if !move_plan.trace.is_empty() {
            lines.push(Line::from(format!("trace={}", move_plan.trace)));
        }
        if !memory.notes.is_empty() {
            lines.push(Line::from(format!("notes={}", memory.notes.join(" | "))));
        }
        lines.push(Line::from(format!("hover_name={}", name.0)));
    }

    Text::from(lines)
}

fn build_stats_panel(
    needs: &PlayerNeeds,
    player_pos: Position,
    cursor: Position,
    visible_tiles: &HashSet<Position>,
    npc_query: &NpcPanelQuery,
) -> Text<'static> {
    let mut lines = vec![
        meter_line("Hunger", needs.hunger, Color::Yellow),
        meter_line("Thirst", needs.thirst, Color::LightBlue),
        Line::from(""),
        Line::from(format!(
            "Seat: stone flagstones @ {},{}",
            player_pos.x, player_pos.y
        )),
    ];

    if let Some(entity) = npc_under_cursor(cursor, visible_tiles, npc_query)
        && let Ok((_, pos, _, name, memory, move_plan, pending_action, pending_reply)) =
            npc_query.get(entity)
    {
        lines.push(Line::from(format!(
            "Nearby: {} ({},{})",
            name.0, pos.x, pos.y
        )));
        lines.push(Line::from(format!(
            "State: {}",
            movement_status(move_plan, *pending_action, *pending_reply)
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
    let tile = map.tile(cursor.x, cursor.y);
    let prop = map.prop(cursor.x, cursor.y);
    let nearby_props = nearby_prop_details(map, cursor);
    let nearby_summary = nearby_props
        .iter()
        .map(|(dir, prop)| format!("{dir}: {} [{}]", prop.label(), prop.glyph()))
        .collect::<Vec<_>>()
        .join(" | ");
    lines.push(Line::from(format!(
        "Visible: {}",
        if visible { "yes" } else { "no" }
    )));

    match prop {
        Some(prop) => {
            lines.push(Line::from(format!(
                "Object: {} [{}]",
                prop.label(),
                prop.glyph()
            )));
            lines.push(Line::from(format!("Type: {}", prop_category(prop))));
            lines.push(Line::from(format!(
                "Blocks: movement={} sight={}",
                yes_no(prop.blocks_movement()),
                yes_no(prop.blocks_sight())
            )));
            lines.push(Line::from(if player_pos.chebyshev_distance(cursor) <= 1 {
                "Press E to interact."
            } else {
                "Move closer or inspect from afar."
            }));
            lines.push(Line::from(prop.description()));
        }
        None if !nearby_props.is_empty() => {
            let (dir, prop) = nearby_props[0];
            lines.push(Line::from(format!(
                "Nearest fixture: {dir}: {} [{}]",
                prop.label(),
                prop.glyph()
            )));
            lines.push(Line::from(format!("Nearby: {}", nearby_summary)));
            lines.push(Line::from("Cursor is on open floor beside those fixtures."));
        }
        None => {
            lines.push(Line::from("Object: none"));
        }
    }

    lines.push(Line::from(format!(
        "Actor: {}",
        actor_at(cursor, player_query, npc_query).unwrap_or_else(|| {
            if visible {
                "none".to_string()
            } else {
                "unseen".to_string()
            }
        })
    )));

    lines.push(Line::from(format!(
        "Ground: {} [{}]",
        tile.label(),
        tile.glyph()
    )));
    lines.push(Line::from(if visible {
        "Line of sight: clear"
    } else {
        "Line of sight: obstructed"
    }));
    if prop.is_none() {
        lines.push(Line::from(tile.description()));
    }

    if let Some(entity) = npc_query
        .iter()
        .find(|(_, pos, _, _, _, _, _, _)| **pos == cursor)
        .map(|(entity, ..)| entity)
        && let Ok((_, _, _, _, memory, move_plan, pending_action, pending_reply)) =
            npc_query.get(entity)
    {
        lines.push(Line::from(format!(
            "State: {}",
            movement_status(move_plan, *pending_action, *pending_reply)
        )));
        if !memory.notes.is_empty() {
            lines.push(Line::from(format!("Notes: {}", memory.notes.join(" | "))));
        }
    }

    Text::from(lines)
}

fn cursor_blink_visible(frame: u64) -> bool {
    (frame / CURSOR_BLINK_HALF_PERIOD).is_multiple_of(2)
}

fn invert_cursor_style(style: Style) -> Style {
    let fg = style.fg.unwrap_or(Color::White);
    let bg = style.bg.unwrap_or(Color::Black);
    style.fg(bg).bg(fg).add_modifier(Modifier::BOLD)
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

fn nearby_prop_details(map: &TileMap, cursor: Position) -> Vec<(&'static str, PropKind)> {
    let mut labels = Vec::new();
    for (dx, dy, label) in [
        (0, -1, "north"),
        (1, 0, "east"),
        (0, 1, "south"),
        (-1, 0, "west"),
        (-1, -1, "northwest"),
        (1, -1, "northeast"),
        (-1, 1, "southwest"),
        (1, 1, "southeast"),
    ] {
        let neighbor = cursor.offset(dx, dy);
        if let Some(prop) = map.prop(neighbor.x, neighbor.y) {
            labels.push((label, prop));
        }
    }
    labels
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn prop_category(prop: PropKind) -> &'static str {
    match prop {
        PropKind::BarCounter | PropKind::Table | PropKind::Chair | PropKind::Stool => "furniture",
        PropKind::Barrel | PropKind::Bottle | PropKind::Mug => "drinkware",
        PropKind::Crate | PropKind::Shelf => "storage",
        PropKind::Candle => "light source",
        PropKind::Piano => "instrument",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn tile_style(tile: Tile, visible: bool) -> Style {
    match (tile, visible) {
        (Tile::Floor, true) => Style::default().fg(Color::Rgb(123, 102, 75)),
        (Tile::Floor, false) => Style::default().fg(Color::Rgb(48, 40, 31)),
        (Tile::Wall, true) => Style::default().fg(Color::Rgb(160, 141, 110)),
        (Tile::Wall, false) => Style::default().fg(Color::Rgb(62, 57, 44)),
    }
}

fn prop_style(prop: PropKind, visible: bool) -> Style {
    let base = match prop {
        PropKind::BarCounter => Color::Rgb(153, 108, 63),
        PropKind::Table => Color::Rgb(168, 134, 88),
        PropKind::Chair | PropKind::Stool => Color::Rgb(153, 117, 76),
        PropKind::Barrel | PropKind::Crate => Color::Rgb(131, 98, 62),
        PropKind::Bottle => Color::Rgb(82, 135, 98),
        PropKind::Mug => Color::Rgb(191, 179, 129),
        PropKind::Candle => Color::Rgb(255, 210, 110),
        PropKind::Shelf => Color::Rgb(148, 122, 94),
        PropKind::Piano => Color::Rgb(181, 181, 181),
    };

    if visible {
        Style::default().fg(base).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(54, 47, 39))
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

fn npc_under_cursor(
    cursor: Position,
    visible_tiles: &HashSet<Position>,
    npc_query: &NpcPanelQuery,
) -> Option<Entity> {
    if !visible_tiles.contains(&cursor) {
        return None;
    }

    npc_query
        .iter()
        .find(|(_, pos, _, _, _, _, _, _)| **pos == cursor)
        .map(|(entity, ..)| entity)
}

fn hovered_name(
    cursor: Position,
    visible_tiles: &HashSet<Position>,
    npc_query: &NpcPanelQuery,
) -> String {
    npc_under_cursor(cursor, visible_tiles, npc_query)
        .and_then(|entity| npc_query.get(entity).ok())
        .map(|(_, _, _, name, _, _, _, _)| name.0.clone())
        .unwrap_or_else(|| "none".to_string())
}

fn movement_status(
    move_plan: &MovePlan,
    pending_action: PendingAction,
    pending_reply: PendingReply,
) -> &'static str {
    if pending_reply.waiting() {
        "replying"
    } else if pending_action.waiting() {
        "deciding"
    } else if move_plan.has_steps() {
        "walking"
    } else {
        "idle"
    }
}
