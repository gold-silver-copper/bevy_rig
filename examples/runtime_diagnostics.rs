use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);
    app.add_systems(RunExecution, complete_echo_runs.in_set(RunExecutionSystems));

    let (agent, workflow) = {
        let world = app.world_mut();
        let agent = spawn_agent(world, AgentSpec::new("diag-agent", "mock-diag")).agent;

        let workflow = spawn_workflow(
            world,
            WorkflowSpec::new("diag-workflow", "Simple prompt-only workflow"),
        );
        let prompt_node = spawn_workflow_node(world, workflow, WorkflowNodeKind::Prompt, "prompt")
            .expect("prompt node");
        set_workflow_node_prompt_template(world, prompt_node, "Workflow handled: {{input}}")
            .expect("prompt template");
        set_workflow_entry(world, workflow, prompt_node).expect("entry node");

        (agent, workflow)
    };

    app.world_mut()
        .write_message(RunAgent::new(agent, "diagnostic run"));
    app.world_mut()
        .write_message(RunWorkflow::new(workflow, "diagnostic workflow"));
    app.update();

    let diagnostics = app.world().resource::<RuntimeDiagnostics>();
    println!(
        "runs: queued={}, running={}, completed={}, failed={}, cancelled={}",
        diagnostics.runs_queued,
        diagnostics.runs_running,
        diagnostics.runs_completed,
        diagnostics.runs_failed,
        diagnostics.runs_cancelled
    );
    println!(
        "workflows: queued={}, running={}, completed={}, failed={}",
        diagnostics.workflows_queued,
        diagnostics.workflows_running,
        diagnostics.workflows_completed,
        diagnostics.workflows_failed
    );
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
