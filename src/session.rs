use bevy_ecs::{
    hierarchy::{ChildOf, Children},
    prelude::*,
};
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub title: String,
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatMessage;

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatMessageRole {
    User,
    Assistant,
    System,
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ChatMessageText(pub String);

#[derive(Bundle)]
pub struct SessionBundle {
    pub session: Session,
}

impl SessionBundle {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            session: Session {
                title: title.into(),
            },
        }
    }
}

#[derive(Bundle)]
pub struct ChatMessageBundle {
    pub message: ChatMessage,
    pub role: ChatMessageRole,
    pub text: ChatMessageText,
    pub child_of: ChildOf,
}

impl ChatMessageBundle {
    pub fn new(session: Entity, role: ChatMessageRole, text: impl Into<String>) -> Self {
        Self {
            message: ChatMessage,
            role,
            text: ChatMessageText(text.into()),
            child_of: ChildOf(session),
        }
    }
}

pub fn spawn_session(world: &mut World, title: impl Into<String>) -> Entity {
    world.spawn(SessionBundle::new(title)).id()
}

pub fn spawn_chat_message(
    world: &mut World,
    session: Entity,
    role: ChatMessageRole,
    text: impl Into<String>,
) -> Entity {
    world
        .spawn(ChatMessageBundle::new(session, role, text))
        .id()
}

pub fn collect_transcript(world: &World, session: Entity) -> Vec<(ChatMessageRole, String)> {
    let mut transcript = Vec::new();

    let Some(children) = world.get::<Children>(session) else {
        return transcript;
    };

    for child in children.iter() {
        let Some(role) = world.get::<ChatMessageRole>(child) else {
            continue;
        };
        let Some(text) = world.get::<ChatMessageText>(child) else {
            continue;
        };
        transcript.push((role.clone(), text.0.clone()));
    }

    transcript
}
