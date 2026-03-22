use bevy_ecs::prelude::*;

use crate::{
    run::{Run, RunStatus},
    workflow::{WorkflowInvocation, WorkflowRunStatus},
};

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeDiagnostics {
    pub runs_queued: usize,
    pub runs_running: usize,
    pub runs_completed: usize,
    pub runs_failed: usize,
    pub runs_cancelled: usize,
    pub workflows_queued: usize,
    pub workflows_running: usize,
    pub workflows_completed: usize,
    pub workflows_failed: usize,
}

pub fn refresh_runtime_diagnostics(
    mut diagnostics: ResMut<RuntimeDiagnostics>,
    runs: Query<&RunStatus, With<Run>>,
    workflows: Query<&WorkflowRunStatus, With<WorkflowInvocation>>,
) {
    let mut next = RuntimeDiagnostics::default();

    for status in &runs {
        match status {
            RunStatus::Queued => next.runs_queued += 1,
            RunStatus::Running => next.runs_running += 1,
            RunStatus::Completed => next.runs_completed += 1,
            RunStatus::Failed => next.runs_failed += 1,
            RunStatus::Cancelled => next.runs_cancelled += 1,
        }
    }

    for status in &workflows {
        match status {
            WorkflowRunStatus::Queued => next.workflows_queued += 1,
            WorkflowRunStatus::Running => next.workflows_running += 1,
            WorkflowRunStatus::Completed => next.workflows_completed += 1,
            WorkflowRunStatus::Failed => next.workflows_failed += 1,
        }
    }

    *diagnostics = next;
}
