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
            Duration::from_secs_f32(1. / 30.),
        )),
        RatatuiPlugins::default(),
        RoguelikePlugin,
    ));

    #[cfg(feature = "windowed")]
    app.add_plugins((
        DefaultPlugins,
        RatatuiPlugins {
            enable_input_forwarding: true,
            ..default()
        },
        RoguelikePlugin,
    ));

    app.run();
}
