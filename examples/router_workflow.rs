use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_rig::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);

    let workflow = {
        let world = app.world_mut();
        let workflow = spawn_workflow(
            world,
            WorkflowSpec::new(
                "router",
                "Route Bevy prompts differently from everything else",
            ),
        );

        let router = spawn_workflow_node(world, workflow, WorkflowNodeKind::Router, "route")
            .expect("router node");
        let bevy_branch =
            spawn_workflow_node(world, workflow, WorkflowNodeKind::Prompt, "bevy_branch")
                .expect("bevy branch");
        let general_branch =
            spawn_workflow_node(world, workflow, WorkflowNodeKind::Prompt, "general_branch")
                .expect("general branch");

        set_workflow_node_prompt_template(
            world,
            bevy_branch,
            "Bevy-specific branch selected for: {{input}}",
        )
        .expect("bevy branch template");
        set_workflow_node_prompt_template(
            world,
            general_branch,
            "General branch selected for: {{input}}",
        )
        .expect("general branch template");
        set_workflow_entry(world, workflow, router).expect("entry node");
        connect_workflow_nodes(world, router, bevy_branch, Some("bevy")).expect("router -> bevy");
        connect_workflow_nodes(world, router, general_branch, None::<String>)
            .expect("router -> general");

        workflow
    };

    app.world_mut()
        .write_message(RunWorkflow::new(workflow, "bevy schedules and plugins"));
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
