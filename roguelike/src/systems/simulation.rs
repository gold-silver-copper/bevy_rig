use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::{
    components::{
        Memory, MovePlan, Name, Npc, NpcPace, PendingMove, PendingReply, Player, Position,
        RigPersona, Speaker, Wanderer,
    },
    events::{InteractIntent, MoveIntent, TalkIntent},
    map::{PropKind, TileMap},
    resources::{GameLog, PLAYER_VIEW_RADIUS, PlayerNeeds, UiMode, UiState, WorldClock},
    runtime::{MovementCandidate, RigResponse, RigRuntime, RuntimeMessage},
};

const DIRECTIONS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
const EARSHOT_RADIUS: i32 = 8;
const NO_REPLY_TOKEN: &str = "[NO_REPLY]";

pub fn advance_frame_system(time: Res<Time>, mut clock: ResMut<WorldClock>) {
    clock.frame += 1;
    clock.elapsed_seconds += time.delta_secs_f64();
}

pub fn update_player_needs_system(time: Res<Time>, mut needs: ResMut<PlayerNeeds>) {
    let dt = time.delta_secs();
    needs.hunger = (needs.hunger - dt * 0.35).clamp(0.0, 100.0);
    needs.thirst = (needs.thirst - dt * 0.55).clamp(0.0, 100.0);
}

pub fn interact_with_cursor_system(
    mut intents: MessageReader<InteractIntent>,
    map: Res<TileMap>,
    mut needs: ResMut<PlayerNeeds>,
    player_query: Query<&Position, With<Player>>,
    npc_query: Query<(&Position, &Name), With<Npc>>,
    mut log: ResMut<GameLog>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let visible_tiles = map.visible_tiles(*player_pos, PLAYER_VIEW_RADIUS);

    for intent in intents.read() {
        let target = intent.position;
        if !map.in_bounds(target.x, target.y) {
            log.push("There is nothing there beyond the tavern walls.");
            continue;
        }

        if !visible_tiles.contains(&target) {
            log.push("You cannot make that out clearly from here.");
            continue;
        }

        if target == *player_pos {
            log.push("You pat your coat, check your footing, and keep your balance.");
            continue;
        }

        if let Some((_, name)) = npc_query.iter().find(|(pos, _)| **pos == target) {
            log.push(format!(
                "{} is there. Speak aloud with T or Enter if you want the room to hear you.",
                name.0
            ));
            continue;
        }

        let distance = player_pos.chebyshev_distance(target);
        if let Some(prop) = map.prop(target.x, target.y) {
            if distance > 1 {
                log.push(format!(
                    "You study the {} from across the room. {}",
                    prop.label(),
                    prop.description()
                ));
                continue;
            }

            let (message, hunger_delta, thirst_delta) = interact_with_prop(prop);
            needs.hunger = (needs.hunger + hunger_delta).clamp(0.0, 100.0);
            needs.thirst = (needs.thirst + thirst_delta).clamp(0.0, 100.0);
            log.push(message);
            continue;
        }

        let tile = map.tile(target.x, target.y);
        if distance > 1 {
            log.push(format!("You look over the {}.", tile.label()));
        } else {
            log.push(format!(
                "You tap the {} with your boot. {}",
                tile.label(),
                tile.description()
            ));
        }
    }
}

pub fn request_npc_move_plans_system(
    time: Res<Time>,
    ui: Res<UiState>,
    map: Res<TileMap>,
    player_query: Query<&Position, With<Player>>,
    occupants: Query<(Entity, &Position, Option<&Name>), Or<(With<Player>, With<Npc>)>>,
    mut runtime: ResMut<RigRuntime>,
    mut npcs: Query<
        (
            Entity,
            &Position,
            &Name,
            &RigPersona,
            &Memory,
            &mut NpcPace,
            &mut Wanderer,
            &mut MovePlan,
            &mut PendingMove,
            &PendingReply,
        ),
        With<Npc>,
    >,
) {
    if ui.mode == UiMode::Talking {
        return;
    }

    let Ok(player_pos) = player_query.single() else {
        return;
    };

    let occupied = build_occupant_map(&occupants);
    let occupied_positions = occupied.keys().copied().collect::<HashSet<_>>();

    for (
        entity,
        pos,
        name,
        persona,
        memory,
        mut pace,
        mut wanderer,
        mut move_plan,
        mut pending_move,
        pending_reply,
    ) in &mut npcs
    {
        pace.think_timer.tick(time.delta());
        if !pace.think_timer.just_finished() {
            continue;
        }

        if pending_reply.waiting() || pending_move.waiting() || move_plan.has_steps() {
            continue;
        }

        let mut blocked = occupied_positions.clone();
        blocked.remove(&(pos.x, pos.y));

        let (context, candidates) =
            build_move_context(*pos, *player_pos, name, memory, *wanderer, &map, &occupied);

        if candidates.is_empty() {
            move_plan.trace = "planner skipped: no visible floor candidates".to_string();
            let fallback = choose_fallback_destination(*pos, &mut wanderer, &map, &blocked);
            continue_with_fallback_plan(
                *pos,
                fallback,
                "holds the line",
                &map,
                &blocked,
                &mut move_plan,
            );
            continue;
        }

        let system_prompt = build_movement_system_prompt(name, persona, memory);
        if let Some(dispatch) = runtime.spawn_move_decision(
            entity,
            system_prompt,
            persona.preferred_model.as_deref(),
            context,
            candidates,
        ) {
            pending_move.request_id = Some(dispatch.request_id);
            move_plan.trace = format!(
                "req#{} provider={} model={} status=awaiting_structured_decision",
                dispatch.request_id, dispatch.provider_label, dispatch.model
            );
        } else {
            move_plan.trace = "planner unavailable: falling back to local movement".to_string();
            let fallback = choose_fallback_destination(*pos, &mut wanderer, &map, &blocked);
            continue_with_fallback_plan(
                *pos,
                fallback,
                "wanders by instinct",
                &map,
                &blocked,
                &mut move_plan,
            );
        }
    }
}

pub fn advance_npc_move_plans_system(
    time: Res<Time>,
    ui: Res<UiState>,
    mut npcs: Query<
        (
            Entity,
            &Position,
            &MovePlan,
            &mut NpcPace,
            &PendingMove,
            &PendingReply,
        ),
        With<Npc>,
    >,
    mut move_intents: MessageWriter<MoveIntent>,
) {
    if ui.mode == UiMode::Talking {
        return;
    }

    for (entity, pos, move_plan, mut pace, pending_move, pending_reply) in &mut npcs {
        pace.move_timer.tick(time.delta());
        if !pace.move_timer.just_finished() {
            continue;
        }

        if pending_move.waiting() || pending_reply.waiting() {
            continue;
        }

        let Some(next) = move_plan.steps.first().copied() else {
            continue;
        };

        move_intents.write(MoveIntent {
            entity,
            dx: next.x - pos.x,
            dy: next.y - pos.y,
        });
    }
}

pub fn resolve_move_intents_system(
    mut intents: MessageReader<MoveIntent>,
    map: Res<TileMap>,
    mut clock: ResMut<WorldClock>,
    ui: Option<ResMut<UiState>>,
    player_entities: Query<(), With<Player>>,
    mut npc_move_plans: Query<&mut MovePlan, With<Npc>>,
    mut positions: Query<(Entity, &mut Position, Option<&Name>), Or<(With<Player>, With<Npc>)>>,
) {
    let mut ui = ui;
    let mut current_positions = HashMap::new();
    let mut occupied = HashMap::new();

    for (entity, pos, _) in &mut positions {
        current_positions.insert(entity, *pos);
        occupied.insert((pos.x, pos.y), entity);
    }

    for intent in intents.read() {
        let Some(current) = current_positions.get(&intent.entity).copied() else {
            continue;
        };

        if intent.dx == 0 && intent.dy == 0 {
            if player_entities.get(intent.entity).is_ok() {
                clock.turn += 1;
            }
            continue;
        }

        let next = current.offset(intent.dx, intent.dy);
        if !map.is_walkable(next) {
            reroute_or_clear_npc_plan(intent.entity, current, &map, &occupied, &mut npc_move_plans);
            continue;
        }

        if let Some(blocker) = occupied.get(&(next.x, next.y))
            && *blocker != intent.entity
        {
            reroute_or_clear_npc_plan(intent.entity, current, &map, &occupied, &mut npc_move_plans);
            continue;
        }

        if let Ok((_, mut pos, maybe_name)) = positions.get_mut(intent.entity) {
            occupied.remove(&(current.x, current.y));
            occupied.insert((next.x, next.y), intent.entity);
            current_positions.insert(intent.entity, next);
            *pos = next;
            clock.turn += 1;
            if player_entities.get(intent.entity).is_ok() {
                if let Some(ui) = ui.as_deref_mut() {
                    ui.cursor = Position::new(
                        (ui.cursor.x + intent.dx).clamp(0, map.width - 1),
                        (ui.cursor.y + intent.dy).clamp(0, map.height - 1),
                    );
                }
            }
            let _ = maybe_name;
        }
    }
}

pub fn settle_npc_move_plans_system(mut npcs: Query<(&Position, &mut MovePlan), With<Npc>>) {
    for (pos, mut move_plan) in &mut npcs {
        while move_plan.steps.first().copied() == Some(*pos) {
            move_plan.steps.remove(0);
        }

        if move_plan.steps.is_empty() && move_plan.target == Some(*pos) {
            move_plan.target = None;
        }
    }
}

pub fn start_talk_system(
    mut intents: MessageReader<TalkIntent>,
    map: Res<TileMap>,
    player_query: Query<&Position, With<Player>>,
    mut clock: ResMut<WorldClock>,
    mut log: ResMut<GameLog>,
    mut runtime: ResMut<RigRuntime>,
    mut npcs: Query<
        (
            Entity,
            &Name,
            &Position,
            &RigPersona,
            &mut Memory,
            &mut MovePlan,
            &mut PendingMove,
            &mut PendingReply,
        ),
        With<Npc>,
    >,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };

    for intent in intents.read() {
        clock.turn += 1;
        log.push(format!("You say: {}", intent.prompt));
        let heard_tiles = map.visible_tiles(*player_pos, EARSHOT_RADIUS);

        let mut heard_anyone = false;
        for (
            entity,
            name,
            pos,
            persona,
            mut memory,
            mut move_plan,
            mut pending_move,
            mut pending_reply,
        ) in &mut npcs
        {
            if !heard_tiles.contains(pos) {
                continue;
            }

            heard_anyone = true;
            let history = memory
                .conversation
                .iter()
                .map(|entry| RuntimeMessage {
                    speaker: entry.speaker,
                    content: entry.text.clone(),
                })
                .collect::<Vec<_>>();

            memory.push(clock.turn, Speaker::Player, intent.prompt.clone());

            if pending_reply.waiting() {
                continue;
            }

            pending_move.request_id = None;
            move_plan.clear();

            let system_prompt = build_chat_system_prompt(name, persona, &memory);
            if let Some(dispatch) = runtime.spawn_chat(
                entity,
                system_prompt,
                persona.preferred_model.as_deref(),
                history,
                intent.prompt.clone(),
            ) {
                pending_reply.request_id = Some(dispatch.request_id);
            } else {
                let reply = offline_reply(name, &memory);
                memory.push(clock.turn, Speaker::Npc, reply.clone());
                log.push(format!("{} says: {}", name.0, reply));
            }
        }

        if !heard_anyone {
            log.push("No one has a clear line close enough to hear.");
        }
    }
}

pub fn poll_rig_responses_system(
    map: Res<TileMap>,
    mut clock: ResMut<WorldClock>,
    mut log: ResMut<GameLog>,
    runtime: Res<RigRuntime>,
    occupants: Query<(Entity, &Position), Or<(With<Player>, With<Npc>)>>,
    mut npcs: Query<
        (
            &Name,
            &Position,
            &mut Memory,
            &mut MovePlan,
            &mut PendingMove,
            &mut PendingReply,
            &mut Wanderer,
        ),
        With<Npc>,
    >,
) {
    let occupied = occupants
        .iter()
        .map(|(entity, pos)| (entity, *pos))
        .collect::<HashMap<_, _>>();

    while let Some(response) = runtime.try_recv() {
        match response {
            RigResponse::ChatSuccess {
                request_id,
                entity,
                content,
            } => {
                if let Ok((name, _, mut memory, _, _, mut pending_reply, _)) = npcs.get_mut(entity)
                    && pending_reply.request_id == Some(request_id)
                {
                    pending_reply.request_id = None;
                    if is_no_reply(&content) {
                        continue;
                    }
                    memory.push(clock.turn, Speaker::Npc, content.clone());
                    log.push(format!("{} says: {}", name.0, content));
                }
            }
            RigResponse::ChatFailure {
                request_id,
                entity,
                error,
            } => {
                if let Ok((name, _, mut memory, _, _, mut pending_reply, _)) = npcs.get_mut(entity)
                    && pending_reply.request_id == Some(request_id)
                {
                    pending_reply.request_id = None;
                    let fallback = fallback_reply(name, &memory);
                    memory.push(clock.turn, Speaker::Npc, fallback.clone());
                    let _ = error;
                    log.push(format!("{} says: {}", name.0, fallback));
                }
            }
            RigResponse::MoveSuccess {
                request_id,
                entity,
                destination,
                summary,
                trace,
            } => {
                if let Ok((name, pos, _, mut move_plan, mut pending_move, _, _)) =
                    npcs.get_mut(entity)
                    && pending_move.request_id == Some(request_id)
                {
                    pending_move.request_id = None;
                    move_plan.trace = trace;
                    let blocked = occupied_positions_excluding(&occupied, entity);
                    continue_with_fallback_plan(
                        *pos,
                        destination,
                        &summary,
                        &map,
                        &blocked,
                        &mut move_plan,
                    );
                    if move_plan.has_steps() && !move_plan.summary.is_empty() {
                        log.push(format!("{} {}", name.0, move_plan.summary));
                    }
                }
            }
            RigResponse::MoveFailure {
                request_id,
                entity,
                error,
                trace,
            } => {
                if let Ok((name, pos, _, mut move_plan, mut pending_move, _, mut wanderer)) =
                    npcs.get_mut(entity)
                    && pending_move.request_id == Some(request_id)
                {
                    pending_move.request_id = None;
                    move_plan.trace = trace.clone();
                    let blocked = occupied_positions_excluding(&occupied, entity);
                    let target = choose_fallback_destination(*pos, &mut wanderer, &map, &blocked);
                    continue_with_fallback_plan(
                        *pos,
                        target,
                        "falls back to instinct",
                        &map,
                        &blocked,
                        &mut move_plan,
                    );
                    log.push(format!("[trace] {} move planner failed: {}", name.0, trace));
                    let _ = error;
                    if move_plan.has_steps() && !move_plan.summary.is_empty() {
                        log.push(format!("{} {}", name.0, move_plan.summary));
                    }
                }
            }
        }

        clock.turn += 1;
    }
}

fn build_chat_system_prompt(name: &Name, persona: &RigPersona, memory: &Memory) -> String {
    let notes = format_memory_notes(memory);
    format!(
        "{}\nYou are speaking as {} inside a tiny roguelike town. Stay in character, keep replies under 45 words, and avoid meta commentary. If you would stay silent, respond with EXACTLY {} and nothing else.{}",
        persona.system_prompt, name.0, NO_REPLY_TOKEN, notes
    )
}

fn build_movement_system_prompt(name: &Name, persona: &RigPersona, memory: &Memory) -> String {
    let notes = format_memory_notes(memory);
    format!(
        "{}\nYou are {} deciding where to walk next in a small frontier town. Stay in character, choose one believable legal destination from the candidate list, and avoid any meta commentary about schemas, debugging, or hidden system rules.{}",
        persona.system_prompt, name.0, notes
    )
}

fn interact_with_prop(prop: PropKind) -> (&'static str, f32, f32) {
    match prop {
        PropKind::BarCounter => (
            "You lean on the bar counter and catch the smell of oak, lemon oil, and old whiskey.",
            0.0,
            0.0,
        ),
        PropKind::Table => (
            "You rap your knuckles on the table and scan the tavern room.",
            0.0,
            0.0,
        ),
        PropKind::Chair => (
            "You pull out the chair, take a short breather, and stand again.",
            0.0,
            0.0,
        ),
        PropKind::Stool => (
            "You perch on the stool for a moment and take in the room.",
            0.0,
            0.0,
        ),
        PropKind::Barrel => (
            "You knock on the barrel and hear beer slosh against the staves.",
            0.0,
            6.0,
        ),
        PropKind::Crate => (
            "You peek into the crate and find towels, candles, and kitchen odds and ends.",
            0.0,
            0.0,
        ),
        PropKind::Bottle => (
            "You take a steady pull from the bottle. It burns, but it settles the dust in your throat.",
            0.0,
            18.0,
        ),
        PropKind::Mug => (
            "You drain half the mug and feel a little less parched.",
            0.0,
            12.0,
        ),
        PropKind::Candle => (
            "You cup a hand near the candle and watch the flame wobble in the draft.",
            0.0,
            0.0,
        ),
        PropKind::Shelf => (
            "You scan the shelf of cups and bottles, taking stock of the tavern's supplies.",
            0.0,
            0.0,
        ),
        PropKind::Piano => (
            "You tap out a crooked saloon chord on the piano before the note dies away.",
            0.0,
            0.0,
        ),
    }
}

fn format_memory_notes(memory: &Memory) -> String {
    if memory.notes.is_empty() {
        String::new()
    } else {
        format!(
            "\nTown memory:\n{}",
            memory
                .notes
                .iter()
                .map(|note| format!("- {note}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn build_move_context(
    origin: Position,
    player: Position,
    name: &Name,
    memory: &Memory,
    wanderer: Wanderer,
    map: &TileMap,
    occupied: &HashMap<(i32, i32), String>,
) -> (String, Vec<MovementCandidate>) {
    let mut visible_people = Vec::new();
    let mut visible_fixtures = Vec::new();
    let mut candidates = Vec::new();
    let visible_set = map.visible_tiles(origin, wanderer.vision_radius);
    let player_visible = visible_set.contains(&player);
    let mut visible_tiles = visible_set.into_iter().collect::<Vec<_>>();
    visible_tiles.sort_by_key(|pos| (pos.y, pos.x));

    for pos in visible_tiles {
        let tile = map.tile(pos.x, pos.y);
        let prop = map.prop(pos.x, pos.y);
        let occupied_by = occupied.get(&(pos.x, pos.y)).cloned();
        let walkable = map.is_walkable(pos);
        let inside_roam = pos.chebyshev_distance(wanderer.home) <= wanderer.radius;
        let candidate = walkable && inside_roam && (occupied_by.is_none() || pos == origin);

        if let Some(prop) = prop {
            visible_fixtures.push(format!(
                "{}@({}, {}) dist_self={}",
                prop.label(),
                pos.x,
                pos.y,
                origin.chebyshev_distance(pos)
            ));
        }

        if let Some(occupant) = occupied_by.as_deref()
            && pos != origin
        {
            visible_people.push(format!(
                "{}@({}, {}) dist_self={} dist_home={}",
                occupant,
                pos.x,
                pos.y,
                origin.chebyshev_distance(pos),
                wanderer.home.chebyshev_distance(pos)
            ));
        }

        if candidate {
            let id = candidates.len() as u16;
            candidates.push(MovementCandidate {
                id,
                position: pos,
                metadata: format!(
                    "tile=({}, {}) ground={} prop={} dist_self={} dist_player={} dist_home={} current={} home={} occupied_by={}",
                    pos.x,
                    pos.y,
                    tile.label(),
                    prop.map(PropKind::label).unwrap_or("none"),
                    origin.chebyshev_distance(pos),
                    player.chebyshev_distance(pos),
                    wanderer.home.chebyshev_distance(pos),
                    pos == origin,
                    pos == wanderer.home,
                    occupied_by.as_deref().unwrap_or("none"),
                ),
            });
        }
    }

    let last_exchange = memory
        .conversation
        .iter()
        .rev()
        .take(2)
        .rev()
        .map(|entry| {
            format!(
                "- {}: {}",
                match entry.speaker {
                    Speaker::Player => "player",
                    Speaker::Npc => name.0.as_str(),
                },
                truncate_for_reply(&entry.text)
            )
        })
        .collect::<Vec<_>>();

    let context = format!(
        "Current state:\n\
         - position=({}, {})\n\
         - home=({}, {})\n\
         - roam_radius={}\n\
         - vision_radius={}\n\
         - player_position=({}, {})\n\
         - player_visible={}\n\
         - visible_people={}\n\
         - visible_fixtures={}\n\
         - recent_dialogue={}\n\
         - legal_candidate_count={}",
        origin.x,
        origin.y,
        wanderer.home.x,
        wanderer.home.y,
        wanderer.radius,
        wanderer.vision_radius,
        player.x,
        player.y,
        player_visible,
        if visible_people.is_empty() {
            "none".to_string()
        } else {
            visible_people.join(" | ")
        },
        if visible_fixtures.is_empty() {
            "none".to_string()
        } else {
            visible_fixtures.join(" | ")
        },
        if last_exchange.is_empty() {
            "none".to_string()
        } else {
            last_exchange.join(" | ")
        },
        candidates.len()
    );

    (context, candidates)
}

fn build_occupant_map(
    occupants: &Query<(Entity, &Position, Option<&Name>), Or<(With<Player>, With<Npc>)>>,
) -> HashMap<(i32, i32), String> {
    occupants
        .iter()
        .map(|(entity, pos, maybe_name)| {
            let name = maybe_name
                .map(|name| name.0.clone())
                .unwrap_or_else(|| format!("entity-{entity:?}"));
            ((pos.x, pos.y), name)
        })
        .collect()
}

fn occupied_positions_excluding(
    occupied: &HashMap<Entity, Position>,
    entity: Entity,
) -> HashSet<(i32, i32)> {
    occupied
        .iter()
        .filter_map(|(candidate, pos)| (*candidate != entity).then_some((pos.x, pos.y)))
        .collect()
}

fn choose_fallback_destination(
    current: Position,
    wanderer: &mut Wanderer,
    map: &TileMap,
    occupied: &HashSet<(i32, i32)>,
) -> Position {
    for offset in 0..DIRECTIONS.len() {
        let index = (wanderer.next_direction + offset) % DIRECTIONS.len();
        let (dx, dy) = DIRECTIONS[index];
        let candidate = current.offset(dx, dy);
        if candidate.chebyshev_distance(wanderer.home) > wanderer.radius {
            continue;
        }
        if !map.is_walkable(candidate) {
            continue;
        }
        if candidate != current && occupied.contains(&(candidate.x, candidate.y)) {
            continue;
        }

        wanderer.next_direction = (index + 1) % DIRECTIONS.len();
        return candidate;
    }

    wanderer.next_direction = (wanderer.next_direction + 1) % DIRECTIONS.len();
    current
}

fn reroute_or_clear_npc_plan(
    entity: Entity,
    current: Position,
    map: &TileMap,
    occupied: &HashMap<(i32, i32), Entity>,
    npc_move_plans: &mut Query<&mut MovePlan, With<Npc>>,
) {
    let Ok(mut move_plan) = npc_move_plans.get_mut(entity) else {
        return;
    };

    let Some(target) = move_plan.target else {
        move_plan.steps.clear();
        return;
    };

    if target == current {
        move_plan.target = None;
        move_plan.steps.clear();
        move_plan.summary.clear();
        move_plan.trace = "movement plan cleared: already at target".to_string();
        return;
    }

    let blocked = occupied
        .iter()
        .filter_map(|((x, y), occupant)| (*occupant != entity).then_some((*x, *y)))
        .collect::<HashSet<_>>();

    if let Some(path) = map.find_path(current, target, &blocked) {
        if path.is_empty() {
            move_plan.target = None;
            move_plan.steps.clear();
            move_plan.summary.clear();
            move_plan.trace = "movement plan cleared: empty reroute".to_string();
        } else {
            move_plan.steps = path;
            move_plan.trace = format!("rerouted toward ({}, {})", target.x, target.y);
        }
    } else {
        move_plan.target = None;
        move_plan.steps.clear();
        move_plan.summary.clear();
        move_plan.trace = format!("movement plan blocked near ({}, {})", target.x, target.y);
    }
}

fn continue_with_fallback_plan(
    current: Position,
    target: Position,
    summary: &str,
    map: &TileMap,
    occupied: &HashSet<(i32, i32)>,
    move_plan: &mut MovePlan,
) {
    move_plan.summary = summary.to_string();
    if target == current {
        move_plan.target = None;
        move_plan.steps.clear();
        return;
    }

    if let Some(path) = map.find_path(current, target, occupied) {
        move_plan.target = Some(target);
        move_plan.steps = path;
    } else {
        move_plan.target = None;
        move_plan.steps.clear();
    }
}

fn offline_reply(name: &Name, memory: &Memory) -> String {
    let last_player_line = memory
        .conversation
        .iter()
        .rev()
        .find(|entry| entry.speaker == Speaker::Player)
        .map(|entry| entry.text.as_str())
        .unwrap_or("hello");

    format!(
        "{} tips their hat. \"I'd answer better with a live Rig provider, but I heard you say: {}\"",
        name.0,
        truncate_for_reply(last_player_line)
    )
}

fn fallback_reply(name: &Name, memory: &Memory) -> String {
    let motif = memory
        .notes
        .first()
        .map(String::as_str)
        .unwrap_or("Keeps their own counsel.");
    format!("{} pauses. \"The line went dead. {}\"", name.0, motif)
}

fn is_no_reply(content: &str) -> bool {
    content.trim() == NO_REPLY_TOKEN
}

fn truncate_for_reply(input: &str) -> String {
    let mut truncated = input.trim().replace('\n', " ");
    if truncated.len() > 48 {
        truncated.truncate(48);
        truncated.push_str("...");
    }
    truncated
}
