use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);

    let agent = {
        let world = app.world_mut();
        spawn_agent(world, AgentSpec::new("cancel-agent", "mock-cancel")).agent
    };

    app.world_mut()
        .write_message(RunAgent::new(agent, "this run will be cancelled"));
    app.update();

    let run = {
        let mut query = app.world_mut().query_filtered::<Entity, With<Run>>();
        query
            .iter(app.world())
            .next()
            .expect("a run should have been created")
    };

    app.world_mut()
        .write_message(CancelRun::new(run, Some("user requested cancellation")));
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
