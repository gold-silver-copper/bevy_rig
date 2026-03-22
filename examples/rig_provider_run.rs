use bevy_app::App;
use bevy_rig::prelude::*;

fn main() {
    if std::env::var_os("OPENAI_API_KEY").is_none() {
        println!("Skipping example: OPENAI_API_KEY is not set.");
        return;
    }

    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);

    let agent = {
        let world = app.world_mut();
        let provider = spawn_provider(
            world,
            ProviderSpec::new(ProviderKind::OpenAi, "openai"),
            ProviderCapabilities::text_tooling(),
        );
        let model = spawn_model(
            world,
            provider,
            ModelSpec::new("gpt-4o-mini"),
            ModelCapabilities::chat_with_tools(),
            128_000,
        )
        .expect("model should register");

        spawn_agent_from_model(world, "ops-agent", model)
            .expect("agent should spawn from the provider-backed model")
            .agent
    };

    app.world_mut().write_message(RunAgent::new(
        agent,
        "Reply in one short sentence describing what bevy_rig is.",
    ));
    app.update();

    let session = app
        .world()
        .get::<PrimarySession>(agent)
        .expect("agent should have a session")
        .0;

    for (role, text) in collect_transcript(app.world(), session) {
        println!("{role:?}: {text}");
    }
}
