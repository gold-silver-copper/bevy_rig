use std::collections::{HashSet, VecDeque};

use bevy_ecs::{
    hierarchy::{ChildOf, Children},
    prelude::*,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::{
    agent::AgentSpec,
    rig_runtime::execute_agent_prompt,
    session::{ChatMessageBundle, ChatMessageRole, SessionBundle},
    tool::{ToolCall, dispatch_tool},
};

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Workflow;

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    pub description: String,
}

impl WorkflowSpec {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowEntry(pub Entity);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowNode;

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowNodeKind {
    Agent,
    Tool,
    Router,
    Extractor,
    Prompt,
    Output,
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct WorkflowNodeName(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowEdge {
    pub target: Entity,
    pub condition: Option<String>,
}

impl WorkflowEdge {
    pub fn new(target: Entity, condition: Option<impl Into<String>>) -> Self {
        Self {
            target,
            condition: condition.map(Into::into),
        }
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowEdges(pub Vec<WorkflowEdge>);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowBinding(pub Entity);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct WorkflowNodePromptTemplate(pub String);

#[derive(Bundle)]
pub struct WorkflowBundle {
    pub workflow: Workflow,
    pub spec: WorkflowSpec,
}

impl WorkflowBundle {
    pub fn new(spec: WorkflowSpec) -> Self {
        Self {
            workflow: Workflow,
            spec,
        }
    }
}

#[derive(Bundle)]
pub struct WorkflowNodeBundle {
    pub node: WorkflowNode,
    pub name: WorkflowNodeName,
    pub kind: WorkflowNodeKind,
    pub edges: WorkflowEdges,
    pub child_of: ChildOf,
}

impl WorkflowNodeBundle {
    pub fn new(workflow: Entity, kind: WorkflowNodeKind, name: impl Into<String>) -> Self {
        Self {
            node: WorkflowNode,
            name: WorkflowNodeName(name.into()),
            kind,
            edges: WorkflowEdges::default(),
            child_of: ChildOf(workflow),
        }
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowInvocation;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowRunWorkflow(pub Entity);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowRunSession(pub Entity);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct WorkflowRunRequest {
    pub prompt: String,
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct WorkflowRunCursor {
    pub current_prompt: String,
    pub remaining: VecDeque<Entity>,
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowRunTrace(pub Vec<String>);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub enum WorkflowRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct WorkflowRunResult(pub String);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct WorkflowRunFailure(pub String);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkflowRunFinalized;

#[derive(Bundle)]
pub struct WorkflowInvocationBundle {
    pub invocation: WorkflowInvocation,
    pub workflow: WorkflowRunWorkflow,
    pub session: WorkflowRunSession,
    pub request: WorkflowRunRequest,
    pub cursor: WorkflowRunCursor,
    pub trace: WorkflowRunTrace,
    pub status: WorkflowRunStatus,
}

impl WorkflowInvocationBundle {
    pub fn new(
        workflow: Entity,
        session: Entity,
        entry: Entity,
        prompt: impl Into<String>,
    ) -> Self {
        let prompt = prompt.into();
        Self {
            invocation: WorkflowInvocation,
            workflow: WorkflowRunWorkflow(workflow),
            session: WorkflowRunSession(session),
            request: WorkflowRunRequest {
                prompt: prompt.clone(),
            },
            cursor: WorkflowRunCursor {
                current_prompt: prompt,
                remaining: VecDeque::from([entry]),
            },
            trace: WorkflowRunTrace::default(),
            status: WorkflowRunStatus::Queued,
        }
    }
}

#[derive(Message, Clone, Debug)]
pub struct RunWorkflow {
    pub workflow: Entity,
    pub prompt: String,
}

impl RunWorkflow {
    pub fn new(workflow: Entity, prompt: impl Into<String>) -> Self {
        Self {
            workflow,
            prompt: prompt.into(),
        }
    }
}

#[derive(Message, Clone, Copy, Debug)]
pub struct WorkflowCommitted {
    pub invocation: Entity,
}

#[derive(Message, Clone, Debug)]
pub struct WorkflowFailed {
    pub invocation: Option<Entity>,
    pub error: String,
}

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("workflow entity {0:?} does not exist or is missing WorkflowSpec")]
    UnknownWorkflow(Entity),
    #[error("node entity {0:?} does not exist or is missing WorkflowNode")]
    UnknownNode(Entity),
    #[error("node entity {node:?} does not belong to workflow {workflow:?}")]
    NodeNotInWorkflow { workflow: Entity, node: Entity },
    #[error("workflow entity {0:?} does not have an entry node")]
    MissingEntry(Entity),
    #[error("target entity {0:?} does not exist")]
    UnknownTarget(Entity),
}

#[derive(Debug, Error)]
enum WorkflowExecutionError {
    #[error("workflow node {0:?} is missing a name or kind")]
    InvalidNode(Entity),
    #[error("workflow node {0:?} is missing a binding")]
    MissingBinding(Entity),
    #[error("workflow agent node {node:?} failed: {error}")]
    AgentFailure { node: Entity, error: String },
    #[error("workflow tool node {node:?} failed: {error}")]
    ToolFailure { node: Entity, error: String },
}

pub fn spawn_workflow(world: &mut World, spec: WorkflowSpec) -> Entity {
    world.spawn(WorkflowBundle::new(spec)).id()
}

pub fn spawn_workflow_node(
    world: &mut World,
    workflow: Entity,
    kind: WorkflowNodeKind,
    name: impl Into<String>,
) -> Result<Entity, WorkflowError> {
    if world.get::<WorkflowSpec>(workflow).is_none() {
        return Err(WorkflowError::UnknownWorkflow(workflow));
    }

    Ok(world
        .spawn(WorkflowNodeBundle::new(workflow, kind, name))
        .id())
}

pub fn bind_workflow_node(
    world: &mut World,
    node: Entity,
    target: Entity,
) -> Result<(), WorkflowError> {
    if world.get::<WorkflowNode>(node).is_none() {
        return Err(WorkflowError::UnknownNode(node));
    }
    if world.get_entity(target).is_err() {
        return Err(WorkflowError::UnknownTarget(target));
    }

    world.entity_mut(node).insert(WorkflowBinding(target));
    Ok(())
}

pub fn set_workflow_node_prompt_template(
    world: &mut World,
    node: Entity,
    template: impl Into<String>,
) -> Result<(), WorkflowError> {
    if world.get::<WorkflowNode>(node).is_none() {
        return Err(WorkflowError::UnknownNode(node));
    }

    world
        .entity_mut(node)
        .insert(WorkflowNodePromptTemplate(template.into()));
    Ok(())
}

pub fn set_workflow_entry(
    world: &mut World,
    workflow: Entity,
    node: Entity,
) -> Result<(), WorkflowError> {
    ensure_workflow_node_membership(world, workflow, node)?;
    world.entity_mut(workflow).insert(WorkflowEntry(node));
    Ok(())
}

pub fn connect_workflow_nodes(
    world: &mut World,
    from: Entity,
    to: Entity,
    condition: Option<impl Into<String>>,
) -> Result<(), WorkflowError> {
    if world.get::<WorkflowNode>(from).is_none() {
        return Err(WorkflowError::UnknownNode(from));
    }
    if world.get::<WorkflowNode>(to).is_none() {
        return Err(WorkflowError::UnknownNode(to));
    }

    let mut from_entity = world.entity_mut(from);
    let mut edges = from_entity
        .get_mut::<WorkflowEdges>()
        .expect("workflow nodes always include WorkflowEdges");
    edges.0.push(WorkflowEdge::new(to, condition));
    Ok(())
}

pub fn workflow_nodes(world: &World, workflow: Entity) -> Result<Vec<Entity>, WorkflowError> {
    if world.get::<WorkflowSpec>(workflow).is_none() {
        return Err(WorkflowError::UnknownWorkflow(workflow));
    }

    let Some(children) = world.get::<Children>(workflow) else {
        return Ok(Vec::new());
    };

    Ok(children
        .iter()
        .filter(|child| world.get::<WorkflowNode>(*child).is_some())
        .collect())
}

pub fn reachable_workflow_nodes(
    world: &World,
    workflow: Entity,
) -> Result<Vec<Entity>, WorkflowError> {
    if world.get::<WorkflowSpec>(workflow).is_none() {
        return Err(WorkflowError::UnknownWorkflow(workflow));
    }

    let entry = world
        .get::<WorkflowEntry>(workflow)
        .map(|entry| entry.0)
        .ok_or(WorkflowError::MissingEntry(workflow))?;
    ensure_workflow_node_membership(world, workflow, entry)?;

    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([entry]);
    let mut ordered = Vec::new();

    while let Some(node) = queue.pop_front() {
        if !visited.insert(node) {
            continue;
        }
        ordered.push(node);

        if let Some(edges) = world.get::<WorkflowEdges>(node) {
            for edge in &edges.0 {
                queue.push_back(edge.target);
            }
        }
    }

    Ok(ordered)
}

pub fn capture_workflow_requests(
    mut commands: Commands,
    mut requests: MessageReader<RunWorkflow>,
    workflows: Query<(&WorkflowSpec, &WorkflowEntry)>,
    mut failures: MessageWriter<WorkflowFailed>,
) {
    for request in requests.read() {
        let Ok((spec, entry)) = workflows.get(request.workflow) else {
            failures.write(WorkflowFailed {
                invocation: None,
                error: format!(
                    "workflow {:?} is missing spec or entry node",
                    request.workflow
                ),
            });
            continue;
        };

        let session = commands
            .spawn(SessionBundle::new(format!(
                "{} workflow session",
                spec.name
            )))
            .id();
        commands.spawn(ChatMessageBundle::new(
            session,
            ChatMessageRole::User,
            request.prompt.clone(),
        ));
        commands.spawn(WorkflowInvocationBundle::new(
            request.workflow,
            session,
            entry.0,
            request.prompt.clone(),
        ));
    }
}

pub fn execute_workflow_invocations(world: &mut World) {
    let invocations: Vec<Entity> = {
        let mut query = world
            .query_filtered::<Entity, (With<WorkflowInvocation>, Without<WorkflowRunFinalized>)>();
        query.iter(world).collect()
    };

    for invocation in invocations {
        let Some(status) = world.get::<WorkflowRunStatus>(invocation).cloned() else {
            continue;
        };
        if !matches!(
            status,
            WorkflowRunStatus::Queued | WorkflowRunStatus::Running
        ) {
            continue;
        }

        let Some(workflow) = world.get::<WorkflowRunWorkflow>(invocation).copied() else {
            insert_workflow_failure(world, invocation, "workflow invocation is missing workflow");
            continue;
        };
        let Some(cursor) = world.get::<WorkflowRunCursor>(invocation).cloned() else {
            insert_workflow_failure(world, invocation, "workflow invocation is missing cursor");
            continue;
        };
        let mut current_prompt = cursor.current_prompt;
        let mut remaining = cursor.remaining;
        let mut trace = world
            .get::<WorkflowRunTrace>(invocation)
            .cloned()
            .unwrap_or_default();

        let mut failure = None;
        while let Some(node) = remaining.pop_front() {
            match execute_workflow_node(world, workflow.0, invocation, node, &current_prompt) {
                Ok(step) => {
                    trace.0.push(step.trace_line);
                    current_prompt = step.next_prompt;
                    for target in step.next_nodes {
                        remaining.push_back(target);
                    }
                }
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }

        let mut entity = world.entity_mut(invocation);
        entity.insert(trace);

        if let Some(error) = failure {
            entity.insert(WorkflowRunStatus::Failed);
            entity.insert(WorkflowRunFailure(error));
            entity.remove::<WorkflowRunResult>();
            continue;
        }

        entity.insert(WorkflowRunCursor {
            current_prompt: current_prompt.clone(),
            remaining,
        });
        entity.insert(WorkflowRunStatus::Completed);
        entity.insert(WorkflowRunResult(current_prompt));
        entity.remove::<WorkflowRunFailure>();
    }
}

pub fn persist_completed_workflows(
    mut commands: Commands,
    invocations: Query<
        (
            Entity,
            &WorkflowRunSession,
            &WorkflowRunResult,
            &WorkflowRunTrace,
            &WorkflowRunStatus,
        ),
        (With<WorkflowInvocation>, Without<WorkflowRunFinalized>),
    >,
    mut committed: MessageWriter<WorkflowCommitted>,
) {
    for (invocation, session, result, trace, status) in &invocations {
        if *status != WorkflowRunStatus::Completed {
            continue;
        }

        commands.spawn(ChatMessageBundle::new(
            session.0,
            ChatMessageRole::Assistant,
            render_workflow_result(trace, &result.0),
        ));
        commands.entity(invocation).insert(WorkflowRunFinalized);
        committed.write(WorkflowCommitted { invocation });
    }
}

pub fn persist_failed_workflows(
    mut commands: Commands,
    invocations: Query<
        (
            Entity,
            &WorkflowRunSession,
            &WorkflowRunFailure,
            &WorkflowRunStatus,
        ),
        (With<WorkflowInvocation>, Without<WorkflowRunFinalized>),
    >,
    mut failures: MessageWriter<WorkflowFailed>,
) {
    for (invocation, session, failure, status) in &invocations {
        if *status != WorkflowRunStatus::Failed {
            continue;
        }

        commands.spawn(ChatMessageBundle::new(
            session.0,
            ChatMessageRole::System,
            format!("workflow failed: {}", failure.0),
        ));
        commands.entity(invocation).insert(WorkflowRunFinalized);
        failures.write(WorkflowFailed {
            invocation: Some(invocation),
            error: failure.0.clone(),
        });
    }
}

fn ensure_workflow_node_membership(
    world: &World,
    workflow: Entity,
    node: Entity,
) -> Result<(), WorkflowError> {
    if world.get::<WorkflowSpec>(workflow).is_none() {
        return Err(WorkflowError::UnknownWorkflow(workflow));
    }
    let Some(parent) = world.get::<ChildOf>(node) else {
        return Err(WorkflowError::UnknownNode(node));
    };
    if world.get::<WorkflowNode>(node).is_none() {
        return Err(WorkflowError::UnknownNode(node));
    }
    if parent.parent() != workflow {
        return Err(WorkflowError::NodeNotInWorkflow { workflow, node });
    }
    Ok(())
}

#[derive(Debug)]
struct WorkflowStepOutcome {
    next_prompt: String,
    next_nodes: Vec<Entity>,
    trace_line: String,
}

fn execute_workflow_node(
    world: &mut World,
    workflow: Entity,
    invocation: Entity,
    node: Entity,
    input: &str,
) -> Result<WorkflowStepOutcome, WorkflowExecutionError> {
    let name = world
        .get::<WorkflowNodeName>(node)
        .map(|name| name.0.clone())
        .ok_or(WorkflowExecutionError::InvalidNode(node))?;
    let kind = world
        .get::<WorkflowNodeKind>(node)
        .cloned()
        .ok_or(WorkflowExecutionError::InvalidNode(node))?;
    let edges = world
        .get::<WorkflowEdges>(node)
        .cloned()
        .unwrap_or_default();

    let next_prompt = match kind {
        WorkflowNodeKind::Agent => {
            let binding = world
                .get::<WorkflowBinding>(node)
                .copied()
                .ok_or(WorkflowExecutionError::MissingBinding(node))?;
            let spec = world.get::<AgentSpec>(binding.0).cloned().ok_or(
                WorkflowExecutionError::AgentFailure {
                    node,
                    error: format!("agent {:?} is missing AgentSpec", binding.0),
                },
            )?;

            if spec.provider.is_some() {
                let session = world.get::<WorkflowRunSession>(invocation).copied().ok_or(
                    WorkflowExecutionError::AgentFailure {
                        node,
                        error: format!(
                            "workflow invocation {:?} is missing its session",
                            invocation
                        ),
                    },
                )?;

                execute_agent_prompt(world, binding.0, input, Some(session.0), Some(input))
                    .map_err(|error| WorkflowExecutionError::AgentFailure {
                        node,
                        error: error.to_string(),
                    })?
            } else {
                format!("{} ({}) processed: {}", spec.name, spec.model, input)
            }
        }
        WorkflowNodeKind::Tool => {
            let binding = world
                .get::<WorkflowBinding>(node)
                .copied()
                .ok_or(WorkflowExecutionError::MissingBinding(node))?;
            let call = ToolCall::new(
                invocation,
                binding.0,
                json!({
                    "text": input,
                    "prompt": input,
                    "input": input,
                    "node": name,
                    "workflow": format!("{workflow:?}"),
                }),
            );
            match dispatch_tool(world, call) {
                Ok(Ok(output)) => output
                    .as_text()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| output.value.to_string()),
                Ok(Err(error)) => {
                    return Err(WorkflowExecutionError::ToolFailure {
                        node,
                        error: error.message,
                    });
                }
                Err(error) => {
                    return Err(WorkflowExecutionError::ToolFailure {
                        node,
                        error: error.to_string(),
                    });
                }
            }
        }
        WorkflowNodeKind::Prompt => world
            .get::<WorkflowNodePromptTemplate>(node)
            .map(|template| template.0.replace("{{input}}", input))
            .unwrap_or_else(|| input.to_string()),
        WorkflowNodeKind::Router => input.to_string(),
        WorkflowNodeKind::Extractor => format!("extracted: {input}"),
        WorkflowNodeKind::Output => input.to_string(),
    };

    let next_nodes = select_workflow_targets(&kind, &next_prompt, &edges.0);
    let trace_line = format!("{name} [{kind:?}] => {next_prompt}");

    Ok(WorkflowStepOutcome {
        next_prompt,
        next_nodes,
        trace_line,
    })
}

fn select_workflow_targets(
    kind: &WorkflowNodeKind,
    prompt: &str,
    edges: &[WorkflowEdge],
) -> Vec<Entity> {
    if matches!(kind, WorkflowNodeKind::Router) {
        let mut selected = Vec::new();
        for edge in edges {
            match &edge.condition {
                Some(condition) if prompt.contains(condition) => selected.push(edge.target),
                None if selected.is_empty() => selected.push(edge.target),
                _ => {}
            }
        }
        return selected;
    }

    edges.iter().map(|edge| edge.target).collect()
}

fn render_workflow_result(trace: &WorkflowRunTrace, result: &str) -> String {
    if trace.0.is_empty() {
        return result.to_string();
    }

    format!(
        "Workflow trace:\n{}\n\nFinal output:\n{}",
        trace.0.join("\n"),
        result
    )
}

fn insert_workflow_failure(world: &mut World, invocation: Entity, error: impl Into<String>) {
    let error = error.into();
    let mut entity = world.entity_mut(invocation);
    entity.insert(WorkflowRunStatus::Failed);
    entity.insert(WorkflowRunFailure(error));
    entity.remove::<WorkflowRunResult>();
}
