use bevy::prelude::*;

use crate::{
    components::{
        Glyph, Memory, MovePlan, Name, Npc, NpcPace, PendingMove, PendingReply, Player, Position,
        RigPersona, Wanderer,
    },
    resources::UiState,
};

pub fn setup_world(mut commands: Commands, mut ui: ResMut<UiState>) {
    let player_start = Position::new(8, 11);
    commands.spawn((Player, Name("Ranger".into()), Glyph('@'), player_start));

    let bartender = commands
        .spawn((
            Npc,
            Name("Maeve the Bartender".into()),
            Glyph('B'),
            Position::new(12, 10),
            Memory {
                notes: vec![
                    "Runs the saloon and knows most local gossip.".into(),
                    "Prefers brief, practical answers with a western cadence.".into(),
                ],
                conversation: Vec::new(),
            },
            RigPersona {
                system_prompt: "You are Maeve, a sharp bartender in a dusty frontier town. Speak in one or two short sentences, stay in character, and react to the player's questions naturally.".into(),
                preferred_model: None,
            },
            Wanderer {
                home: Position::new(12, 10),
                next_direction: 0,
                radius: 2,
                vision_radius: 5,
            },
            NpcPace::new(1.4, 0.28, 0.15),
            PendingMove::default(),
            MovePlan::default(),
            PendingReply::default(),
        ))
        .id();

    commands.spawn((
        Npc,
        Name("Sheriff Holt".into()),
        Glyph('S'),
        Position::new(26, 8),
        Memory {
            notes: vec![
                "Keeps watch over the main street.".into(),
                "Suspicious of strangers but generally fair.".into(),
            ],
            conversation: Vec::new(),
        },
        RigPersona {
            system_prompt: "You are Sheriff Holt, calm and authoritative. Reply in one or two concise sentences and stay grounded in the current town scene.".into(),
            preferred_model: None,
        },
        Wanderer {
            home: Position::new(26, 8),
            next_direction: 1,
            radius: 4,
            vision_radius: 7,
        },
        NpcPace::new(1.1, 0.22, 0.4),
        PendingMove::default(),
        MovePlan::default(),
        PendingReply::default(),
    ));

    commands.spawn((
        Npc,
        Name("Juniper the Drifter".into()),
        Glyph('D'),
        Position::new(41, 15),
        Memory {
            notes: vec![
                "A drifter passing through town looking for work.".into(),
                "Friendly, restless, and always half-ready to leave.".into(),
            ],
            conversation: Vec::new(),
        },
        RigPersona {
            system_prompt: "You are Juniper, a wandering drifter. Speak casually, keep answers short, and stay within the fiction of a frontier town.".into(),
            preferred_model: None,
        },
        Wanderer {
            home: Position::new(41, 15),
            next_direction: 2,
            radius: 5,
            vision_radius: 7,
        },
        NpcPace::new(0.9, 0.18, 0.65),
        PendingMove::default(),
        MovePlan::default(),
        PendingReply::default(),
    ));

    ui.selected_npc = Some(bartender);
    ui.cursor = player_start;
}
