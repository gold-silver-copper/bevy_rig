use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);
    app.add_systems(RunExecution, complete_echo_runs.in_set(RunExecutionSystems));

    let agent = {
        let world = app.world_mut();
        spawn_agent(world, AgentSpec::new("echo-agent", "mock-echo")).agent
    };

    app.world_mut()
        .write_message(RunAgent::new(agent, "hello from bevy_rig"));
    app.update();

    let session = app
        .world()
        .get::<PrimarySession>(agent)
        .expect("agent should have a primary session")
        .0;

    for (role, text) in collect_transcript(app.world(), session) {
        println!("{role:?}: {text}");
    }
}

fn complete_echo_runs(
    mut commands: Commands,
    runs: Query<(Entity, &RunRequest, &RunStatus), With<Run>>,
) {
    for (run, request, status) in &runs {
        if *status != RunStatus::Queued {
            continue;
        }

        mark_run_completed(&mut commands, run, format!("Echo: {}", request.prompt));
    }
}
