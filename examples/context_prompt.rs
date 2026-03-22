use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);
    app.add_systems(
        RunExecution,
        answer_from_assembled_prompt.in_set(RunExecutionSystems),
    );

    let agent = {
        let world = app.world_mut();
        let agent = spawn_agent(world, AgentSpec::new("context-agent", "mock-context")).agent;

        let bevy_context = spawn_context(
            world,
            ContextSource::Generated("bevy_schedules".to_string()),
            "Bevy schedules order systems into phases like Update and PostUpdate.",
        );
        let rig_context = spawn_context(
            world,
            ContextSource::Generated("rig_tools".to_string()),
            "Rig tools are callable units that can be exposed to language models.",
        );
        let unrelated_context = spawn_context(
            world,
            ContextSource::Generated("gardening".to_string()),
            "Tomatoes need full sun and consistent watering.",
        );

        attach_context(world, agent, bevy_context).expect("attach bevy context");
        attach_context(world, agent, rig_context).expect("attach rig context");
        attach_context(world, agent, unrelated_context).expect("attach unrelated context");
        agent
    };

    app.world_mut().write_message(RunAgent::new(
        agent,
        "How does Bevy order systems in schedules?",
    ));
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

fn answer_from_assembled_prompt(
    mut commands: Commands,
    runs: Query<(Entity, &RunPrompt, &RunStatus), With<Run>>,
) {
    for (run, prompt, status) in &runs {
        if *status != RunStatus::Queued {
            continue;
        }

        mark_run_completed(
            &mut commands,
            run,
            format!("Assembled prompt:\n{}", prompt.0),
        );
    }
}
