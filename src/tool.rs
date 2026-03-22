use std::collections::HashMap;

use bevy_ecs::{
    message::Messages,
    prelude::*,
    system::{In, IntoSystem, SystemId},
};
use serde_json::Value;
use thiserror::Error;

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
    System,
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
            kind: ToolKind::System,
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
        Self {
            run,
            tool,
            call_id: format!("run{}-tool{}", run.index(), tool.index()),
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
pub type ToolSystemId = SystemId<In<ToolCall>, ToolExecutionResult>;

#[derive(Clone, Debug)]
pub struct RegisteredTool {
    pub entity: Entity,
    pub name: String,
    pub system: ToolSystemId,
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

#[derive(Debug, Error)]
pub enum ToolDispatchError {
    #[error("tool entity {0:?} is not registered")]
    UnregisteredTool(Entity),
    #[error("tool system failed to run: {0}")]
    Invocation(String),
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

pub fn register_tool_system<M, S>(
    world: &mut World,
    tool: Entity,
    system: S,
) -> Result<ToolSystemId, ToolRegistrationError>
where
    M: 'static,
    S: IntoSystem<In<ToolCall>, ToolExecutionResult, M> + 'static,
{
    let spec = world
        .get::<ToolSpec>(tool)
        .cloned()
        .ok_or(ToolRegistrationError::MissingSpec(tool))?;

    let registry = world.resource_mut::<ToolRegistry>();
    if let Some(existing) = registry.get_by_name(&spec.name) {
        if existing != tool {
            return Err(ToolRegistrationError::DuplicateName(spec.name));
        }
    }
    drop(registry);

    let system_id = world.register_system(system);
    let mut registry = world.resource_mut::<ToolRegistry>();
    registry.by_name.insert(spec.name.clone(), tool);
    registry.by_entity.insert(
        tool,
        RegisteredTool {
            entity: tool,
            name: spec.name,
            system: system_id,
        },
    );

    Ok(system_id)
}

pub fn dispatch_tool(
    world: &mut World,
    call: ToolCall,
) -> Result<ToolExecutionResult, ToolDispatchError> {
    let system_id = {
        let registry = world.resource::<ToolRegistry>();
        let Some(registered) = registry.get(call.tool) else {
            return Err(ToolDispatchError::UnregisteredTool(call.tool));
        };
        registered.system
    };

    world
        .run_system_with(system_id, call)
        .map_err(|error| ToolDispatchError::Invocation(error.to_string()))
}

pub fn dispatch_requested_tool_calls(world: &mut World) {
    let calls: Vec<ToolCall> = {
        let mut messages = world.resource_mut::<Messages<ToolCallRequested>>();
        messages.drain().map(|message| message.call).collect()
    };

    for call in calls {
        match dispatch_tool(world, call.clone()) {
            Ok(Ok(output)) => {
                world.write_message(ToolCallCompleted { call, output });
            }
            Ok(Err(error)) => {
                world.write_message(ToolCallFailed {
                    call,
                    error: error.message,
                });
            }
            Err(error) => {
                world.write_message(ToolCallFailed {
                    call,
                    error: error.to_string(),
                });
            }
        }
    }
}
