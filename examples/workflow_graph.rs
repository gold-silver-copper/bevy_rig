use bevy_app::App;
use bevy_rig::prelude::*;
use serde_json::json;

fn main() {
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

        let planner = spawn_agent_from_model(world, "planner", model)
            .expect("planner agent should spawn")
            .agent;
        let summarizer = spawn_agent_from_model(world, "summarizer", model)
            .expect("summarizer agent should spawn")
            .agent;
        let lookup_tool = world
            .spawn(ToolBundle::new(ToolSpec::new(
                "lookup",
                "Dummy lookup tool",
                json!({"type":"object","properties":{}}),
            )))
            .id();

        let workflow = spawn_workflow(
            world,
            WorkflowSpec::new("research", "Plan, call a tool, then summarize"),
        );
        let plan_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Agent, "plan")
            .expect("plan node");
        let tool_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Tool, "lookup")
            .expect("tool node");
        let summarize_node =
            spawn_workflow_node(world, workflow, WorkflowNodeKind::Agent, "summarize")
                .expect("summarize node");

        bind_workflow_node(world, plan_node, planner).expect("planner binding");
        bind_workflow_node(world, tool_node, lookup_tool).expect("tool binding");
        bind_workflow_node(world, summarize_node, summarizer).expect("summarizer binding");
        set_workflow_entry(world, workflow, plan_node).expect("entry node");
        connect_workflow_nodes(world, plan_node, tool_node, Some("needs_lookup"))
            .expect("plan -> tool");
        connect_workflow_nodes(world, tool_node, summarize_node, None::<String>)
            .expect("tool -> summarize");

        workflow
    };

    for node in reachable_workflow_nodes(app.world(), workflow).expect("workflow traversal") {
        let name = &app
            .world()
            .get::<WorkflowNodeName>(node)
            .expect("workflow node name")
            .0;
        let kind = app
            .world()
            .get::<WorkflowNodeKind>(node)
            .expect("workflow node kind");
        let binding = app
            .world()
            .get::<WorkflowBinding>(node)
            .expect("workflow binding")
            .0;

        println!("{name}: {kind:?} -> {binding:?}");
    }
}
