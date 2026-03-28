mod graph;
mod ui;

use bevy::prelude::*;
use ui::NodeGraphEditorPlugin;

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(20, 21, 24)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Node Graph".into(),
                resolution: (1600u32, 920u32).into(),
                resizable: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(NodeGraphEditorPlugin)
        .run();
}
