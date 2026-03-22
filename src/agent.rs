use bevy_ecs::{hierarchy::ChildOf, prelude::*};
use thiserror::Error;

use crate::{model::ModelSpec, session};

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct Agent;

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct AgentSpec {
    pub name: String,
    pub model: String,
    pub max_turns: Option<usize>,
    pub provider: Option<Entity>,
}

impl AgentSpec {
    pub fn new(name: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            model: model.into(),
            max_turns: None,
            provider: None,
        }
    }

    pub fn with_provider(mut self, provider: Entity) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = Some(max_turns);
        self
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentToolRefs(pub Vec<Entity>);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentContextRefs(pub Vec<Entity>);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrimarySession(pub Entity);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentModelRef(pub Entity);

#[derive(Bundle)]
pub struct AgentBundle {
    pub agent: Agent,
    pub spec: AgentSpec,
    pub tools: AgentToolRefs,
    pub contexts: AgentContextRefs,
    pub primary_session: PrimarySession,
}

impl AgentBundle {
    pub fn new(spec: AgentSpec, primary_session: Entity) -> Self {
        Self {
            agent: Agent,
            spec,
            tools: AgentToolRefs::default(),
            contexts: AgentContextRefs::default(),
            primary_session: PrimarySession(primary_session),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentHandles {
    pub agent: Entity,
    pub session: Entity,
}

#[derive(Debug, Error)]
pub enum AgentModelError {
    #[error("agent entity {0:?} does not exist")]
    UnknownAgent(Entity),
    #[error("model entity {0:?} does not exist")]
    UnknownModel(Entity),
    #[error("model entity {0:?} is missing ModelSpec")]
    MissingModelSpec(Entity),
}

#[derive(Debug, Error)]
pub enum AgentLinkError {
    #[error("agent entity {0:?} does not exist")]
    UnknownAgent(Entity),
    #[error("linked entity {0:?} does not exist")]
    UnknownLinkedEntity(Entity),
}

pub fn spawn_agent(world: &mut World, spec: AgentSpec) -> AgentHandles {
    let session = session::spawn_session(world, format!("{} session", spec.name));
    let agent = world.spawn(AgentBundle::new(spec, session)).id();

    AgentHandles { agent, session }
}

pub fn spawn_agent_from_model(
    world: &mut World,
    name: impl Into<String>,
    model: Entity,
) -> Result<AgentHandles, AgentModelError> {
    let model_spec = world.get::<ModelSpec>(model).cloned().ok_or_else(|| {
        if world.get_entity(model).is_err() {
            AgentModelError::UnknownModel(model)
        } else {
            AgentModelError::MissingModelSpec(model)
        }
    })?;
    let provider = world.get::<ChildOf>(model).map(ChildOf::parent);

    let mut spec = AgentSpec::new(name, model_spec.name);
    if let Some(provider) = provider {
        spec = spec.with_provider(provider);
    }

    let handles = spawn_agent(world, spec);
    world.entity_mut(handles.agent).insert(AgentModelRef(model));
    Ok(handles)
}

pub fn bind_model(world: &mut World, agent: Entity, model: Entity) -> Result<(), AgentModelError> {
    if world.get_entity(agent).is_err() {
        return Err(AgentModelError::UnknownAgent(agent));
    }

    let model_spec = world.get::<ModelSpec>(model).cloned().ok_or_else(|| {
        if world.get_entity(model).is_err() {
            AgentModelError::UnknownModel(model)
        } else {
            AgentModelError::MissingModelSpec(model)
        }
    })?;
    let provider = world.get::<ChildOf>(model).map(ChildOf::parent);

    let mut entity = world.entity_mut(agent);
    {
        let mut spec = entity
            .get_mut::<AgentSpec>()
            .expect("agent bundles always include AgentSpec");
        spec.model = model_spec.name;
        spec.provider = provider;
    }
    entity.insert(AgentModelRef(model));

    Ok(())
}

pub fn attach_tool(world: &mut World, agent: Entity, tool: Entity) -> Result<(), AgentLinkError> {
    ensure_entities_exist(world, agent, tool)?;

    let mut agent_entity = world.entity_mut(agent);
    let mut refs = agent_entity
        .get_mut::<AgentToolRefs>()
        .expect("agent bundles always include tool references");
    if !refs.0.contains(&tool) {
        refs.0.push(tool);
    }

    Ok(())
}

pub fn attach_context(
    world: &mut World,
    agent: Entity,
    context: Entity,
) -> Result<(), AgentLinkError> {
    ensure_entities_exist(world, agent, context)?;

    let mut agent_entity = world.entity_mut(agent);
    let mut refs = agent_entity
        .get_mut::<AgentContextRefs>()
        .expect("agent bundles always include context references");
    if !refs.0.contains(&context) {
        refs.0.push(context);
    }

    Ok(())
}

fn ensure_entities_exist(
    world: &mut World,
    agent: Entity,
    entity: Entity,
) -> Result<(), AgentLinkError> {
    if world.get_entity(agent).is_err() {
        return Err(AgentLinkError::UnknownAgent(agent));
    }

    if world.get_entity(entity).is_err() {
        return Err(AgentLinkError::UnknownLinkedEntity(entity));
    }

    Ok(())
}
