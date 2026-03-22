use bevy_app::App;
use bevy_ecs::hierarchy::ChildOf;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);

    let (provider, agent) = {
        let world = app.world_mut();

        let provider = spawn_provider(
            world,
            ProviderSpec::new(ProviderKind::OpenAi, "openai"),
            ProviderCapabilities::text_tooling(),
        );

        let chat_model = spawn_model(
            world,
            provider,
            ModelSpec::new("gpt-4o-mini").with_family("gpt-4o"),
            ModelCapabilities::chat_with_tools(),
            128_000,
        )
        .expect("chat model should register");

        spawn_model(
            world,
            provider,
            ModelSpec::new("text-embedding-3-large").with_family("embedding"),
            ModelCapabilities::embeddings_only(),
            8_192,
        )
        .expect("embedding model should register");

        let agent = spawn_agent_from_model(world, "planner", chat_model)
            .expect("agent should bind to a registered model")
            .agent;

        (provider, agent)
    };

    let world = app.world();
    let model = world
        .get::<AgentModelRef>(agent)
        .expect("agent should have a model ref")
        .0;
    let model_spec = world.get::<ModelSpec>(model).expect("model spec");
    let provider_spec = world
        .get::<ProviderSpec>(
            world
                .get::<ChildOf>(model)
                .expect("model should be owned by provider")
                .parent(),
        )
        .expect("provider spec");
    let registry = world.resource::<ModelRegistry>();

    let qualified_name = format!("{}/{}", provider_spec.label, model_spec.name);
    let resolved = registry
        .resolve_qualified(&qualified_name)
        .expect("qualified model lookup should succeed");

    println!("Agent model: {qualified_name}");
    println!("Resolved entity matches: {}", resolved == model);

    for model_entity in registry.models_for_provider(provider) {
        let spec = world.get::<ModelSpec>(model_entity).expect("model spec");
        let capabilities = world
            .get::<ModelCapabilities>(model_entity)
            .expect("model capabilities");
        println!(
            "Provider model: {} (chat={}, embeddings={}, tools={})",
            spec.name, capabilities.completions, capabilities.embeddings, capabilities.tools
        );
    }
}
