use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

use crate::{
    catalog::build_registry,
    domain::{ChatState, ProviderRegistry},
    runtime::{RuntimeBridge, RuntimeEvent},
    ui::{configure_egui, render_chat_ui},
};

pub struct RigStudioPlugin;

impl Plugin for RigStudioPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RuntimeBridge::new())
            .insert_resource(build_registry())
            .init_resource::<ChatState>()
            .add_systems(Startup, setup_camera)
            .add_systems(Startup, initialize_chat_state.after(setup_camera))
            .add_systems(Update, refresh_provider_statuses)
            .add_systems(Update, poll_runtime_events)
            .add_systems(
                EguiPrimaryContextPass,
                (configure_egui, render_chat_ui).chain(),
            );
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn initialize_chat_state(mut chat: ResMut<ChatState>, registry: Res<ProviderRegistry>) {
    chat.selected_provider = registry
        .providers
        .iter()
        .find(|provider| provider.ready)
        .or_else(|| registry.providers.first())
        .map(|provider| provider.kind);

    if let Some(provider) = selected_provider(&registry, &chat) {
        if provider.ready {
            chat.status = Some(format!(
                "Ready on {} / {}.",
                provider.label, provider.default_model
            ));
            chat.push_log(format!(
                "Selected ready provider {} / {} at startup.",
                provider.label, provider.default_model
            ));
        } else {
            chat.status = Some(format!(
                "No ready provider detected. Currently inspecting {}.",
                provider.label
            ));
            chat.push_log(
                "No ready provider detected at startup. Start a local backend or provide an API key.",
            );
        }
    }
}

fn refresh_provider_statuses(
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
    mut registry: ResMut<ProviderRegistry>,
    mut chat: ResMut<ChatState>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(1.0, TimerMode::Repeating));
    if !timer.tick(time.delta()).just_finished() {
        return;
    }

    let mut changed = false;
    for provider in &mut registry.providers {
        let ready = crate::catalog::provider_ready(provider.kind);
        if provider.ready != ready {
            provider.ready = ready;
            changed = true;
        }
    }

    if changed
        && chat.history.is_empty()
        && !chat.sending
        && selected_provider(&registry, &chat).is_none_or(|provider| !provider.ready)
        && let Some(provider) = registry.providers.iter().find(|provider| provider.ready)
    {
        chat.selected_provider = Some(provider.kind);
        chat.status = Some(format!(
            "Switched to ready provider {} / {}.",
            provider.label, provider.default_model
        ));
        chat.push_log(format!(
            "Auto-selected ready provider {} / {}.",
            provider.label, provider.default_model
        ));
    }
}

fn poll_runtime_events(runtime: Res<RuntimeBridge>, mut chat: ResMut<ChatState>) {
    for event in runtime.receiver().try_iter() {
        match event {
            RuntimeEvent::ChatFinished(result) => {
                chat.sending = false;
                match result {
                    Ok(response) => {
                        chat.history.push(crate::domain::ChatMessage {
                            role: crate::domain::ChatRole::Assistant,
                            content: response,
                        });
                        chat.status = Some("Assistant replied.".to_string());
                        chat.push_log("Received assistant response.");
                    }
                    Err(error) => {
                        chat.status = Some(format!("Runtime error: {error}"));
                        chat.push_log(format!("Runtime error: {error}"));
                    }
                }
            }
        }
    }
}

pub fn selected_provider<'a>(
    registry: &'a ProviderRegistry,
    chat: &ChatState,
) -> Option<&'a crate::domain::ProviderEntry> {
    let kind = chat.selected_provider?;
    registry
        .providers
        .iter()
        .find(|provider| provider.kind == kind)
}

pub fn provider_state_label(provider: &crate::domain::ProviderEntry) -> &'static str {
    if provider.ready {
        "ready"
    } else if provider.is_local {
        "offline"
    } else {
        "missing env"
    }
}
