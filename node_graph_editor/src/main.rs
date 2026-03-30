mod catalog;
mod compile;
mod document;
mod graph;
mod providers;
mod runtime;
mod session;
mod ui;

use bevy::prelude::*;
use runtime::NodeGraphRuntimePlugin;
use ui::NodeGraphEditorPlugin;

fn main() {
    let asset_path = if std::path::Path::new("node_graph_editor/assets").exists() {
        "node_graph_editor/assets"
    } else {
        "assets"
    };

    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(20, 21, 24)))
        .add_plugins(
            DefaultPlugins
                .set(bevy::asset::AssetPlugin {
                    file_path: asset_path.into(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Bevy Rig Graph".into(),
                        resolution: (1680u32, 980u32).into(),
                        resizable: true,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins((NodeGraphRuntimePlugin, NodeGraphEditorPlugin))
        .run();
}
