use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::{
    components::{
        Memory, MovePlan, Name, Npc, NpcPace, PendingAction, PendingReply, Player, Position,
        RigPersona, Speaker, Wanderer,
    },
    events::{InteractIntent, MoveIntent, TalkIntent},
    map::{PropKind, TileMap},
    resources::{
        GameLog, PLAYER_VIEW_MAX_RANGE, PLAYER_VIEW_RADIUS, PlayerNeeds, UiMode, UiState,
        WorldClock,
    },
    runtime::{
        DrinkCandidate, MovementCandidate, NpcActionOutcome, RigResponse, RigRuntime,
        RuntimeMessage,
    },
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
    ui: Res<UiState>,
    player_query: Query<&Position, With<Player>>,
    npc_query: Query<(&Position, &Name), With<Npc>>,
    mut log: ResMut<GameLog>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let visible_tiles = map.player_visible_tiles(
        *player_pos,
        ui.cursor,
        PLAYER_VIEW_RADIUS,
        PLAYER_VIEW_MAX_RANGE,
    );

    for intent in intents.read() {
        let target = intent.position;
        if !map.in_bounds(target.x, target.y) {
            log.push("There is nothing there beyond the carved stone of the alehall.");
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

pub fn request_npc_actions_system(
    time: Res<Time>,
    ui: Res<UiState>,
    map: Res<TileMap>,
    mut clock: ResMut<WorldClock>,
    mut log: ResMut<GameLog>,
    player_query: Query<&Position, With<Player>>,
    occupants: Query<(Entity, &Position, Option<&Name>), Or<(With<Player>, With<Npc>)>>,
    mut runtime: ResMut<RigRuntime>,
    mut npcs: Query<
        (
            Entity,
            &Position,
            &Name,
            &RigPersona,
            &mut Memory,
            &mut NpcPace,
            &mut Wanderer,
            &mut MovePlan,
            &mut PendingAction,
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
        mut memory,
        mut pace,
        mut wanderer,
        mut move_plan,
        mut pending_action,
        pending_reply,
    ) in &mut npcs
    {
        pace.think_timer.tick(time.delta());
        if !pace.think_timer.just_finished() {
            continue;
        }

        if pending_reply.waiting() || pending_action.waiting() || move_plan.has_steps() {
            continue;
        }

        let mut blocked = occupied_positions.clone();
        blocked.remove(&(pos.x, pos.y));

        let (context, move_candidates, drink_candidates) =
            build_action_context(*pos, *player_pos, name, &memory, *wanderer, &map, &occupied);

        let system_prompt = build_action_system_prompt(name, persona, &memory);
        if let Some(dispatch) = runtime.spawn_action_decision(
            entity,
            system_prompt,
            persona.preferred_model.as_deref(),
            context,
            move_candidates.clone(),
            drink_candidates.clone(),
        ) {
            pending_action.request_id = Some(dispatch.request_id);
            move_plan.trace = format!(
                "req#{} provider={} model={} status=awaiting_action_decision",
                dispatch.request_id, dispatch.provider_label, dispatch.model
            );
        } else {
            let trace = "planner unavailable: falling back to local action".to_string();
            let outcome =
                choose_fallback_action(*pos, &mut wanderer, &move_candidates, &drink_candidates);
            clock.turn += 1;
            apply_npc_action_outcome(
                name,
                *pos,
                &mut memory,
                &mut move_plan,
                outcome,
                &trace,
                &map,
                &blocked,
                &mut log,
                clock.turn,
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
            &PendingAction,
            &PendingReply,
        ),
        With<Npc>,
    >,
    mut move_intents: MessageWriter<MoveIntent>,
) {
    if ui.mode == UiMode::Talking {
        return;
    }

    for (entity, pos, move_plan, mut pace, pending_action, pending_reply) in &mut npcs {
        pace.move_timer.tick(time.delta());
        if !pace.move_timer.just_finished() {
            continue;
        }

        if pending_action.waiting() || pending_reply.waiting() {
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
            &mut PendingAction,
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
            mut pending_action,
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

            move_plan.clear();
            pending_action.request_id = None;

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
    player_query: Query<&Position, With<Player>>,
    occupants: Query<(Entity, &Position, Option<&Name>), Or<(With<Player>, With<Npc>)>>,
    mut npcs: Query<
        (
            Entity,
            &Name,
            &Position,
            &mut Memory,
            &mut MovePlan,
            &mut PendingAction,
            &mut PendingReply,
            &mut Wanderer,
        ),
        With<Npc>,
    >,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let occupied = occupants
        .iter()
        .map(|(entity, pos, maybe_name)| {
            let name = maybe_name
                .map(|name| name.0.clone())
                .unwrap_or_else(|| format!("entity-{entity:?}"));
            (entity, (*pos, name))
        })
        .collect::<HashMap<_, _>>();
    let mut npc_speeches = Vec::new();

    while let Some(response) = runtime.try_recv() {
        match response {
            RigResponse::ChatSuccess {
                request_id,
                entity,
                content,
            } => {
                if let Ok((_, name, pos, mut memory, _, _, mut pending_reply, _)) =
                    npcs.get_mut(entity)
                    && pending_reply.request_id == Some(request_id)
                {
                    pending_reply.request_id = None;
                    if is_no_reply(&content) {
                        continue;
                    }
                    memory.push(clock.turn, Speaker::Npc, content.clone());
                    log.push(format!("{} says: {}", name.0, content));
                    npc_speeches.push((entity, *pos, content));
                }
            }
            RigResponse::ChatFailure {
                request_id,
                entity,
                error,
            } => {
                if let Ok((_, name, pos, mut memory, _, _, mut pending_reply, _)) =
                    npcs.get_mut(entity)
                    && pending_reply.request_id == Some(request_id)
                {
                    pending_reply.request_id = None;
                    let fallback = fallback_reply(name, &memory);
                    memory.push(clock.turn, Speaker::Npc, fallback.clone());
                    let _ = error;
                    log.push(format!("{} says: {}", name.0, fallback));
                    npc_speeches.push((entity, *pos, fallback));
                }
            }
            RigResponse::ActionSuccess {
                request_id,
                entity,
                outcome,
                trace,
            } => {
                if let Ok((_, name, pos, mut memory, mut move_plan, mut pending_action, _, _)) =
                    npcs.get_mut(entity)
                    && pending_action.request_id == Some(request_id)
                {
                    pending_action.request_id = None;
                    let blocked = occupied_positions_excluding(&occupied, entity);
                    if let Some(spoken) = apply_npc_action_outcome(
                        name,
                        *pos,
                        &mut memory,
                        &mut move_plan,
                        outcome,
                        &trace,
                        &map,
                        &blocked,
                        &mut log,
                        clock.turn,
                    ) {
                        npc_speeches.push((entity, *pos, spoken));
                    }
                }
            }
            RigResponse::ActionFailure {
                request_id,
                entity,
                error,
                trace,
            } => {
                if let Ok((
                    _,
                    name,
                    pos,
                    mut memory,
                    mut move_plan,
                    mut pending_action,
                    _,
                    mut wanderer,
                )) = npcs.get_mut(entity)
                    && pending_action.request_id == Some(request_id)
                {
                    pending_action.request_id = None;
                    let blocked = occupied_positions_excluding(&occupied, entity);
                    let (_, move_candidates, drink_candidates) = build_action_context(
                        *pos,
                        *player_pos,
                        name,
                        &memory,
                        *wanderer,
                        &map,
                        &build_occupant_map(&occupants),
                    );
                    let outcome = choose_fallback_action(
                        *pos,
                        &mut wanderer,
                        &move_candidates,
                        &drink_candidates,
                    );
                    log.push(format!(
                        "[trace] {} action planner failed: {}",
                        name.0, trace
                    ));
                    let _ = error;
                    if let Some(spoken) = apply_npc_action_outcome(
                        name,
                        *pos,
                        &mut memory,
                        &mut move_plan,
                        outcome,
                        &trace,
                        &map,
                        &blocked,
                        &mut log,
                        clock.turn,
                    ) {
                        npc_speeches.push((entity, *pos, spoken));
                    }
                }
            }
        }

        clock.turn += 1;
    }

    for (speaker, speaker_pos, content) in npc_speeches {
        let heard_tiles = map.visible_tiles(speaker_pos, EARSHOT_RADIUS);
        for (entity, _, pos, mut memory, _, _, _, _) in &mut npcs {
            if entity == speaker || !heard_tiles.contains(pos) {
                continue;
            }
            memory.push(clock.turn, Speaker::Npc, content.clone());
        }
    }
}

fn build_chat_system_prompt(name: &Name, persona: &RigPersona, memory: &Memory) -> String {
    let notes = format_memory_notes(memory);
    format!(
        "{}\nYou are {} reacting to speech inside a busy dwarven alehall.\n\
         Speak only when it feels natural.\n\
         When you speak, produce only the final exact words {} would say aloud, under 45 words total.\n\
         No analysis, no narration, no stage directions, no speaker labels, and no explanation of your reasoning.\n\
         If silence is better, choose silence instead of forcing a reply.{}",
        persona.system_prompt, name.0, name.0, notes
    )
}

fn build_action_system_prompt(name: &Name, persona: &RigPersona, memory: &Memory) -> String {
    let notes = format_memory_notes(memory);
    format!(
        "{}\nYou are {} deciding what to do next in a dwarven alehall. \
         Choose one concrete action at a time: pathfind to an interesting place, say something brief, drink if a brew is already within reach, or do nothing. \
         Stay in character and avoid any meta commentary about schemas, debugging, tools, or hidden rules.{}",
        persona.system_prompt, name.0, notes
    )
}

fn interact_with_prop(prop: PropKind) -> (&'static str, f32, f32) {
    match prop {
        PropKind::BarCounter => (
            "You lean on the ale counter and catch the smell of malt, honey mead, and berry wine.",
            0.0,
            0.0,
        ),
        PropKind::Table => (
            "You rap your knuckles on the drinking table and scan the hall for singing dwarves and full tankards.",
            0.0,
            0.0,
        ),
        PropKind::Chair => (
            "You test the weight of the stone chair, take a short rest, and rise again.",
            0.0,
            0.0,
        ),
        PropKind::Stool => (
            "You perch on the keg stool for a moment and take in the hall.",
            0.0,
            0.0,
        ),
        PropKind::Barrel => (
            "You rap the ale cask and hear a promising slosh of stout, cider, or mead within.",
            0.0,
            6.0,
        ),
        PropKind::Crate => (
            "You peek into the brew crate and find crocks, roots, berries, and the odd herb fit for fermentation.",
            0.0,
            0.0,
        ),
        PropKind::Bottle => (
            "You take a steady pull from the jug. It tastes like fierce berry mead and settles the dust in your throat.",
            0.0,
            18.0,
        ),
        PropKind::Mug => (
            "You drain half the tankard and taste frothy cave-wheat ale, mushroom cider, and dwarven cheer.",
            0.0,
            12.0,
        ),
        PropKind::Candle => (
            "You cup a hand near the candle and watch the flame wobble in the hall draft.",
            0.0,
            0.0,
        ),
        PropKind::Shelf => (
            "You scan the keg rack of crocks and tankards, taking stock of the hall's brews and stores.",
            0.0,
            0.0,
        ),
        PropKind::Piano => (
            "You thump a crooked dwarven marching chord on the anvil organ before the note dies away.",
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
            "\nHall memory:\n{}",
            memory
                .notes
                .iter()
                .map(|note| format!("- {note}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn build_action_context(
    origin: Position,
    player: Position,
    name: &Name,
    memory: &Memory,
    wanderer: Wanderer,
    map: &TileMap,
    occupied: &HashMap<(i32, i32), String>,
) -> (String, Vec<MovementCandidate>, Vec<DrinkCandidate>) {
    let mut visible_people = Vec::new();
    let mut visible_fixtures = Vec::new();
    let mut move_candidates = Vec::new();
    let mut drink_candidates = Vec::new();
    let visible_set = map.visible_tiles(origin, wanderer.vision_radius);
    let player_visible = visible_set.contains(&player);
    let mut visible_tiles = visible_set.into_iter().collect::<Vec<_>>();
    visible_tiles.sort_by_key(|pos| (pos.y, pos.x));

    for pos in &visible_tiles {
        let tile = map.tile(pos.x, pos.y);
        let prop = map.prop(pos.x, pos.y);
        let occupied_by = occupied.get(&(pos.x, pos.y)).cloned();
        let walkable = map.is_walkable(*pos);
        let inside_roam = pos.chebyshev_distance(wanderer.home) <= wanderer.radius;
        let candidate = walkable && inside_roam && (occupied_by.is_none() || *pos == origin);

        if let Some(prop) = prop {
            visible_fixtures.push(format!(
                "{}@({}, {}) dist_self={}",
                prop.label(),
                pos.x,
                pos.y,
                origin.chebyshev_distance(*pos)
            ));
        }

        if let Some(occupant) = occupied_by.as_deref()
            && *pos != origin
        {
            visible_people.push(format!(
                "{}@({}, {}) dist_self={} dist_home={}",
                occupant,
                pos.x,
                pos.y,
                origin.chebyshev_distance(*pos),
                wanderer.home.chebyshev_distance(*pos)
            ));
        }

        if let Some(prop) = prop.filter(|prop| is_drinkable_prop(*prop))
            && origin.chebyshev_distance(*pos) <= 1
        {
            let id = drink_candidates.len() as u16;
            drink_candidates.push(DrinkCandidate {
                id,
                position: *pos,
                prop,
                metadata: format!(
                    "tile=({}, {}) prop={} dist_self={} within_reach=true",
                    pos.x,
                    pos.y,
                    prop.label(),
                    origin.chebyshev_distance(*pos)
                ),
            });
        }

        if candidate && *pos != origin {
            let interest = interesting_place_tags(*pos, origin, wanderer.home, map, occupied);
            if interest.is_empty() {
                continue;
            }

            let id = move_candidates.len() as u16;
            move_candidates.push(MovementCandidate {
                id,
                position: *pos,
                metadata: format!(
                    "tile=({}, {}) ground={} prop={} dist_self={} dist_player={} dist_home={} interest={} occupied_by={}",
                    pos.x,
                    pos.y,
                    tile.label(),
                    prop.map(PropKind::label).unwrap_or("none"),
                    origin.chebyshev_distance(*pos),
                    player.chebyshev_distance(*pos),
                    wanderer.home.chebyshev_distance(*pos),
                    interest.join("+"),
                    occupied_by.as_deref().unwrap_or("none"),
                ),
            });
        }
    }

    if move_candidates.is_empty() {
        for pos in &visible_tiles {
            let occupied_by = occupied.get(&(pos.x, pos.y)).cloned();
            let candidate = map.is_walkable(*pos)
                && pos.chebyshev_distance(wanderer.home) <= wanderer.radius
                && (occupied_by.is_none() || *pos == origin);
            if !candidate || *pos == origin {
                continue;
            }

            let id = move_candidates.len() as u16;
            move_candidates.push(MovementCandidate {
                id,
                position: *pos,
                metadata: format!(
                    "tile=({}, {}) ground={} prop={} dist_self={} dist_player={} dist_home={} interest=open_floor occupied_by={}",
                    pos.x,
                    pos.y,
                    map.tile(pos.x, pos.y).label(),
                    map.prop(pos.x, pos.y).map(PropKind::label).unwrap_or("none"),
                    origin.chebyshev_distance(*pos),
                    player.chebyshev_distance(*pos),
                    wanderer.home.chebyshev_distance(*pos),
                    occupied_by.as_deref().unwrap_or("none"),
                ),
            });
            if move_candidates.len() >= 6 {
                break;
            }
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
         - legal_move_candidate_count={}\n\
         - legal_drink_candidate_count={}",
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
        move_candidates.len(),
        drink_candidates.len()
    );

    (context, move_candidates, drink_candidates)
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
    occupied: &HashMap<Entity, (Position, String)>,
    entity: Entity,
) -> HashSet<(i32, i32)> {
    occupied
        .iter()
        .filter_map(|(candidate, (pos, _))| (*candidate != entity).then_some((pos.x, pos.y)))
        .collect()
}

fn interesting_place_tags(
    pos: Position,
    origin: Position,
    home: Position,
    map: &TileMap,
    occupied: &HashMap<(i32, i32), String>,
) -> Vec<String> {
    let mut tags = Vec::new();

    if pos == home {
        tags.push("home".to_string());
    }
    if pos.chebyshev_distance(origin) >= 3 {
        tags.push("change_of_scene".to_string());
    }
    if let Some(prop) = map.prop(pos.x, pos.y) {
        tags.push(format!("at_{}", slugify_label(prop.label())));
    }

    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }

            let neighbor = pos.offset(dx, dy);
            if let Some(prop) = map.prop(neighbor.x, neighbor.y) {
                tags.push(format!("near_{}", slugify_label(prop.label())));
            }
            if let Some(name) = occupied.get(&(neighbor.x, neighbor.y))
                && neighbor != origin
            {
                tags.push(format!("near_{}", slugify_label(name)));
            }
        }
    }

    tags.sort();
    tags.dedup();
    tags
}

fn slugify_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn is_drinkable_prop(prop: PropKind) -> bool {
    matches!(prop, PropKind::Barrel | PropKind::Bottle | PropKind::Mug)
}

fn choose_fallback_action(
    _current: Position,
    wanderer: &mut Wanderer,
    move_candidates: &[MovementCandidate],
    drink_candidates: &[DrinkCandidate],
) -> NpcActionOutcome {
    if !drink_candidates.is_empty() && wanderer.next_direction % 4 == 0 {
        let choice = &drink_candidates[wanderer.next_direction % drink_candidates.len()];
        wanderer.next_direction = (wanderer.next_direction + 1) % DIRECTIONS.len();
        return NpcActionOutcome::Drink {
            position: choice.position,
            summary: format!("takes a drink by the {}", choice.prop.label()),
        };
    }

    if !move_candidates.is_empty() {
        let choice = &move_candidates[wanderer.next_direction % move_candidates.len()];
        wanderer.next_direction = (wanderer.next_direction + 1) % DIRECTIONS.len();
        return NpcActionOutcome::Move {
            destination: choice.position,
            summary: format!(
                "heads to ({}, {}) because instinct nudges them that way",
                choice.position.x, choice.position.y
            ),
        };
    }

    if let Some(choice) = drink_candidates.first() {
        return NpcActionOutcome::Drink {
            position: choice.position,
            summary: format!("takes a drink by the {}", choice.prop.label()),
        };
    }

    NpcActionOutcome::Idle {
        summary: "does nothing for the moment".to_string(),
    }
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

fn apply_npc_action_outcome(
    name: &Name,
    current: Position,
    memory: &mut Memory,
    move_plan: &mut MovePlan,
    outcome: NpcActionOutcome,
    trace: &str,
    map: &TileMap,
    occupied: &HashSet<(i32, i32)>,
    log: &mut GameLog,
    turn: u64,
) -> Option<String> {
    move_plan.trace = trace.to_string();

    match outcome {
        NpcActionOutcome::Move {
            destination,
            summary,
        } => {
            continue_with_fallback_plan(current, destination, &summary, map, occupied, move_plan);
            if move_plan.has_steps() && !move_plan.summary.is_empty() {
                log.push(format!("{} {}", name.0, move_plan.summary));
            }
            None
        }
        NpcActionOutcome::Speak { text } => {
            move_plan.target = None;
            move_plan.steps.clear();
            move_plan.summary = "speaks up".to_string();
            memory.push(turn, Speaker::Npc, text.clone());
            log.push(format!("{} says: {}", name.0, text));
            Some(text)
        }
        NpcActionOutcome::Drink { summary, .. } => {
            move_plan.target = None;
            move_plan.steps.clear();
            move_plan.summary = summary.clone();
            log.push(format!("{} {}", name.0, summary));
            None
        }
        NpcActionOutcome::Idle { summary } => {
            move_plan.target = None;
            move_plan.steps.clear();
            move_plan.summary = summary;
            None
        }
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
    let trimmed = content.trim();
    trimmed.is_empty() || trimmed == NO_REPLY_TOKEN
}

fn truncate_for_reply(input: &str) -> String {
    let mut truncated = input.trim().replace('\n', " ");
    if truncated.len() > 48 {
        truncated.truncate(48);
        truncated.push_str("...");
    }
    truncated
}
