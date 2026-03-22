use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);
    app.add_systems(
        RunExecution,
        emit_streaming_response.in_set(RunExecutionSystems),
    );

    let agent = {
        let world = app.world_mut();
        spawn_agent(world, AgentSpec::new("stream-agent", "mock-stream")).agent
    };

    app.world_mut()
        .write_message(RunAgent::new(agent, "stream this reply"));
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

fn emit_streaming_response(
    mut deltas: MessageWriter<TextDelta>,
    mut completed: MessageWriter<StreamCompleted>,
    runs: Query<(Entity, &RunRequest, &RunStatus), With<Run>>,
) {
    for (run, request, status) in &runs {
        if *status != RunStatus::Queued {
            continue;
        }

        deltas.write(TextDelta::new(run, "Streamed "));
        deltas.write(TextDelta::new(run, "reply for "));
        deltas.write(TextDelta::new(run, &request.prompt));
        completed.write(StreamCompleted { run });
    }
}
