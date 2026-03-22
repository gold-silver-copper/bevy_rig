use bevy::prelude::*;

use crate::components::Position;

#[derive(Message, Clone, Copy, Debug)]
pub struct MoveIntent {
    pub entity: Entity,
    pub dx: i32,
    pub dy: i32,
}

#[derive(Message, Clone, Debug)]
pub struct TalkIntent {
    pub prompt: String,
}

#[derive(Message, Clone, Copy, Debug)]
pub struct InteractIntent {
    pub position: Position,
}
