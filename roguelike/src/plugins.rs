use bevy::prelude::*;

use crate::{
    events::{InteractIntent, MoveIntent, TalkIntent},
    map::TileMap,
    resources::{GameLog, PlayerNeeds, UiState, WorldClock},
    runtime::RigRuntime,
    systems::{input, render, setup, simulation},
};

pub struct RoguelikePlugin;

impl Plugin for RoguelikePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MoveIntent>()
            .add_message::<TalkIntent>()
            .add_message::<InteractIntent>()
            .insert_resource(TileMap::demo())
            .insert_resource(UiState::default())
            .insert_resource(GameLog::default())
            .insert_resource(PlayerNeeds::default())
            .insert_resource(WorldClock::default())
            .insert_resource(RigRuntime::new())
            .add_systems(Startup, setup::setup_world)
            .add_systems(PreUpdate, input::input_system)
            .add_systems(
                Update,
                (
                    simulation::advance_frame_system,
                    simulation::update_player_needs_system,
                    simulation::interact_with_cursor_system,
                    simulation::start_talk_system,
                    simulation::request_npc_move_plans_system,
                    simulation::advance_npc_move_plans_system,
                    simulation::resolve_move_intents_system,
                    simulation::settle_npc_move_plans_system,
                    simulation::poll_rig_responses_system,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, render::draw_system);
    }
}
