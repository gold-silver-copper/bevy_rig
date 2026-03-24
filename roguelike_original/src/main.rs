#[cfg(not(feature = "windowed"))]
use std::time::Duration;

use bevy::prelude::*;
use bevy_ratatui::RatatuiPlugins;

use roguelike::plugins::RoguelikePlugin;

fn main() {
    let mut app = App::new();

    #[cfg(not(feature = "windowed"))]
    app.add_plugins((
        MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f32(1. / 60.),
        )),
        RatatuiPlugins::default(),
        RoguelikePlugin,
    ));

    #[cfg(feature = "windowed")]
    app.add_plugins((
        DefaultPlugins
            .set(ImagePlugin::default_nearest())
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Escape from Yerba Buena".into(),
                    fit_canvas_to_parent: true,
                    prevent_default_event_handling: false,
                    canvas: Some("#bevy".into()),
                    ..default()
                }),
                ..default()
            }),
        RatatuiPlugins {
            enable_input_forwarding: true,
            ..default()
        },
        RoguelikePlugin,
    ));

    app.run();
}
