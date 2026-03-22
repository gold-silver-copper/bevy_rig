mod app;
mod catalog;
mod domain;
mod runtime;
mod ui;

use app::RigStudioPlugin;
use bevy::{input_focus::InputFocus, prelude::*, window::Window, winit::WinitSettings};
use bevy_egui::EguiPlugin;

fn main() {
    App::new()
        .insert_resource(WinitSettings::desktop_app())
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.08)))
        .init_resource::<InputFocus>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Rig Entity Studio".into(),
                resolution: (1680, 980).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_plugins(RigStudioPlugin)
        .run();
}
