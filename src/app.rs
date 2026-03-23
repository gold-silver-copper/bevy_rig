use bevy_app::{App, MainScheduleOrder, Plugin, Update};
use bevy_ecs::{
    prelude::*,
    schedule::{IntoScheduleConfigs, Schedule, ScheduleLabel},
};

use crate::{
    context::{self, ContextIndex},
    diagnostics::{self, RuntimeDiagnostics},
    model::ModelRegistry,
    provider::ProviderCatalog,
    rig_runtime::{self, ProviderClientCache, RigRuntime},
    run::{self, CancelRun, RunAgent, RunCommitted, RunFailed, StreamCompleted, TextDelta},
    tool::{self, ToolCallCompleted, ToolCallFailed, ToolCallRequested, ToolRegistry},
    workflow::{self, RunWorkflow, WorkflowCommitted, WorkflowFailed},
};

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CatalogSync;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunPreparation;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunExecution;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunCommit;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Telemetry;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunPreparationSystems;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunExecutionSystems;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RigExecutionSystems;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolDispatchSystems;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct StreamApplySystems;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RunCommitSystems;

#[derive(Default)]
pub struct BevyRigPlugin;

impl Plugin for BevyRigPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ToolRegistry>()
            .init_resource::<ProviderCatalog>()
            .init_resource::<ModelRegistry>()
            .init_resource::<ContextIndex>()
            .init_resource::<RuntimeDiagnostics>()
            .init_resource::<ProviderClientCache>()
            .init_resource::<RigRuntime>()
            .add_message::<RunAgent>()
            .add_message::<CancelRun>()
            .add_message::<RunCommitted>()
            .add_message::<RunFailed>()
            .add_message::<RunWorkflow>()
            .add_message::<WorkflowCommitted>()
            .add_message::<WorkflowFailed>()
            .add_message::<ToolCallRequested>()
            .add_message::<ToolCallCompleted>()
            .add_message::<ToolCallFailed>()
            .add_message::<TextDelta>()
            .add_message::<StreamCompleted>()
            .add_schedule(Schedule::new(CatalogSync))
            .add_schedule(Schedule::new(RunPreparation))
            .add_schedule(Schedule::new(RunExecution))
            .add_schedule(Schedule::new(RunCommit))
            .add_schedule(Schedule::new(Telemetry))
            .configure_sets(RunPreparation, RunPreparationSystems)
            .configure_sets(
                RunExecution,
                (
                    RigExecutionSystems,
                    RunExecutionSystems,
                    ToolDispatchSystems,
                    StreamApplySystems,
                )
                    .chain(),
            )
            .configure_sets(RunCommit, RunCommitSystems)
            .add_systems(
                CatalogSync,
                (
                    context::rebuild_context_index,
                    tool::rebuild_tool_registry,
                    rig_runtime::prune_provider_client_cache,
                ),
            )
            .add_systems(
                RunPreparation,
                (
                    run::capture_run_requests,
                    workflow::capture_workflow_requests,
                    run::cancel_runs,
                    run::assemble_run_prompts,
                )
                    .chain()
                    .in_set(RunPreparationSystems),
            )
            .add_systems(
                RunExecution,
                rig_runtime::execute_rig_runs.in_set(RigExecutionSystems),
            )
            .add_systems(
                RunExecution,
                workflow::execute_workflow_invocations.in_set(RunExecutionSystems),
            )
            .add_systems(
                RunExecution,
                tool::queue_requested_tool_calls.before(ToolDispatchSystems),
            )
            .add_systems(
                RunExecution,
                tool::publish_tool_invocation_results.after(ToolDispatchSystems),
            )
            .add_systems(
                RunExecution,
                (
                    rig_runtime::resolve_rig_tool_results,
                    workflow::apply_workflow_tool_results,
                    workflow::apply_workflow_run_results,
                )
                    .after(tool::publish_tool_invocation_results),
            )
            .add_systems(
                RunExecution,
                (run::apply_text_deltas, run::finish_streams)
                    .chain()
                    .in_set(StreamApplySystems),
            )
            .add_systems(
                RunCommit,
                (
                    run::persist_completed_runs,
                    run::persist_failed_runs,
                    run::persist_cancelled_runs,
                    workflow::persist_completed_workflows,
                    workflow::persist_failed_workflows,
                )
                    .in_set(RunCommitSystems),
            )
            .add_systems(Telemetry, diagnostics::refresh_runtime_diagnostics);

        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.insert_after(Update, CatalogSync);
        order.insert_after(CatalogSync, RunPreparation);
        order.insert_after(RunPreparation, RunExecution);
        order.insert_after(RunExecution, RunCommit);
        order.insert_after(RunCommit, Telemetry);
    }
}
