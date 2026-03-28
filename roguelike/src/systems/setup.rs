use bevy::prelude::*;

use crate::{
    components::{
        Glyph, Memory, MovePlan, Name, Npc, NpcPace, PendingAction, PendingReply, Player, Position,
        RigPersona, Wanderer,
    },
    map::TileMap,
    resources::UiState,
};

const DWARF_COUNT: usize = 20;

struct DwarfProfile {
    name: String,
    glyph: char,
    home: Position,
    notes: Vec<String>,
    system_prompt: String,
    think_seconds: f32,
    move_seconds: f32,
    phase_offset_seconds: f32,
    radius: i32,
    vision_radius: i32,
}

pub fn setup_world(mut commands: Commands, mut ui: ResMut<UiState>, map: Res<TileMap>) {
    let player_start = Position::new(42, 22);
    commands.spawn((Player, Name("Urist".into()), Glyph('@'), player_start));

    for (index, profile) in generate_dwarf_profiles(&map, player_start)
        .into_iter()
        .enumerate()
    {
        commands.spawn((
            Npc,
            Name(profile.name),
            Glyph(profile.glyph),
            profile.home,
            Memory {
                notes: profile.notes,
                conversation: Vec::new(),
            },
            RigPersona {
                system_prompt: profile.system_prompt,
                preferred_model: None,
            },
            Wanderer {
                home: profile.home,
                next_direction: index % 4,
                radius: profile.radius,
                vision_radius: profile.vision_radius,
            },
            NpcPace::new(
                profile.think_seconds,
                profile.move_seconds,
                profile.phase_offset_seconds,
            ),
            PendingAction::default(),
            MovePlan::default(),
            PendingReply::default(),
        ));
    }

    ui.cursor = player_start;
}

fn generate_dwarf_profiles(map: &TileMap, player_start: Position) -> Vec<DwarfProfile> {
    let homes = dwarf_spawn_positions(map, player_start, DWARF_COUNT);
    homes
        .into_iter()
        .enumerate()
        .map(|(index, home)| dwarf_profile(index, home))
        .collect()
}

fn dwarf_profile(index: usize, home: Position) -> DwarfProfile {
    const FORENAMES_A: [&str; 10] = [
        "Urist", "Domas", "Stukos", "Zasit", "Meng", "Thob", "Rakust", "Likot", "Erush", "Kadol",
    ];
    const FORENAMES_B: [&str; 10] = [
        "Atis", "Nil", "Vucar", "Tobul", "Sodel", "Inod", "Kogan", "Thikut", "Asmel", "Mistem",
    ];
    const CLAN_A: [&str; 10] = [
        "Ale", "Copper", "Foam", "Granite", "Malt", "Honey", "Cinder", "Oak", "Deep", "Cask",
    ];
    const CLAN_B: [&str; 10] = [
        "thane", "guard", "belly", "song", "brew", "beard", "delver", "keg", "pick", "mantle",
    ];
    const ROLES: [&str; 20] = [
        "alewife",
        "hall-warden",
        "mason",
        "brewer",
        "cask-tender",
        "bookkeeper",
        "forge-stoker",
        "mushroom-grower",
        "miner",
        "cooper",
        "stonemug seller",
        "chant leader",
        "cook",
        "mead-maker",
        "cider brewer",
        "beekeeper",
        "gem setter",
        "tunnel scout",
        "storyteller",
        "quartermaster",
    ];
    const TEMPERAMENTS: [&str; 10] = [
        "booming and merry",
        "dryly funny and observant",
        "gruff but warm-hearted",
        "fond of long grudges and longer songs",
        "practical, patient, and hard to rattle",
        "easily delighted by a fresh keg",
        "suspicious at first, loyal after",
        "proud of every clever brew and carved stone",
        "restless whenever the hall grows too quiet",
        "quick to laugh and quicker to toast",
    ];
    const DRINKS: [&str; 12] = [
        "cave-wheat ale",
        "honey mead",
        "berry wine",
        "plum cider",
        "mushroom stout",
        "barley beer",
        "pear mead",
        "root liquor",
        "apple cider",
        "herb wine",
        "blackberry ale",
        "stonefruit brew",
    ];
    const QUIRKS: [&str; 12] = [
        "Hums old tunnel songs under their breath.",
        "Insists anything can be fermented if given time and a sealed crock.",
        "Collects unusual tankards and remembers who chipped each one.",
        "Treats every spilled drink as a minor tragedy.",
        "Loves swapping gossip over the noisiest table in the hall.",
        "Claims the best mead starts with stolen wild honey.",
        "Keeps count of every cask tapped this season.",
        "Will praise good stonework before any person's manners.",
        "Thinks berry wine improves every feast.",
        "Has a favorite stool and defends it with stubborn silence.",
        "Prefers broad toasts and blunt truths.",
        "Believes the hall sounds wrong unless someone is laughing.",
    ];

    let forename = if index % 2 == 0 {
        FORENAMES_A[index % FORENAMES_A.len()]
    } else {
        FORENAMES_B[index % FORENAMES_B.len()]
    };
    let clan = format!(
        "{}{}",
        CLAN_A[(index * 3 + 1) % CLAN_A.len()],
        CLAN_B[(index * 5 + 2) % CLAN_B.len()]
    );
    let name = format!("{forename} {clan}");
    let role = ROLES[index % ROLES.len()];
    let temperament = TEMPERAMENTS[(index * 7 + 3) % TEMPERAMENTS.len()];
    let drink = DRINKS[(index * 5 + 4) % DRINKS.len()];
    let quirk = QUIRKS[(index * 11 + 1) % QUIRKS.len()];
    let radius = 3 + (index as i32 % 5);
    let vision_radius = 7 + (index as i32 % 4);
    let think_seconds = 0.85 + (index % 5) as f32 * 0.14;
    let move_seconds = 0.16 + (index % 4) as f32 * 0.05;
    let phase_offset_seconds = (index as f32) * 0.17;

    DwarfProfile {
        name: name.clone(),
        glyph: dwarf_glyph(index, role),
        home,
        notes: vec![
            format!("A {} of the alehall.", role),
            format!("{name} is {temperament} and especially fond of {drink}."),
            quirk.to_string(),
        ],
        system_prompt: format!(
            "You are {name}, a dwarven {role} in a roaring mountain alehall. \
             You are {temperament}, you favor {drink}, and your habit is: {quirk} \
             Speak in one or two short sentences, stay in character, sound like a sociable dwarf, and react naturally to the room."
        ),
        think_seconds,
        move_seconds,
        phase_offset_seconds,
        radius,
        vision_radius,
    }
}

fn dwarf_glyph(index: usize, role: &str) -> char {
    if role.contains("warden") || role.contains("scout") {
        'G'
    } else if role.contains("brewer") || role.contains("mead") || role.contains("cider") {
        'B'
    } else if role.contains("cook") || role.contains("bookkeeper") {
        'C'
    } else if index % 3 == 0 {
        'D'
    } else {
        'd'
    }
}

fn dwarf_spawn_positions(map: &TileMap, player_start: Position, count: usize) -> Vec<Position> {
    let mut candidates = Vec::new();
    for y in 4..(map.height - 4) {
        for x in 4..(map.width - 4) {
            let pos = Position::new(x, y);
            if pos == player_start || !map.is_walkable(pos) {
                continue;
            }

            let nearby_props = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| pos.offset(dx, dy)))
                .filter(|neighbor| map.prop(neighbor.x, neighbor.y).is_some())
                .count() as i32;

            if nearby_props == 0 {
                continue;
            }

            let dist_from_player = player_start.chebyshev_distance(pos);
            let score = nearby_props * 10 - dist_from_player;
            candidates.push((score, pos));
        }
    }

    candidates.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.y.cmp(&right.1.y))
            .then_with(|| left.1.x.cmp(&right.1.x))
    });

    let mut chosen = Vec::new();
    for (_, pos) in &candidates {
        if chosen
            .iter()
            .all(|existing: &Position| existing.chebyshev_distance(*pos) >= 3)
        {
            chosen.push(*pos);
            if chosen.len() == count {
                return chosen;
            }
        }
    }

    for (_, pos) in candidates {
        if chosen.iter().all(|existing| *existing != pos) {
            chosen.push(pos);
            if chosen.len() == count {
                break;
            }
        }
    }

    chosen
}
