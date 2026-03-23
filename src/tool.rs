use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use bevy_ecs::{message::Messages, prelude::*};
use serde_json::Value;
use thiserror::Error;

static NEXT_TOOL_CALL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Tool;

#[derive(Component, Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, description: impl Into<String>, schema: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
        }
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolKind {
    #[default]
    Invocation,
}

#[derive(Bundle)]
pub struct ToolBundle {
    pub tool: Tool,
    pub spec: ToolSpec,
    pub kind: ToolKind,
}

impl ToolBundle {
    pub fn new(spec: ToolSpec) -> Self {
        Self {
            tool: Tool,
            spec,
            kind: ToolKind::Invocation,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub run: Entity,
    pub tool: Entity,
    pub call_id: String,
    pub args: Value,
}

impl ToolCall {
    pub fn new(run: Entity, tool: Entity, args: Value) -> Self {
        let nonce = NEXT_TOOL_CALL_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            run,
            tool,
            call_id: format!("run{}-tool{}-call{nonce}", run.index(), tool.index()),
            args,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    pub value: Value,
}

impl ToolOutput {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            value: Value::String(value.into()),
        }
    }

    pub fn json(value: Value) -> Self {
        Self { value }
    }

    pub fn as_text(&self) -> Option<&str> {
        self.value.as_str()
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ToolExecutionError {
    pub message: String,
}

impl ToolExecutionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub type ToolExecutionResult = Result<ToolOutput, ToolExecutionError>;

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolInvocation;

#[derive(Component, Clone, Debug, PartialEq)]
pub struct ToolInvocationCall(pub ToolCall);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolInvocationStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct ToolInvocationOutput(pub ToolOutput);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ToolInvocationError(pub String);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolInvocationPublished;

#[derive(Bundle)]
pub struct ToolInvocationBundle {
    pub invocation: ToolInvocation,
    pub call: ToolInvocationCall,
    pub status: ToolInvocationStatus,
}

impl ToolInvocationBundle {
    pub fn new(call: ToolCall) -> Self {
        Self {
            invocation: ToolInvocation,
            call: ToolInvocationCall(call),
            status: ToolInvocationStatus::Queued,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegisteredTool {
    pub entity: Entity,
    pub name: String,
    pub kind: ToolKind,
}

#[derive(Resource, Default)]
pub struct ToolRegistry {
    by_entity: HashMap<Entity, RegisteredTool>,
    by_name: HashMap<String, Entity>,
}

impl ToolRegistry {
    pub fn get(&self, entity: Entity) -> Option<&RegisteredTool> {
        self.by_entity.get(&entity)
    }

    pub fn get_by_name(&self, name: &str) -> Option<Entity> {
        self.by_name.get(name).copied()
    }
}

#[derive(Debug, Error)]
pub enum ToolRegistrationError {
    #[error("tool entity {0:?} is missing ToolSpec")]
    MissingSpec(Entity),
    #[error("tool name {0:?} is already registered to another entity")]
    DuplicateName(String),
}

#[derive(Message, Clone, Debug)]
pub struct ToolCallRequested {
    pub call: ToolCall,
}

#[derive(Message, Clone, Debug)]
pub struct ToolCallCompleted {
    pub call: ToolCall,
    pub output: ToolOutput,
}

#[derive(Message, Clone, Debug)]
pub struct ToolCallFailed {
    pub call: ToolCall,
    pub error: String,
}

pub fn register_tool(world: &mut World, tool: Entity) -> Result<(), ToolRegistrationError> {
    register_tool_metadata(world, tool)
}

pub fn rebuild_tool_registry(world: &mut World) {
    let mut tools = {
        let mut query = world.query::<(Entity, &ToolSpec, Option<&ToolKind>)>();
        query
            .iter(world)
            .map(|(entity, spec, kind)| {
                (
                    entity,
                    spec.name.clone(),
                    kind.copied().unwrap_or(ToolKind::Invocation),
                )
            })
            .collect::<Vec<_>>()
    };
    tools.sort_by_key(|(entity, name, _)| (name.clone(), entity.index()));

    let mut by_entity = HashMap::new();
    let mut by_name = HashMap::new();

    for (entity, name, kind) in tools {
        if by_name.contains_key(&name) {
            continue;
        }

        by_name.insert(name.clone(), entity);
        by_entity.insert(entity, RegisteredTool { entity, name, kind });
    }

    *world.resource_mut::<ToolRegistry>() = ToolRegistry { by_entity, by_name };
}

pub fn queue_requested_tool_calls(world: &mut World) {
    let calls: Vec<ToolCall> = {
        let mut messages = world.resource_mut::<Messages<ToolCallRequested>>();
        messages.drain().map(|message| message.call).collect()
    };

    for call in calls {
        let registered = {
            let registry = world.resource::<ToolRegistry>();
            registry.get(call.tool).cloned()
        };

        if registered.is_none() {
            world.write_message(ToolCallFailed {
                call: call.clone(),
                error: format!("tool entity {:?} is not registered", call.tool),
            });
            continue;
        }

        world.spawn(ToolInvocationBundle::new(call));
    }
}

pub fn mark_tool_invocation_running(commands: &mut Commands, invocation: Entity) {
    commands
        .entity(invocation)
        .insert(ToolInvocationStatus::Running)
        .remove::<ToolInvocationOutput>()
        .remove::<ToolInvocationError>()
        .remove::<ToolInvocationPublished>();
}

pub fn complete_tool_invocation(commands: &mut Commands, invocation: Entity, output: ToolOutput) {
    commands
        .entity(invocation)
        .insert((
            ToolInvocationStatus::Completed,
            ToolInvocationOutput(output),
        ))
        .remove::<ToolInvocationError>()
        .remove::<ToolInvocationPublished>();
}

pub fn fail_tool_invocation(commands: &mut Commands, invocation: Entity, error: impl Into<String>) {
    commands
        .entity(invocation)
        .insert((
            ToolInvocationStatus::Failed,
            ToolInvocationError(error.into()),
        ))
        .remove::<ToolInvocationOutput>()
        .remove::<ToolInvocationPublished>();
}

pub fn publish_tool_invocation_results(world: &mut World) {
    let ready = {
        let mut query = world.query::<(
            Entity,
            &ToolInvocationCall,
            &ToolInvocationStatus,
            Option<&ToolInvocationOutput>,
            Option<&ToolInvocationError>,
            Option<&ToolInvocationPublished>,
        )>();

        query
            .iter(world)
            .filter_map(|(entity, call, status, output, error, published)| {
                if published.is_some() {
                    return None;
                }

                match status {
                    ToolInvocationStatus::Completed => {
                        output.map(|output| (entity, call.0.clone(), Some(output.0.clone()), None))
                    }
                    ToolInvocationStatus::Failed => {
                        error.map(|error| (entity, call.0.clone(), None, Some(error.0.clone())))
                    }
                    ToolInvocationStatus::Queued | ToolInvocationStatus::Running => None,
                }
            })
            .collect::<Vec<_>>()
    };

    for (invocation, call, output, error) in ready {
        if let Some(output) = output {
            world.write_message(ToolCallCompleted { call, output });
        } else if let Some(error) = error {
            world.write_message(ToolCallFailed { call, error });
        }

        world.entity_mut(invocation).insert(ToolInvocationPublished);
    }
}

fn register_tool_metadata(world: &mut World, tool: Entity) -> Result<(), ToolRegistrationError> {
    let spec = world
        .get::<ToolSpec>(tool)
        .cloned()
        .ok_or(ToolRegistrationError::MissingSpec(tool))?;
    let kind = world.get::<ToolKind>(tool).copied().unwrap_or_default();

    let registry = world.resource_mut::<ToolRegistry>();
    if let Some(existing) = registry.get_by_name(&spec.name)
        && existing != tool
    {
        return Err(ToolRegistrationError::DuplicateName(spec.name));
    }
    drop(registry);

    let mut registry = world.resource_mut::<ToolRegistry>();
    registry.by_name.insert(spec.name.clone(), tool);
    registry.by_entity.insert(
        tool,
        RegisteredTool {
            entity: tool,
            name: spec.name,
            kind,
        },
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queued_invocations_publish_completion_messages() {
        let mut world = World::new();
        world.init_resource::<ToolRegistry>();
        world.init_resource::<Messages<ToolCallRequested>>();
        world.init_resource::<Messages<ToolCallCompleted>>();
        world.init_resource::<Messages<ToolCallFailed>>();

        let run = world.spawn_empty().id();
        let tool = world
            .spawn(ToolBundle::new(ToolSpec::new(
                "echo",
                "Echoes text",
                json!({"type":"object"}),
            )))
            .id();

        rebuild_tool_registry(&mut world);

        let call = ToolCall::new(run, tool, json!({"text":"hello"}));
        world.write_message(ToolCallRequested { call: call.clone() });
        queue_requested_tool_calls(&mut world);

        let invocation = {
            let mut query = world.query_filtered::<Entity, With<ToolInvocation>>();
            query
                .iter(&world)
                .next()
                .expect("tool invocation should be spawned")
        };

        world.entity_mut(invocation).insert((
            ToolInvocationStatus::Completed,
            ToolInvocationOutput(ToolOutput::text("done")),
        ));

        publish_tool_invocation_results(&mut world);

        let completed = {
            let mut messages = world.resource_mut::<Messages<ToolCallCompleted>>();
            messages.drain().collect::<Vec<_>>()
        };
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].call.call_id, call.call_id);
        assert_eq!(completed[0].output.as_text(), Some("done"));
    }
}
