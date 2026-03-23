use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;
use serde_json::json;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin).add_systems(
        RunExecution,
        uppercase_text_tool.in_set(ToolDispatchSystems),
    );

    let workflow = {
        let world = app.world_mut();
        let reviewer = spawn_agent(world, AgentSpec::new("reviewer", "mock-reviewer")).agent;
        let uppercase_tool = world
            .spawn(ToolBundle::new(ToolSpec::new(
                "uppercase",
                "Uppercases the input",
                json!({"type":"object","properties":{"text":{"type":"string"}}}),
            )))
            .id();
        register_tool(world, uppercase_tool).expect("tool registration should work");

        let workflow = spawn_workflow(
            world,
            WorkflowSpec::new("review-flow", "Prompt, transform, then review"),
        );
        let prompt_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Prompt, "rewrite")
            .expect("prompt node");
        let tool_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Tool, "uppercase")
            .expect("tool node");
        let agent_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Agent, "review")
            .expect("agent node");

        set_workflow_node_prompt_template(world, prompt_node, "Rewrite for review: {{input}}")
            .expect("prompt template");
        bind_workflow_node(world, tool_node, uppercase_tool).expect("tool binding");
        bind_workflow_node(world, agent_node, reviewer).expect("agent binding");
        set_workflow_entry(world, workflow, prompt_node).expect("entry node");
        connect_workflow_nodes(world, prompt_node, tool_node, None::<String>)
            .expect("prompt -> tool");
        connect_workflow_nodes(world, tool_node, agent_node, None::<String>)
            .expect("tool -> agent");

        workflow
    };

    app.world_mut()
        .write_message(RunWorkflow::new(workflow, "please inspect bevy_rig"));
    for _ in 0..3 {
        app.update();
    }

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

fn uppercase_text_tool(
    mut commands: Commands,
    invocations: Query<(Entity, &ToolInvocationCall, &ToolInvocationStatus), With<ToolInvocation>>,
    specs: Query<&ToolSpec>,
) {
    for (invocation, call, status) in &invocations {
        if *status != ToolInvocationStatus::Queued {
            continue;
        }

        let Ok(spec) = specs.get(call.0.tool) else {
            continue;
        };
        if spec.name != "uppercase" {
            continue;
        }

        mark_tool_invocation_running(&mut commands, invocation);
        match call.0.args.get("text").and_then(|value| value.as_str()) {
            Some(text) => complete_tool_invocation(
                &mut commands,
                invocation,
                ToolOutput::text(text.to_ascii_uppercase()),
            ),
            None => fail_tool_invocation(
                &mut commands,
                invocation,
                ToolExecutionError::new("missing text argument").message,
            ),
        }
    }
}
