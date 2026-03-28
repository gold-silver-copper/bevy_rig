use std::collections::VecDeque;

use bevy::prelude::*;

use crate::components::Position;

pub const PLAYER_VIEW_RADIUS: i32 = 24;
pub const PLAYER_VIEW_MAX_RANGE: i32 = 44;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    Explore,
    Talking,
}

#[derive(Resource, Debug)]
pub struct UiState {
    pub mode: UiMode,
    pub draft: String,
    pub cursor: Position,
    pub show_debug: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            mode: UiMode::Explore,
            draft: String::new(),
            cursor: Position::new(0, 0),
            show_debug: false,
        }
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct PlayerNeeds {
    pub hunger: f32,
    pub thirst: f32,
}

impl Default for PlayerNeeds {
    fn default() -> Self {
        Self {
            hunger: 82.0,
            thirst: 74.0,
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct WorldClock {
    pub frame: u64,
    pub elapsed_seconds: f64,
    pub turn: u64,
}

#[derive(Resource, Debug, Default)]
pub struct GameLog {
    pub lines: VecDeque<String>,
}

impl GameLog {
    pub fn push(&mut self, line: impl Into<String>) {
        self.lines.push_back(line.into());
        while self.lines.len() > 48 {
            self.lines.pop_front();
        }
    }
}
