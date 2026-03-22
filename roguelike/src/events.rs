use bevy::prelude::*;

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
