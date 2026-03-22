use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::{
    app::{provider_state_label, selected_provider},
    domain::{ChatMessage, ChatRole, ChatState, ProviderEntry, ProviderRegistry},
    runtime::{RuntimeBridge, RuntimeMessage, RuntimeRequest},
};

pub fn configure_egui(mut contexts: EguiContexts, mut configured: Local<bool>) {
    if *configured {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let mut visuals = egui::Visuals::dark();
    visuals.window_corner_radius = egui::CornerRadius::same(4);
    visuals.panel_fill = egui::Color32::from_rgb(14, 16, 22);
    visuals.extreme_bg_color = egui::Color32::from_rgb(10, 12, 17);
    visuals.faint_bg_color = egui::Color32::from_rgb(22, 25, 34);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(34, 59, 46);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(28, 39, 53);
    visuals.selection.bg_fill = egui::Color32::from_rgb(34, 59, 46);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.override_text_style = Some(egui::TextStyle::Monospace);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 8.0);
    ctx.set_style(style);

    *configured = true;
}

pub fn render_chat_ui(
    mut contexts: EguiContexts,
    registry: Res<ProviderRegistry>,
    runtime: Res<RuntimeBridge>,
    mut chat: ResMut<ChatState>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let mut select_provider_action = None;
    let mut select_message_action = None;
    let mut toggle_role_action = false;
    let mut delete_message_action = false;
    let mut clear_draft_action = false;
    let mut new_chat_action = false;
    let mut send_action =
        ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Enter));

    egui::TopBottomPanel::top("top_bar")
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("bevy_rig");
                ui.separator();

                if let Some(provider) = selected_provider(&registry, &chat) {
                    ui.label(format!(
                        "provider={} / {} [{}]",
                        provider.label,
                        provider.default_model,
                        provider_state_label(provider)
                    ));
                } else {
                    ui.label("provider=none");
                }

                ui.separator();
                ui.label(format!("messages={}", chat.history.len()));
                ui.separator();
                ui.label(if chat.sending {
                    "sending=yes"
                } else {
                    "sending=no"
                });
                ui.separator();
                ui.label("Cmd/Ctrl+Enter send");
            });

            if let Some(status) = &chat.status {
                ui.add_space(4.0);
                ui.colored_label(status_color(status), status);
            }
        });

    egui::TopBottomPanel::bottom("composer_panel")
        .resizable(true)
        .default_height(240.0)
        .min_height(180.0)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Composer");
                ui.separator();
                ui.small(
                    "Cmd/Ctrl+Enter sends. Edit previous messages from the transcript or the history editor.",
                );
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !chat.sending && !chat.draft.trim().is_empty(),
                        egui::Button::new("Send"),
                    )
                    .clicked()
                {
                    send_action = true;
                }

                if ui.button("Clear draft").clicked() {
                    clear_draft_action = true;
                }

                if ui
                    .add_enabled(!chat.sending, egui::Button::new("New chat"))
                    .clicked()
                {
                    new_chat_action = true;
                }
            });

            ui.add_space(8.0);
            ui.add_sized(
                ui.available_size(),
                egui::TextEdit::multiline(&mut chat.draft)
                    .desired_width(f32::INFINITY)
                    .hint_text("Type a prompt here. The next send uses the current edited history.")
                    .lock_focus(true),
            );
        });

    egui::SidePanel::left("providers")
        .default_width(300.0)
        .min_width(240.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Providers");
            ui.label(format!(
                "{} ready / {} total",
                registry
                    .providers
                    .iter()
                    .filter(|provider| provider.ready)
                    .count(),
                registry.providers.len()
            ));
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    for provider in &registry.providers {
                        let selected = chat.selected_provider == Some(provider.kind);
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(selected, provider.label)
                                    .on_hover_text(provider.detail.as_str())
                                    .clicked()
                                {
                                    select_provider_action = Some(provider.kind);
                                }

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.colored_label(
                                            provider_state_color(provider),
                                            provider_state_label(provider),
                                        );
                                    },
                                );
                            });

                            ui.horizontal_wrapped(|ui| {
                                ui.small(provider.default_model);
                                ui.separator();
                                ui.small(provider.detail.as_str());
                            });
                        });
                    }
                });

            ui.separator();
            ui.heading("Recent");
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    if chat.logs.is_empty() {
                        ui.small("No activity yet.");
                    } else {
                        for line in chat.logs.iter().rev() {
                            ui.small(line);
                        }
                    }
                });
        });

    egui::SidePanel::right("history_editor")
        .default_width(360.0)
        .min_width(280.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("History Editor");
            ui.label("Select a transcript entry to edit it live.");
            ui.separator();

            if let Some(index) = chat.selected_message {
                let sending = chat.sending;
                if let Some(message) = chat.history.get_mut(index) {
                    ui.horizontal(|ui| {
                        ui.label(format!("message #{}", index + 1));
                        ui.separator();
                        ui.colored_label(role_color(message.role), role_label(message.role));
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Toggle role").clicked() {
                            toggle_role_action = true;
                        }
                        if ui
                            .add_enabled(!sending, egui::Button::new("Delete"))
                            .clicked()
                        {
                            delete_message_action = true;
                        }
                    });

                    ui.add_space(8.0);
                    ui.add_sized(
                        ui.available_size(),
                        egui::TextEdit::multiline(&mut message.content)
                            .desired_width(f32::INFINITY)
                            .lock_focus(true)
                            .frame(true),
                    );
                } else {
                    chat.selected_message = None;
                    ui.small("The selected message no longer exists.");
                }
            } else {
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.small("No message selected.");
                    },
                );
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Transcript");
        if let Some(provider) = selected_provider(&registry, &chat) {
            ui.small(format!(
                "Routing through {} / {} [{}]",
                provider.label,
                provider.default_model,
                provider_state_label(provider)
            ));
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if chat.history.is_empty() {
                    ui.small("No messages yet. Type a prompt below and send it.");
                } else {
                    for (index, message) in chat.history.iter().enumerate() {
                        let selected = chat.selected_message == Some(index);
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(
                                        selected,
                                        format!("[{} #{}]", role_label(message.role), index + 1),
                                    )
                                    .clicked()
                                {
                                    select_message_action = Some(index);
                                }
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(message.content.as_str())
                                    .monospace()
                                    .color(role_color(message.role)),
                            );
                        });
                    }
                }
            });
    });

    if let Some(kind) = select_provider_action {
        chat.selected_provider = Some(kind);
        if let Some(provider) = registry
            .providers
            .iter()
            .find(|provider| provider.kind == kind)
        {
            chat.status = Some(format!(
                "Selected {} / {} [{}].",
                provider.label,
                provider.default_model,
                provider_state_label(provider)
            ));
            chat.push_log(format!(
                "Selected provider {} / {}.",
                provider.label, provider.default_model
            ));
        }
    }

    if let Some(index) = select_message_action {
        chat.selected_message = Some(index);
    }

    if toggle_role_action && let Some(index) = chat.selected_message {
        let toggled_role = if let Some(message) = chat.history.get_mut(index) {
            message.role = match message.role {
                ChatRole::User => ChatRole::Assistant,
                ChatRole::Assistant => ChatRole::User,
            };
            Some(message.role)
        } else {
            None
        };

        if let Some(role) = toggled_role {
            chat.push_log(format!(
                "Toggled role for message #{} to {}.",
                index + 1,
                role_label(role)
            ));
        }
    }

    if delete_message_action
        && let Some(index) = chat.selected_message
        && index < chat.history.len()
    {
        chat.history.remove(index);
        chat.selected_message = if chat.history.is_empty() {
            None
        } else {
            Some(index.min(chat.history.len() - 1))
        };
        chat.push_log(format!("Deleted message #{}.", index + 1));
    }

    if clear_draft_action {
        chat.draft.clear();
    }

    if new_chat_action {
        chat.history.clear();
        chat.selected_message = None;
        chat.draft.clear();
        chat.status = Some("Started a fresh chat.".to_string());
        chat.push_log("Started a fresh chat.");
    }

    if send_action {
        attempt_send(&registry, &runtime, &mut chat);
    }
}

fn attempt_send(registry: &ProviderRegistry, runtime: &RuntimeBridge, chat: &mut ChatState) {
    if chat.sending {
        chat.status = Some("A request is already running.".to_string());
        return;
    }

    let prompt = chat.draft.trim().to_string();
    if prompt.is_empty() {
        chat.status = Some("Draft is empty.".to_string());
        return;
    }

    let Some(provider) = selected_provider(registry, chat) else {
        chat.status = Some("No provider selected.".to_string());
        return;
    };

    if !provider.ready {
        chat.status = Some(format!(
            "{} is not ready. Expected {}.",
            provider.label, provider.detail
        ));
        chat.push_log(format!(
            "Blocked send because {} is not ready.",
            provider.label
        ));
        return;
    }

    let request = RuntimeRequest {
        provider: provider.kind,
        model: provider.default_model.to_string(),
        prompt: prompt.clone(),
        history: chat
            .history
            .iter()
            .map(|message| RuntimeMessage {
                role: message.role,
                content: message.content.clone(),
            })
            .collect(),
    };

    runtime.spawn_chat(request);
    chat.sending = true;
    chat.status = Some(format!(
        "Sending to {} / {}.",
        provider.label, provider.default_model
    ));
    chat.push_log(format!(
        "Sent prompt to {} / {}.",
        provider.label, provider.default_model
    ));
    chat.history.push(ChatMessage {
        role: ChatRole::User,
        content: prompt,
    });
    chat.draft.clear();
    chat.selected_message = None;
}

fn role_label(role: ChatRole) -> &'static str {
    match role {
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

fn role_color(role: ChatRole) -> egui::Color32 {
    match role {
        ChatRole::User => egui::Color32::from_rgb(230, 197, 92),
        ChatRole::Assistant => egui::Color32::from_rgb(110, 214, 147),
    }
}

fn provider_state_color(provider: &ProviderEntry) -> egui::Color32 {
    if provider.ready {
        egui::Color32::from_rgb(110, 214, 147)
    } else if provider.is_local {
        egui::Color32::from_rgb(216, 107, 97)
    } else {
        egui::Color32::from_rgb(215, 170, 83)
    }
}

fn status_color(status: &str) -> egui::Color32 {
    if status.to_ascii_lowercase().contains("error")
        || status.to_ascii_lowercase().contains("not ready")
    {
        egui::Color32::from_rgb(216, 107, 97)
    } else {
        egui::Color32::from_rgb(175, 184, 193)
    }
}
