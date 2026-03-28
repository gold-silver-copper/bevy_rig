mod graph;
mod runtime;
mod ui;

use bevy::prelude::*;
use runtime::NodeGraphRuntimePlugin;
use ui::NodeGraphEditorPlugin;

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(20, 21, 24)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Rig Graph".into(),
                resolution: (1680u32, 980u32).into(),
                resizable: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins((NodeGraphRuntimePlugin, NodeGraphEditorPlugin))
        .run();
}
