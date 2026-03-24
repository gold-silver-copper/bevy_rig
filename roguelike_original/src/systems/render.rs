use std::collections::HashSet;

use bevy::prelude::*;
use bevy_ratatui::RatatuiContext;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap};

use crate::components::{
    Cursor, Faction, Health, Inventory, ItemKind, Name, PlayerControlled, Position, Projectile,
    ProjectileVisual, Renderable, Stamina, Viewshed, display_name, item_display_name,
};
use crate::grid_vec::GridVec;
use crate::resources::{
    BloodMap, CameraPosition, Collectibles, CombatLog, CursorPosition, GameMapResource, GameState,
    InputMode, InputState, KillCount, SOUND_RANGE, SoundEvents, SpellParticles, TurnCounter,
};
use crate::systems::input::KEYBINDINGS;
use crate::typedefs::{CoordinateUnit, MyPoint, RatColor};

const UI_BG: Color = Color::Black;

/// Lifetime (in frames) for combat particle animations.
/// Must match the lifetime used in spell.rs when creating particles.
const PARTICLE_LIFETIME: f32 = 8.0;

/// Maximum expected lifetime for smoke particles, used to normalize intensity.
const SMOKE_PARTICLE_MAX_LIFETIME: f32 = 34.0;

/// Minimum intensity for explosion particles so they remain visible.
const MIN_EXPLOSION_INTENSITY: f32 = 0.15;

/// Number of render frames between water animation symbol cycles.
/// At 60 FPS, 20 → water ripples change ~3 times per second.
const WATER_ANIM_FRAMES: u32 = 20;

/// Number of render frames over which the death red-tint effect fades in.
/// At 60 FPS, 120 → full red overlay appears over ~2 seconds.
const DEATH_FADE_FRAMES: u32 = 120;

/// Ticks and renders combat particles each frame. Also computes which
/// sound indicators should be visible on the map from `SoundEvents`.
pub fn particle_tick_system(
    mut particles: ResMut<SpellParticles>,
    sound_events: Res<SoundEvents>,
    player_query: Query<(&Position, Option<&Viewshed>), With<PlayerControlled>>,
) {
    particles.tick();

    // Compute sound indicators: audible events outside visible area.
    particles.sound_indicators.clear();
    if let Ok((pos, Some(vs))) = player_query.single() {
        let player_world = pos.as_grid_vec();
        for (event_pos, _) in &sound_events.events {
            let in_range = player_world.chebyshev_distance(*event_pos) <= SOUND_RANGE;
            let is_visible = vs.visible_tiles.contains(event_pos);
            let is_revealed = vs.revealed_tiles.contains(event_pos);
            if in_range && !is_visible && is_revealed {
                particles.sound_indicators.push(*event_pos);
            }
        }
    }
}

/// Advances the cursor blink timer each frame.
pub fn cursor_blink_system(mut cursor: ResMut<CursorPosition>) {
    cursor.tick_blink();
}

/// Renders the game map and all `Renderable` entities to the terminal.
/// Uses the player's `Viewshed` to determine tile visibility, and the
/// `revealed_tiles` set for fog-of-war memory (dimmed rendering).
///
/// Layout:
/// ┌─────────────────────────────────────────────┐
/// │              Game Area (full width)          │
/// ├────────┬────────────────────────┬───────────┤
/// │ Stats  │    Central Log         │  Info     │
/// │ HP/STA │    (combat log)        │  Inv/Vis  │
/// └────────┴────────────────────────┴───────────┘
pub fn draw_system(
    mut context: ResMut<RatatuiContext>,
    game_map: Res<GameMapResource>,
    camera: Res<CameraPosition>,
    renderables: Query<(&Position, &Renderable, Option<&Name>), Without<Projectile>>,
    player_query: Query<
        (
            &Position,
            Option<&Viewshed>,
            Option<&Health>,
            Option<&Stamina>,
            Option<&Inventory>,
        ),
        With<PlayerControlled>,
    >,
    item_query: Query<(Option<&Name>, Option<&ItemKind>), With<crate::components::Item>>,
    npc_viewsheds: Query<(&Viewshed, Option<&Faction>, &Position, Option<&Cursor>)>,
    npc_info_query: Query<(
        &Position,
        Option<&Name>,
        Option<&Faction>,
        Option<&Inventory>,
        Option<&Health>,
    )>,
    projectiles: Query<(&Position, &Renderable, &Projectile)>,
    state: Res<State<GameState>>,
    combat_log: Res<CombatLog>,
    (turn_counter, kill_count, blood_map): (Res<TurnCounter>, Res<KillCount>, Res<BloodMap>),
    spell_particles: Res<SpellParticles>,
    input_state: Res<InputState>,
    (cursor, collectibles): (Res<CursorPosition>, Res<Collectibles>),
    mut death_fade: ResMut<crate::resources::DeathFade>,
) -> Result {
    context.draw(|frame| {
        let area = frame.area();

        // ── Top-level layout: game area + inventory bar + bottom panel + command bar ──
        let bottom_panel_height = 10u16;
        let inv_bar_height = 3u16; // 1 line + 2 for border
        let cmd_bar_height = 1u16; // single line command bar at very bottom
        let vert_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(inv_bar_height),
                Constraint::Length(bottom_panel_height),
                Constraint::Length(cmd_bar_height),
            ])
            .split(area);

        let game_area = vert_chunks[0];
        let inv_bar_area = vert_chunks[1];
        let bottom_area = vert_chunks[2];
        let cmd_bar_area = vert_chunks[3];

        let render_width = game_area.width;
        let render_height = game_area.height;

        // Collect the player's visible and revealed tiles.
        let (
            visible_tiles,
            revealed_tiles,
            player_hp,
            player_stamina,
            player_inv,
            player_world_pos,
        ): (
            Option<&HashSet<MyPoint>>,
            Option<&HashSet<MyPoint>>,
            Option<&Health>,
            Option<&Stamina>,
            Option<&Inventory>,
            Option<GridVec>,
        ) = player_query
            .single()
            .ok()
            .map(|(pos, vs, hp, sta, inv)| {
                let (vis, rev) = vs
                    .map(|vs| (Some(&vs.visible_tiles), Some(&vs.revealed_tiles)))
                    .unwrap_or((None, None));
                (vis, rev, hp, sta, inv, Some(pos.as_grid_vec()))
            })
            .unwrap_or((None, None, None, None, None, None));

        let mut render_packet = game_map.0.create_render_packet_with_fog(
            &camera.0,
            render_width,
            render_height,
            visible_tiles,
            revealed_tiles,
        );

        // Overlay all renderable entities at their screen-relative positions
        let w_radius = render_width as CoordinateUnit / 2;
        let h_radius = render_height as CoordinateUnit / 2;
        let bottom_left = camera.0 - GridVec::new(w_radius, h_radius);

        // ── Water animation: animate water tiles with cycling symbols and color variety ──
        {
            let anim_frame = cursor.blink_frame();
            // Water symbol frames cycle every WATER_ANIM_FRAMES for a gentle ripple.
            let water_phase = (anim_frame / WATER_ANIM_FRAMES) as usize;
            const SHALLOW_SYMS: [&str; 4] = ["~", "~", "∼", "~"];
            const DEEP_SYMS: [&str; 4] = ["≈", "≈", "~", "≈"];

            for (screen_y, row) in render_packet.iter_mut().enumerate() {
                for (screen_x, cell) in row.iter_mut().enumerate() {
                    let world = bottom_left + GridVec::new(screen_x as i32, screen_y as i32);
                    if let Some(voxel) = game_map.0.get_voxel_at(&world) {
                        let is_visible =
                            visible_tiles.map(|vt| vt.contains(&world)).unwrap_or(true);
                        if !is_visible {
                            continue;
                        }
                        match &voxel.floor {
                            Some(crate::typeenums::Floor::ShallowWater) => {
                                // Position-based phase offset for varied ripple timing.
                                let pos_hash = ((world.x.wrapping_mul(7) ^ world.y.wrapping_mul(13))
                                    as usize)
                                    & 3;
                                let sym_idx = (water_phase + pos_hash) % SHALLOW_SYMS.len();
                                cell.0 = SHALLOW_SYMS[sym_idx].into();
                                // Slight color variation based on position.
                                let color_var = ((world.x.wrapping_mul(3)
                                    ^ world.y.wrapping_mul(11))
                                    & 15) as u8;
                                cell.1 = RatColor::Rgb(
                                    90_u8.wrapping_add(color_var),
                                    160_u8.wrapping_add(color_var / 2),
                                    210_u8.saturating_add(color_var),
                                );
                                cell.2 = RatColor::Rgb(
                                    50_u8.wrapping_add(color_var / 2),
                                    110_u8.wrapping_add(color_var),
                                    170_u8.saturating_add(color_var),
                                );
                            }
                            Some(crate::typeenums::Floor::DeepWater) => {
                                let pos_hash = ((world.x.wrapping_mul(11) ^ world.y.wrapping_mul(7))
                                    as usize)
                                    & 3;
                                let sym_idx = (water_phase + pos_hash) % DEEP_SYMS.len();
                                cell.0 = DEEP_SYMS[sym_idx].into();
                                let color_var = ((world.x.wrapping_mul(5)
                                    ^ world.y.wrapping_mul(9))
                                    & 15) as u8;
                                cell.1 = RatColor::Rgb(
                                    20_u8.wrapping_add(color_var),
                                    70_u8.wrapping_add(color_var / 2),
                                    170_u8.saturating_add(color_var),
                                );
                                cell.2 = RatColor::Rgb(
                                    10_u8.wrapping_add(color_var / 2),
                                    35_u8.wrapping_add(color_var),
                                    110_u8.saturating_add(color_var),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        /// Returns `true` if the screen-relative coordinate is within the render area.
        #[inline]
        fn in_bounds(screen: GridVec, render_width: u16, render_height: u16) -> bool {
            (0..render_width as CoordinateUnit).contains(&screen.x)
                && (0..render_height as CoordinateUnit).contains(&screen.y)
        }

        // Tint NPC field-of-view with colored hues.
        // All NPCs (hostile or not): tint only a small arc in their facing direction.
        // This shows just enough of their FOV to indicate where they're looking.
        // Animals (Wildlife) and Civilians are excluded from FOV highlighting.
        {
            /// Maximum Chebyshev distance from an NPC for direction tint.
            const FOV_TINT_ARC_RADIUS: i32 = 3;

            let mut enemy_visible: HashSet<MyPoint> = HashSet::new();
            for (vs, faction, npc_pos, npc_cursor) in &npc_viewsheds {
                if faction.is_some_and(|f| matches!(f, Faction::Wildlife | Faction::Civilians)) {
                    continue;
                }
                if vs.dirty {
                    continue;
                }
                // Only tint FOV for NPCs that the player can currently see
                let npc_gv = npc_pos.as_grid_vec();
                let npc_in_player_view = visible_tiles
                    .map(|vt| vt.contains(&npc_gv))
                    .unwrap_or(false);
                if !npc_in_player_view {
                    continue;
                }
                let Some(look_delta) = npc_cursor
                    .map(|cursor| (cursor.pos - npc_gv).king_step())
                    .filter(|delta| !delta.is_zero())
                else {
                    continue;
                };

                // Tint only a small arc in facing direction — just enough to
                // see where people are looking.
                for &tile in &vs.visible_tiles {
                    let diff = tile - npc_gv;
                    if diff.x.abs().max(diff.y.abs()) <= FOV_TINT_ARC_RADIUS {
                        let dot =
                            diff.x as i64 * look_delta.x as i64 + diff.y as i64 * look_delta.y as i64;
                        if dot <= 0 && diff != GridVec::ZERO {
                            continue;
                        }
                        enemy_visible.insert(tile);
                    }
                }
            }
            for (screen_y, row) in render_packet.iter_mut().enumerate() {
                for (screen_x, cell) in row.iter_mut().enumerate() {
                    let world = bottom_left + GridVec::new(screen_x as i32, screen_y as i32);
                    let in_player_view =
                        visible_tiles.map(|vt| vt.contains(&world)).unwrap_or(false);
                    if in_player_view
                        && enemy_visible.contains(&world)
                        && let RatColor::Rgb(r, g, b) = cell.2
                    {
                        cell.2 = RatColor::Rgb(
                            r.saturating_add(40),
                            g.saturating_sub(10),
                            b.saturating_sub(10),
                        );
                    }
                }
            }
        }

        // Overlay blood stains on the render packet (after map, before entities).
        {
            let current_turn = turn_counter.0;
            for (&world_pos, &blood_turn) in &blood_map.stains {
                let in_view = visible_tiles
                    .map(|vt| vt.contains(&world_pos))
                    .unwrap_or(false);
                if !in_view {
                    continue;
                }
                let screen = world_pos - bottom_left;
                if in_bounds(screen, render_width, render_height) {
                    let age = current_turn.saturating_sub(blood_turn);
                    // Lerp from bright red (200,0,0) to dark red (80,20,10) over 50 turns
                    let t = (age as f32 / 50.0).min(1.0);
                    let r = (200.0 - 120.0 * t) as u8;
                    let g = (20.0 * t) as u8;
                    let b = (10.0 * t) as u8;
                    let bg = render_packet[screen.y as usize][screen.x as usize].2;
                    render_packet[screen.y as usize][screen.x as usize] =
                        (".".into(), RatColor::Rgb(r, g, b), bg);
                }
            }
        }

        // Collect visible entities for the info panel.
        let mut visible_entity_infos: Vec<(String, RatColor, RatColor, String)> = Vec::new();

        let mut visible_renderables: Vec<(i32, GridVec, &Renderable, Option<&Name>)> = Vec::new();
        for (pos, renderable, name) in &renderables {
            let world = pos.as_grid_vec();
            let screen = world - bottom_left;

            if !in_bounds(screen, render_width, render_height) {
                continue;
            }
            // Only draw entities that are currently visible (not merely revealed)
            let entity_visible = visible_tiles.map(|vt| vt.contains(&world)).unwrap_or(true);
            if !entity_visible {
                continue;
            }

            let draw_priority = match renderable.symbol.as_str() {
                "@" => 2,
                "X" => 1,
                _ => 0,
            };
            visible_renderables.push((draw_priority, screen, renderable, name));

            // Collect for visible entities panel.
            let full_name = display_name(name).to_string();
            visible_entity_infos.push((
                renderable.symbol.clone(),
                renderable.fg,
                renderable.bg,
                full_name,
            ));
        }
        visible_renderables.sort_by_key(|(priority, _, _, _)| *priority);
        for (_, screen, renderable, _) in visible_renderables {
            let bg = render_packet[screen.y as usize][screen.x as usize].2;
            render_packet[screen.y as usize][screen.x as usize] =
                (renderable.symbol.clone(), renderable.fg, bg);
        }

        // Overlay combat particles on the render packet.
        for (particle_pos, lifetime, delay, is_sand, _vx, _vy) in &spell_particles.particles {
            if *delay > 0 {
                continue; // not yet visible
            }
            let screen = *particle_pos - bottom_left;
            if in_bounds(screen, render_width, render_height) {
                let visible = visible_tiles
                    .map(|vt| vt.contains(particle_pos))
                    .unwrap_or(true);
                if visible {
                    let bg = render_packet[screen.y as usize][screen.x as usize].2;
                    if *is_sand {
                        // Smoke plume: gray billows that linger longer than the
                        // opening flash so a breached wall reads as smoke, not
                        // just a brief spark burst.
                        let intensity =
                            (*lifetime as f32 / SMOKE_PARTICLE_MAX_LIFETIME).clamp(0.2, 1.0);
                        let (symbol, r, g, b) = if *lifetime > 18 {
                            (
                                "@",
                                (190.0 * intensity) as u8,
                                (190.0 * intensity) as u8,
                                (190.0 * intensity) as u8,
                            )
                        } else if *lifetime > 10 {
                            (
                                "o",
                                (160.0 * intensity) as u8,
                                (160.0 * intensity) as u8,
                                (165.0 * intensity) as u8,
                            )
                        } else if *lifetime > 5 {
                            (
                                "*",
                                (125.0 * intensity) as u8,
                                (125.0 * intensity) as u8,
                                (130.0 * intensity) as u8,
                            )
                        } else {
                            (
                                ".",
                                (90.0 * intensity) as u8,
                                (90.0 * intensity) as u8,
                                (95.0 * intensity) as u8,
                            )
                        };
                        render_packet[screen.y as usize][screen.x as usize] =
                            (symbol.into(), RatColor::Rgb(r, g, b), bg);
                    } else {
                        // Explosion/fire particle: visible movement with changing symbols
                        let intensity = (*lifetime as f32 / PARTICLE_LIFETIME)
                            .clamp(MIN_EXPLOSION_INTENSITY, 1.0);
                        let r = (255.0 * intensity) as u8;
                        let g = (165.0 * intensity) as u8;
                        let symbol = "*";
                        render_packet[screen.y as usize][screen.x as usize] =
                            (symbol.into(), RatColor::Rgb(r, g, 0), bg);
                    }
                }
            }
        }

        // Overlay cursor position with blinking color inversion.
        // The cursor does not draw a character — it inverts fg/bg colors when visible.
        let cursor_blink_visible = cursor.blink_visible();
        {
            let cursor_screen = cursor.pos - bottom_left;
            if in_bounds(cursor_screen, render_width, render_height) && cursor_blink_visible {
                let sx = cursor_screen.x as usize;
                let sy = cursor_screen.y as usize;
                // Invert fg and bg colors for the cursor cell.
                let (sym, fg, bg) = &render_packet[sy][sx];
                render_packet[sy][sx] = (sym.clone(), *bg, *fg);
            }
        }

        // Render projectile entities on the map with fast blinking effect.
        for (proj_pos, proj_render, proj) in &projectiles {
            let screen = proj_pos.as_grid_vec() - bottom_left;
            if in_bounds(screen, render_width, render_height) {
                let visible = visible_tiles
                    .map(|vt| vt.contains(&proj_pos.as_grid_vec()))
                    .unwrap_or(true);
                if visible {
                    let bg = render_packet[screen.y as usize][screen.x as usize].2;
                    // Fast blink: alternate between bright and dim every 3 frames
                    let blink_bright = (cursor.blink_frame() / 3).is_multiple_of(2);
                    let fg = if blink_bright {
                        proj_render.fg
                    } else {
                        // Dim version of the projectile color
                        if let RatColor::Rgb(r, g, b) = proj_render.fg {
                            RatColor::Rgb(r / 2, g / 2, b / 2)
                        } else {
                            proj_render.fg
                        }
                    };
                    // Render head
                    render_packet[screen.y as usize][screen.x as usize] =
                        (proj_render.symbol.clone(), fg, bg);

                    // Render tail for fast moving linear projectiles.
                    if matches!(
                        proj.visual,
                        ProjectileVisual::BulletTrail | ProjectileVisual::ShrapnelTrail
                    )
                        && let Some(tail) = proj.tail_pos
                    {
                        let tail_screen = tail - bottom_left;
                        if in_bounds(tail_screen, render_width, render_height) {
                            let tail_visible =
                                visible_tiles.map(|vt| vt.contains(&tail)).unwrap_or(true);
                            if tail_visible {
                                let tail_bg =
                                    render_packet[tail_screen.y as usize][tail_screen.x as usize].2;
                                let tail_fg = if let RatColor::Rgb(r, g, b) = proj_render.fg {
                                    if blink_bright {
                                        RatColor::Rgb(
                                            r.saturating_sub(60),
                                            g.saturating_sub(60),
                                            b.saturating_sub(60),
                                        )
                                    } else {
                                        RatColor::Rgb(r / 3, g / 3, b / 3)
                                    }
                                } else {
                                    proj_render.fg
                                };
                                let tail_symbol = if proj.visual == ProjectileVisual::ShrapnelTrail
                                {
                                    "∙"
                                } else {
                                    "·"
                                };
                                render_packet[tail_screen.y as usize][tail_screen.x as usize] =
                                    (tail_symbol.into(), tail_fg, tail_bg);
                            }
                        }
                    }
                }
            }
        }

        // Render sound indicators ("!") for audible events outside visible area.
        // (Sound indicators are pre-computed in particle_tick_system.)
        for event_pos in &spell_particles.sound_indicators {
            let screen = *event_pos - bottom_left;
            if in_bounds(screen, render_width, render_height) {
                let bg = render_packet[screen.y as usize][screen.x as usize].2;
                render_packet[screen.y as usize][screen.x as usize] =
                    ("!".into(), RatColor::Rgb(255, 255, 0), bg);
            }
        }

        // ── Final pass: re-draw entities OVER everything else ──
        // PlayerControlled and NPC symbols should always be drawn over every other symbol
        // (particles, projectiles, blood, etc.)
        let mut final_pass_renderables: Vec<(i32, GridVec, &Renderable)> = Vec::new();
        for (pos, renderable, _) in &renderables {
            let world = pos.as_grid_vec();
            let screen = world - bottom_left;
            if !in_bounds(screen, render_width, render_height) {
                continue;
            }
            let entity_visible = visible_tiles.map(|vt| vt.contains(&world)).unwrap_or(true);
            if !entity_visible {
                continue;
            }
            let draw_priority = match renderable.symbol.as_str() {
                "@" => 2,
                "X" => 1,
                _ => 0,
            };
            final_pass_renderables.push((draw_priority, screen, renderable));
        }
        final_pass_renderables.sort_by_key(|(priority, _, _)| *priority);
        for (_, screen, renderable) in final_pass_renderables {
            let bg = render_packet[screen.y as usize][screen.x as usize].2;
            render_packet[screen.y as usize][screen.x as usize] =
                (renderable.symbol.clone(), renderable.fg, bg);
        }

        // ── Apply per-tile color noise (final step before rendering) ──
        // Each tile gets a deterministic ±TILE_COLOR_NOISE_RANGE jitter
        // on every RGB channel, seeded by its world coordinates.
        {
            use crate::noise::{TILE_COLOR_NOISE_RANGE, tile_color_noise};
            for (screen_y, row) in render_packet.iter_mut().enumerate() {
                for (screen_x, cell) in row.iter_mut().enumerate() {
                    let world = bottom_left + GridVec::new(screen_x as i32, screen_y as i32);
                    if let RatColor::Rgb(r, g, b) = cell.1 {
                        let (nr, ng, nb) =
                            tile_color_noise(r, g, b, world.x, world.y, TILE_COLOR_NOISE_RANGE);
                        cell.1 = RatColor::Rgb(nr, ng, nb);
                    }
                    if let RatColor::Rgb(r, g, b) = cell.2 {
                        let (nr, ng, nb) =
                            tile_color_noise(r, g, b, world.x, world.y, TILE_COLOR_NOISE_RANGE);
                        cell.2 = RatColor::Rgb(nr, ng, nb);
                    }
                }
            }
        }

        // ── Death tint effect ──────────────────────────────────────
        // When the player dies, apply a restrained red wash instead of
        // a heavy blackout so the player can still watch the map.
        let player_is_dead = player_hp.is_some_and(|hp| hp.is_dead());
        if player_is_dead {
            death_fade.frames = death_fade.frames.saturating_add(1);
            let fade = (death_fade.frames as f32 / DEATH_FADE_FRAMES as f32).min(1.0) * 0.35;

            let player_screen = player_world_pos.map(|pw| pw - bottom_left);
            for (screen_y, row) in render_packet.iter_mut().enumerate() {
                for (screen_x, cell) in row.iter_mut().enumerate() {
                    // Compute distance from the player in screen coordinates.
                    let dist = if let Some(ps) = player_screen {
                        let dx = (screen_x as i32 - ps.x).abs();
                        let dy = (screen_y as i32 - ps.y).abs();
                        dx.max(dy) // Chebyshev distance
                    } else {
                        // Player off-screen — treat all tiles as very far.
                        40
                    };

                    // Strongest near the player, but never fades to black.
                    const DEATH_TINT_RADIUS: i32 = 18;
                    let focus = 1.0 - (dist as f32 / DEATH_TINT_RADIUS as f32).min(1.0);
                    let tint = fade * (0.65 + focus * 0.35);

                    if let RatColor::Rgb(r, g, b) = cell.1 {
                        let nr = ((r as f32 * (1.0 - tint * 0.08)) + 36.0 * tint) as u8;
                        let ng = (g as f32 * (1.0 - tint * 0.22)) as u8;
                        let nb = (b as f32 * (1.0 - tint * 0.18)) as u8;
                        cell.1 = RatColor::Rgb(nr, ng, nb);
                    }
                    if let RatColor::Rgb(r, g, b) = cell.2 {
                        let nr = ((r as f32 * (1.0 - tint * 0.10)) + 28.0 * tint) as u8;
                        let ng = (g as f32 * (1.0 - tint * 0.28)) as u8;
                        let nb = (b as f32 * (1.0 - tint * 0.24)) as u8;
                        cell.2 = RatColor::Rgb(nr, ng, nb);
                    }
                }
            }
        } else {
            death_fade.frames = 0;
        }

        let mut render_lines = Vec::new();

        for y in 0..render_height as usize {
            if y < render_packet.len() {
                let spans: Vec<Span> = render_packet[y]
                    .iter()
                    .map(|gt| Span::from(gt.0.clone()).fg(gt.1).bg(gt.2))
                    .collect();
                render_lines.push(Line::from(spans));
            }
        }

        // Reverse so that higher Y values are at the top (standard roguelike convention)
        render_lines.reverse();

        let game_bg = if player_is_dead {
            let fade = (death_fade.frames as f32 / DEATH_FADE_FRAMES as f32).min(1.0) * 0.25;
            RatColor::Rgb((18.0 * fade) as u8, 0, 0)
        } else {
            RatColor::Black
        };
        frame.render_widget(
            Paragraph::new(Text::from(render_lines)).bg(game_bg),
            game_area,
        );

        // Collect inventory item names and kinds for the bottom panel and inventory overlay.
        let inv_item_info: Vec<(String, String)> = player_inv
            .map(|inv| {
                inv.items
                    .iter()
                    .map(|&ent| {
                        let name = item_query.get(ent).ok().and_then(|(n, _)| n);
                        let name_str = item_display_name(name).to_string();
                        let desc = item_query.get(ent).ok().and_then(|(_, k)| k).map_or(
                            "".to_string(),
                            |k| match k {
                                ItemKind::Gun {
                                    loaded,
                                    capacity,
                                    caliber,
                                    ..
                                } => format!("{loaded}/{capacity} {caliber}"),
                                _ => "".to_string(),
                            },
                        );
                        (name_str, desc)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // ── Cursor tile info (replaces old visible props + visible entities panels) ──
        // Gather everything at the cursor position for the new two-column info panel.
        let cursor_world = cursor.pos;
        let cursor_ground: String;
        let cursor_prop: String;
        let mut cursor_effects: Vec<String> = Vec::new(); // smoke, fire, sand, etc.
        let mut cursor_ground_items: Vec<String> = Vec::new();
        let mut cursor_npc_name: String = String::new();
        let mut cursor_npc_faction: String = String::new();
        let mut cursor_npc_inv: Vec<String> = Vec::new();
        let mut cursor_npc_hp: String = String::new();
        let mut cursor_npc_hostile = false;

        // Ground & prop from map
        if let Some(voxel) = game_map.0.get_voxel_at(&cursor_world) {
            cursor_ground = voxel
                .floor
                .as_ref()
                .map_or("(void)".into(), |f| format!("{f:?}"));
            cursor_prop = voxel
                .props
                .as_ref()
                .map_or("(none)".into(), |p| format!("{p}"));
            // Effects
            if let Some(ref floor) = voxel.floor {
                match floor {
                    crate::typeenums::Floor::SandCloud => cursor_effects.push("Smoke Cloud".into()),
                    crate::typeenums::Floor::Fire => cursor_effects.push("Fire".into()),
                    crate::typeenums::Floor::ShallowWater => {
                        cursor_effects.push("Shallow Water".into())
                    }
                    crate::typeenums::Floor::DeepWater => cursor_effects.push("Deep Water".into()),
                    _ => {}
                }
            }
        } else {
            cursor_ground = "(void)".into();
            cursor_prop = "(none)".into();
        }

        // NPCs and ground items at cursor
        for (npc_pos, npc_name, npc_fac, npc_inv, npc_hp) in &npc_info_query {
            if npc_pos.as_grid_vec() == cursor_world && cursor_npc_name.is_empty() {
                cursor_npc_name = display_name(npc_name).to_string();
                cursor_npc_faction = npc_fac.map_or("".into(), |f| format!("{f:?}"));
                // Hostility is faction-based: all NPCs with a faction are hostile to the player.
                cursor_npc_hostile = npc_fac.is_some();
                if let Some(hp) = npc_hp {
                    cursor_npc_hp = format!("{}/{}", hp.current, hp.max);
                }
                if let Some(inv) = npc_inv {
                    for &item_ent in &inv.items {
                        let iname = item_query
                            .get(item_ent)
                            .ok()
                            .and_then(|(n, _)| n)
                            .map_or("?".into(), |n| n.0.clone());
                        cursor_npc_inv.push(iname);
                    }
                }
            }
        }

        // Ground items at cursor (items with Position that are NOT in NPC inventories)
        for (pos, _renderable, name) in &renderables {
            if pos.as_grid_vec() == cursor_world {
                let full_name = display_name(name).to_string();
                // Skip items that are NPCs (already shown above)
                if full_name != cursor_npc_name && !full_name.is_empty() {
                    cursor_ground_items.push(full_name);
                }
            }
        }

        // ── Bottom Panel ────────────────────────────────────────
        render_bottom_panel(
            frame,
            bottom_area,
            player_hp,
            player_stamina,
            &combat_log,
            &turn_counter,
            &kill_count,
            &collectibles,
            // cursor info
            &cursor_ground,
            &cursor_prop,
            &cursor_npc_name,
            &cursor_npc_faction,
            &cursor_npc_hp,
            cursor_npc_hostile,
            &cursor_npc_inv,
            &cursor_ground_items,
            &cursor_effects,
        );

        // ── Inventory Bar (wide, horizontal) ────────────────────
        render_inventory_bar(frame, inv_bar_area, &inv_item_info);

        // ── Command Bar (very bottom) ───────────────────────────
        render_command_bar(frame, cmd_bar_area, &input_state);

        // ── Overlays ────────────────────────────────────────────

        // Show ESC menu overlay when in EscMenu mode (replaces old PAUSED overlay)
        if input_state.mode == InputMode::EscMenu {
            render_esc_menu_overlay(frame, game_area);
        }

        // Show "VICTORY" overlay centered on game area when the gate is destroyed
        if *state.get() == GameState::Victory {
            let label = " VICTORY! You escaped Yerba Buena! Press R to restart. ";
            let label_width = label.len() as u16;
            if render_width >= label_width && render_height >= 1 {
                let cx = game_area.x + (render_width - label_width) / 2;
                let cy = game_area.y + render_height / 2;
                let victory_area = Rect {
                    x: cx,
                    y: cy,
                    width: label_width,
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(Line::from(label).bold()).on_yellow(),
                    victory_area,
                );
            }
        }

        // Show "YOU DIED" overlay when the player has fallen.
        // Show it right above the UI panel, not in the center of the screen.
        if *state.get() == GameState::Dead || player_is_dead {
            let label = " YOU DIED — Press R to restart ";
            let label_width = label.len() as u16;
            if render_width >= label_width && game_area.height >= 1 {
                let cx = game_area.x + (render_width - label_width) / 2;
                // Position right above the UI (bottom of game area)
                let cy = game_area.y + game_area.height.saturating_sub(1);
                let death_area = Rect {
                    x: cx,
                    y: cy,
                    width: label_width,
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(Line::from(label).bold()).on_red(),
                    death_area,
                );
            }
        }

        // Show welcome screen at game start
        if input_state.welcome_visible {
            render_welcome_overlay(frame, game_area);
        }
    })?;

    Ok(())
}

/// Renders the bottom panel with stats, central combat log, and cursor info.
/// Layout: [Stats | Central Log | Cursor Info (two columns)]
fn render_bottom_panel(
    frame: &mut ratatui::Frame,
    area: Rect,
    player_hp: Option<&Health>,
    player_stamina: Option<&Stamina>,
    combat_log: &CombatLog,
    turn_counter: &TurnCounter,
    kill_count: &KillCount,
    collectibles: &Collectibles,
    // Cursor info
    cursor_ground: &str,
    cursor_prop: &str,
    cursor_npc_name: &str,
    cursor_npc_faction: &str,
    cursor_npc_hp: &str,
    cursor_npc_hostile: bool,
    cursor_npc_inv: &[String],
    cursor_ground_items: &[String],
    cursor_effects: &[String],
) {
    // Split bottom panel into three horizontal columns: stats | log | cursor info
    let horiz_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // Stats column (HP, Stamina)
            Constraint::Min(1),     // Central log (wide, fills remaining space)
            Constraint::Length(40), // Cursor info (two sub-columns)
        ])
        .split(area);

    let stats_area = horiz_chunks[0];
    let log_area = horiz_chunks[1];
    let cursor_info_area = horiz_chunks[2];

    // ── Stats Column (left) ─────────────────────────────────────
    render_stats_column(frame, stats_area, player_hp, player_stamina, collectibles);

    // ── Central Log (middle) ────────────────────────────────────
    let log_height = log_area.height.saturating_sub(2) as usize;
    let log_lines: Vec<Line> = combat_log
        .recent(log_height.max(1))
        .into_iter()
        .map(|s| Line::from(format!(" {s}")).dark_gray())
        .collect();

    let title = format!(" Log | Tick:{} Kills:{} ", turn_counter.0, kill_count.0);
    frame.render_widget(
        Paragraph::new(if log_lines.is_empty() {
            vec![Line::from(" (no events)".dark_gray())]
        } else {
            log_lines
        })
        .style(Style::default().bg(UI_BG))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().bg(UI_BG)),
        )
        .wrap(Wrap { trim: true }),
        log_area,
    );

    // ── Cursor Info (right, two sub-columns) ────────────────────
    render_cursor_info(
        frame,
        cursor_info_area,
        cursor_ground,
        cursor_prop,
        cursor_npc_name,
        cursor_npc_faction,
        cursor_npc_hp,
        cursor_npc_hostile,
        cursor_npc_inv,
        cursor_ground_items,
        cursor_effects,
    );
}

/// Renders the stats column (HP, Stamina gauges stacked vertically).
fn render_stats_column(
    frame: &mut ratatui::Frame,
    area: Rect,
    player_hp: Option<&Health>,
    player_stamina: Option<&Stamina>,
    collectibles: &Collectibles,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // HP gauge (compact, no border)
            Constraint::Length(1), // Stamina gauge
            Constraint::Length(1), // Collectibles row 1
            Constraint::Length(1), // Collectibles row 2
            Constraint::Length(1), // Collectibles row 3
            Constraint::Length(1), // Collectibles row 4
        ])
        .split(
            Block::default()
                .borders(Borders::ALL)
                .title("Stats")
                .style(Style::default().bg(UI_BG))
                .inner(area),
        );

    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title("Stats")
            .style(Style::default().bg(UI_BG)),
        area,
    );

    // HP
    if let Some(hp) = player_hp {
        let percent = if hp.max > 0 {
            ((hp.current.max(0) as f64 / hp.max as f64) * 100.0).round() as u16
        } else {
            0
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Red).bg(Color::DarkGray))
            .percent(percent.min(100))
            .label(
                Span::from(format!("HP {}/{}", hp.current, hp.max))
                    .style(Style::default().fg(Color::White)),
            );
        frame.render_widget(gauge, chunks[0]);
    }

    // Stamina
    if let Some(stamina) = player_stamina {
        let percent = if stamina.max > 0 {
            ((stamina.current.max(0) as f64 / stamina.max as f64) * 100.0).round() as u16
        } else {
            0
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Blue).bg(Color::DarkGray))
            .percent(percent.min(100))
            .label(
                Span::from(format!("STA {}/{}", stamina.current, stamina.max))
                    .style(Style::default().fg(Color::White)),
            );
        frame.render_widget(gauge, chunks[1]);
    }

    // Collectibles — 3 entries per row
    let row1 = format!(
        "Cap:{} Pdr:{} Arr:{}",
        collectibles.caps, collectibles.powder, collectibles.arrows,
    );
    let row2 = format!(
        ".31:{} .36:{} .44:{}",
        collectibles.bullets_31, collectibles.bullets_36, collectibles.bullets_44,
    );
    let row3 = format!(
        ".50:{} .58:{} .577:{}",
        collectibles.bullets_50, collectibles.bullets_58, collectibles.bullets_577,
    );
    let row4 = format!(
        ".69:{}",
        collectibles.bullets_69,
    );
    frame.render_widget(
        Paragraph::new(Line::from(row1).white()).style(Style::default().bg(UI_BG)),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(Line::from(row2).white()).style(Style::default().bg(UI_BG)),
        chunks[3],
    );
    frame.render_widget(
        Paragraph::new(Line::from(row3).white()).style(Style::default().bg(UI_BG)),
        chunks[4],
    );
    frame.render_widget(
        Paragraph::new(Line::from(row4).white()).style(Style::default().bg(UI_BG)),
        chunks[5],
    );
}

/// Renders the cursor info panel as a two-column display.
/// Left: Prop, Ground, NPC info. Right: Ground items, effects.
fn render_cursor_info(
    frame: &mut ratatui::Frame,
    area: Rect,
    cursor_ground: &str,
    cursor_prop: &str,
    cursor_npc_name: &str,
    cursor_npc_faction: &str,
    cursor_npc_hp: &str,
    cursor_npc_hostile: bool,
    cursor_npc_inv: &[String],
    cursor_ground_items: &[String],
    cursor_effects: &[String],
) {
    let inner = Block::default()
        .borders(Borders::ALL)
        .title("Cursor")
        .style(Style::default().bg(UI_BG))
        .inner(area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title("Cursor")
            .style(Style::default().bg(UI_BG)),
        area,
    );

    // Split into two columns
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // ── Left column: Prop, Ground, NPC ──
    let max_lines = cols[0].height as usize;
    let mut left_lines: Vec<Line> = Vec::new();
    left_lines.push(Line::from(vec![
        Span::from(" Ground:").bold().dark_gray(),
        Span::from(format!(" {cursor_ground}")).white(),
    ]));
    left_lines.push(Line::from(vec![
        Span::from(" Prop:").bold().dark_gray(),
        Span::from(format!(" {cursor_prop}")).white(),
    ]));
    if !cursor_npc_name.is_empty() {
        let hostile_tag = if cursor_npc_hostile { " [!]" } else { "" };
        left_lines.push(Line::from(vec![
            Span::from(format!(" {cursor_npc_name}{hostile_tag}")).yellow(),
        ]));
        if !cursor_npc_faction.is_empty() {
            left_lines.push(Line::from(vec![
                Span::from("  Fac:").dark_gray(),
                Span::from(format!(" {cursor_npc_faction}")).white(),
            ]));
        }
        if !cursor_npc_hp.is_empty() {
            left_lines.push(Line::from(vec![
                Span::from("  HP:").dark_gray(),
                Span::from(format!(" {cursor_npc_hp}")).white(),
            ]));
        }
        for item_name in cursor_npc_inv.iter() {
            if left_lines.len() >= max_lines {
                break;
            }
            left_lines.push(Line::from(vec![
                Span::from(format!("  {item_name}")).white(),
            ]));
        }
    }
    left_lines.truncate(max_lines);
    frame.render_widget(
        Paragraph::new(left_lines).style(Style::default().bg(UI_BG)),
        cols[0],
    );

    // ── Right column: Ground items, effects ──
    let mut right_lines: Vec<Line> = Vec::new();
    if !cursor_ground_items.is_empty() {
        right_lines.push(Line::from(" Items:").bold().dark_gray());
        for item in cursor_ground_items.iter().take(max_lines.saturating_sub(2)) {
            right_lines.push(Line::from(format!("  {item}")).white());
        }
    }
    if !cursor_effects.is_empty() {
        right_lines.push(Line::from(" Effects:").bold().dark_gray());
        for effect in cursor_effects {
            if right_lines.len() >= max_lines {
                break;
            }
            right_lines.push(Line::from(format!("  {effect}")).yellow());
        }
    }
    if right_lines.is_empty() {
        right_lines.push(Line::from(" (empty)").dark_gray());
    }
    right_lines.truncate(max_lines);
    frame.render_widget(
        Paragraph::new(right_lines).style(Style::default().bg(UI_BG)),
        cols[1],
    );
}

/// Renders the inventory bar as a wide horizontal bar showing usable items.
fn render_inventory_bar(
    frame: &mut ratatui::Frame,
    area: Rect,
    inv_item_info: &[(String, String)],
) {
    let mut spans: Vec<Span> = Vec::new();
    if inv_item_info.is_empty() {
        spans.push(Span::from(" (empty)").dark_gray());
    } else {
        for (i, (name, desc)) in inv_item_info.iter().enumerate() {
            if i > 0 {
                spans.push(Span::from("  ").dark_gray());
            }
            spans.push(Span::from(format!("{}:", i + 1)).bold().yellow());
            spans.push(Span::from(name.to_string()).white());
            if !desc.is_empty() {
                spans.push(Span::from(format!("({desc})")).dark_gray());
            }
        }
    }
    let line = Line::from(spans);
    frame.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(UI_BG))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Inventory 1-0:Use")
                    .style(Style::default().bg(UI_BG)),
            ),
        area,
    );
}

/// Renders the command bar at the very bottom of the screen showing all key commands.
/// Labels are derived from the live `KEYBINDINGS` array in input.rs so they
/// automatically update if bindings change.
fn render_command_bar(frame: &mut ratatui::Frame, area: Rect, input_state: &InputState) {
    let spans = if input_state.mode == InputMode::EscMenu {
        vec![
            Span::from(" Q").bold().yellow(),
            Span::from(":Resume ").dark_gray(),
            Span::from("R").bold().yellow(),
            Span::from(":Restart ").dark_gray(),
        ]
    } else {
        let mut s: Vec<Span> = Vec::new();
        for binding in KEYBINDINGS {
            if !s.is_empty() {
                s.push(Span::from(" ").dark_gray());
            }
            s.push(Span::from(format!("{}", binding.key)).bold().yellow());
            s.push(Span::from(format!(":{} ", binding.name)).dark_gray());
        }
        s
    };
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line).on_black(), area);
}

/// Renders the welcome screen shown at game start.
fn render_welcome_overlay(frame: &mut ratatui::Frame, game_area: Rect) {
    let binding_count = KEYBINDINGS.len() as u16;
    let w = 62u16.min(game_area.width.saturating_sub(4));
    let h = (binding_count + 29).min(game_area.height.saturating_sub(4));

    if w < 20 || h < 10 {
        return;
    }

    let cx = game_area.x + (game_area.width.saturating_sub(w)) / 2;
    let cy = game_area.y + (game_area.height.saturating_sub(h)) / 2;
    let welcome_area = Rect {
        x: cx,
        y: cy,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, welcome_area);

    let mut lines = vec![
        Line::from(""),
        Line::from("  -*-  ESCAPE FROM YERBA BUENA  -*-").bold().yellow(),
        Line::from(""),
        Line::from("  You were in a riverfront cell when the").white(),
        Line::from("  wall came apart in fire and smoke.").white(),
        Line::from("  Somebody broke you out, but that does").white(),
        Line::from("  not make them allies. Four gangs are").white(),
        Line::from("  already tearing the city to pieces.").white(),
        Line::from(""),
        Line::from("  HOW TO PLAY").bold().yellow(),
        Line::from("  Reach any edge of Yerba Buena alive to escape.").white(),
        Line::from("  Aim with IJKL, then press 1-0 to use the").white(),
        Line::from("  matching item slot in the inventory bar.").white(),
        Line::from("  Guns need the right bullets, caps, and powder.").white(),
        Line::from("  Booze heals. Grenades and molotovs clear rooms.").white(),
        Line::from(""),
        Line::from("  Stay moving, pick your fights, take what").yellow(),
        Line::from("  you need, and get out before the whole").yellow(),
        Line::from("  town turns into a killbox. Nobody in").yellow(),
        Line::from("  this city is coming to save you.").yellow(),
        Line::from("  Roundhouse (F) also smashes props around").yellow(),
        Line::from("  you, opening space when a room gets tight.").yellow(),
        Line::from(""),
    ];
    for binding in KEYBINDINGS {
        let desc = if binding.name == "Roundhouse" {
            "Roundhouse (breaks nearby props)"
        } else {
            binding.name
        };
        lines.push(Line::from(vec![
            Span::from(format!("  {:<16}", binding.key)).bold().yellow(),
            Span::from(desc.to_string()).white(),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("  Press any key to begin...").dark_gray());

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Welcome ")
                    .border_style(
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                    ),
            )
            .wrap(Wrap { trim: false })
            .on_black(),
        welcome_area,
    );
}

/// Renders the ESC menu overlay with Resume and Restart options.
fn render_esc_menu_overlay(frame: &mut ratatui::Frame, game_area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(""));
    lines.push(Line::from("  R — Restart").bold().yellow());
    lines.push(Line::from(""));
    lines.push(Line::from("  Q — Resume").white());
    lines.push(Line::from(""));
    lines.push(Line::from("  HOW TO PLAY").bold().yellow());
    lines.push(Line::from("  Escape by reaching any edge of Yerba Buena alive.").white());
    lines.push(Line::from("  Aim with IJKL, then press 1-0 to use that slot.").white());
    lines.push(Line::from("  Reload with R. Roundhouse with F breaks nearby props.").white());
    lines.push(Line::from("  Pick up gear, manage stamina, and let gangs fight each other.").white());
    lines.push(Line::from(""));
    lines.push(Line::from("  CONTROLS").bold().dark_gray());

    for binding in KEYBINDINGS {
        let label = if binding.name == "Roundhouse" {
            format!("    [{}]  {}", binding.key, binding.docs)
        } else {
            format!("    [{}]  {}", binding.key, binding.docs)
        };
        lines.push(Line::from(label).white());
    }
    lines.push(Line::from(""));

    let content_height = lines.len() as u16 + 2; // +2 for border
    let w = 72u16.min(game_area.width.saturating_sub(4));
    let h = content_height.min(game_area.height.saturating_sub(4));

    if w < 20 || h < 5 {
        return;
    }

    let cx = game_area.x + (game_area.width.saturating_sub(w)) / 2;
    let cy = game_area.y + (game_area.height.saturating_sub(h)) / 2;
    let menu_area = Rect {
        x: cx,
        y: cy,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, menu_area);

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Menu ")
                    .border_style(
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                    ),
            )
            .wrap(Wrap { trim: false })
            .on_black(),
        menu_area,
    );
}
