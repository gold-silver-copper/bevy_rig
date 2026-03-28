use std::time::Duration;

use bevy::prelude::*;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

impl Position {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
        }
    }

    pub fn chebyshev_distance(self, other: Self) -> i32 {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }

    pub fn bresenham_line(self, other: Self) -> Vec<Self> {
        let dx = (other.x - self.x).abs();
        let dy = (other.y - self.y).abs();
        let sx = if self.x < other.x { 1 } else { -1 };
        let sy = if self.y < other.y { 1 } else { -1 };
        let mut err = dx - dy;

        let mut points = Vec::with_capacity((dx.max(dy) + 1) as usize);
        let mut current = self;

        loop {
            points.push(current);
            if current == other {
                break;
            }

            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                current.x += sx;
            }
            if e2 < dx {
                err += dx;
                current.y += sy;
            }
        }

        points
    }
}

#[derive(Component, Debug)]
pub struct Player;

#[derive(Component, Debug)]
pub struct Npc;

#[derive(Component, Clone, Debug)]
pub struct Name(pub String);

#[derive(Component, Clone, Copy, Debug)]
pub struct Glyph(pub char);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    Player,
    Npc,
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub turn: u64,
    pub speaker: Speaker,
    pub text: String,
}

#[derive(Component, Debug, Clone, Default)]
pub struct Memory {
    pub notes: Vec<String>,
    pub conversation: Vec<MemoryEntry>,
}

impl Memory {
    pub fn push(&mut self, turn: u64, speaker: Speaker, text: impl Into<String>) {
        self.conversation.push(MemoryEntry {
            turn,
            speaker,
            text: text.into(),
        });
        if self.conversation.len() > 24 {
            let overflow = self.conversation.len() - 24;
            self.conversation.drain(..overflow);
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct RigPersona {
    pub system_prompt: String,
    pub preferred_model: Option<String>,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct Wanderer {
    pub home: Position,
    pub next_direction: usize,
    pub radius: i32,
    pub vision_radius: i32,
}

#[derive(Component, Debug, Clone)]
pub struct NpcPace {
    pub think_timer: Timer,
    pub move_timer: Timer,
}

impl NpcPace {
    pub fn new(think_seconds: f32, move_seconds: f32, phase_offset_seconds: f32) -> Self {
        let think_seconds = think_seconds.max(0.05);
        let move_seconds = move_seconds.max(0.05);
        let mut think_timer = Timer::from_seconds(think_seconds, TimerMode::Repeating);
        let mut move_timer = Timer::from_seconds(move_seconds, TimerMode::Repeating);

        think_timer.tick(Duration::from_secs_f32(
            phase_offset_seconds.rem_euclid(think_seconds),
        ));
        move_timer.tick(Duration::from_secs_f32(
            phase_offset_seconds.rem_euclid(move_seconds),
        ));

        Self {
            think_timer,
            move_timer,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct PendingReply {
    pub request_id: Option<u64>,
}

impl PendingReply {
    pub fn waiting(self) -> bool {
        self.request_id.is_some()
    }
}

#[derive(Component, Debug, Clone, Copy, Default)]
pub struct PendingAction {
    pub request_id: Option<u64>,
}

impl PendingAction {
    pub fn waiting(self) -> bool {
        self.request_id.is_some()
    }
}

#[derive(Component, Debug, Clone, Default)]
pub struct MovePlan {
    pub target: Option<Position>,
    pub steps: Vec<Position>,
    pub summary: String,
    pub trace: String,
}

impl MovePlan {
    pub fn clear(&mut self) {
        self.target = None;
        self.steps.clear();
        self.summary.clear();
        self.trace.clear();
    }

    pub fn has_steps(&self) -> bool {
        !self.steps.is_empty()
    }
}
