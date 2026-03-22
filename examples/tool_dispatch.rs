use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;
use serde_json::json;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);
    app.add_systems(
        RunExecution,
        (
            queue_reverse_tool_calls.before(ToolDispatchSystems),
            finalize_tool_runs.after(ToolDispatchSystems),
        ),
    );

    let agent = {
        let world = app.world_mut();
        let handles = spawn_agent(world, AgentSpec::new("tool-agent", "mock-tool"));
        let tool = world
            .spawn(ToolBundle::new(ToolSpec::new(
                "reverse_text",
                "Reverses the prompt string",
                json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" }
                    },
                    "required": ["text"]
                }),
            )))
            .id();

        register_tool_system(world, tool, reverse_text).expect("tool registration should work");
        attach_tool(world, handles.agent, tool).expect("tool link should work");
        handles.agent
    };

    app.world_mut()
        .write_message(RunAgent::new(agent, "tool me"));
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

fn queue_reverse_tool_calls(
    mut messages: MessageWriter<ToolCallRequested>,
    agents: Query<&AgentToolRefs>,
    mut commands: Commands,
    runs: Query<(Entity, &RunOwner, &RunRequest, &RunStatus), With<Run>>,
) {
    for (run, owner, request, status) in &runs {
        if *status != RunStatus::Queued {
            continue;
        }

        let Ok(tools) = agents.get(owner.0) else {
            continue;
        };

        let Some(tool) = tools.0.first().copied() else {
            continue;
        };

        commands.entity(run).insert(RunStatus::Running);
        messages.write(ToolCallRequested {
            call: ToolCall::new(
                run,
                tool,
                json!({
                    "text": request.prompt
                }),
            ),
        });
    }
}

fn finalize_tool_runs(
    mut commands: Commands,
    mut completed: MessageReader<ToolCallCompleted>,
    mut failed: MessageReader<ToolCallFailed>,
) {
    for message in completed.read() {
        let output = message
            .output
            .as_text()
            .unwrap_or("tool completed without text output");
        mark_run_completed(&mut commands, message.call.run, output.to_string());
    }

    for message in failed.read() {
        mark_run_failed(&mut commands, message.call.run, message.error.clone());
    }
}

fn reverse_text(In(call): In<ToolCall>) -> ToolExecutionResult {
    let text = call
        .args
        .get("text")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolExecutionError::new("missing text argument"))?;

    Ok(ToolOutput::text(text.chars().rev().collect::<String>()))
}
