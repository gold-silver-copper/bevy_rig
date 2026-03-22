use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    if std::env::var_os("OPENAI_API_KEY").is_none() {
        println!("Skipping example: OPENAI_API_KEY is not set.");
        return;
    }

    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);

    let workflow = {
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
        let reviewer = spawn_agent_from_model(world, "reviewer", model)
            .expect("reviewer agent should spawn")
            .agent;

        let workflow = spawn_workflow(
            world,
            WorkflowSpec::new(
                "provider-review-flow",
                "Prompt rewrite followed by a real Rig call",
            ),
        );
        let prompt_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Prompt, "rewrite")
            .expect("prompt node");
        let agent_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Agent, "review")
            .expect("agent node");

        set_workflow_node_prompt_template(
            world,
            prompt_node,
            "Give a terse answer to this request:\n{{input}}",
        )
        .expect("prompt template");
        bind_workflow_node(world, agent_node, reviewer).expect("agent binding");
        set_workflow_entry(world, workflow, prompt_node).expect("entry node");
        connect_workflow_nodes(world, prompt_node, agent_node, None::<String>)
            .expect("prompt -> agent");

        workflow
    };

    app.world_mut().write_message(RunWorkflow::new(
        workflow,
        "Explain what bevy_rig is trying to unify.",
    ));
    app.update();

    let invocation = {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<WorkflowInvocation>>();
        query
            .iter(app.world())
            .next()
            .expect("workflow invocation should exist")
    };
    let session = app
        .world()
        .get::<WorkflowRunSession>(invocation)
        .expect("workflow invocation should have a session")
        .0;

    for (role, text) in collect_transcript(app.world(), session) {
        println!("{role:?}: {text}");
    }
}
