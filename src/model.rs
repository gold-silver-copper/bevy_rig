use std::collections::HashMap;

use bevy_ecs::{hierarchy::ChildOf, prelude::*};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::provider::ProviderSpec;

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Model;

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    pub family: Option<String>,
}

impl ModelSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            family: None,
        }
    }

    pub fn with_family(mut self, family: impl Into<String>) -> Self {
        self.family = Some(family.into());
        self
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub completions: bool,
    pub embeddings: bool,
    pub tools: bool,
    pub streaming: bool,
    pub structured_output: bool,
    pub image_input: bool,
    pub audio_input: bool,
}

impl ModelCapabilities {
    pub fn chat_with_tools() -> Self {
        Self {
            completions: true,
            embeddings: false,
            tools: true,
            streaming: true,
            structured_output: true,
            image_input: true,
            audio_input: false,
        }
    }

    pub fn embeddings_only() -> Self {
        Self {
            completions: false,
            embeddings: true,
            tools: false,
            streaming: false,
            structured_output: false,
            image_input: false,
            audio_input: false,
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelContextWindow(pub usize);

#[derive(Bundle)]
pub struct ModelBundle {
    pub model: Model,
    pub spec: ModelSpec,
    pub capabilities: ModelCapabilities,
    pub context_window: ModelContextWindow,
    pub child_of: ChildOf,
}

impl ModelBundle {
    pub fn new(
        provider: Entity,
        spec: ModelSpec,
        capabilities: ModelCapabilities,
        context_window: usize,
    ) -> Self {
        Self {
            model: Model,
            spec,
            capabilities,
            context_window: ModelContextWindow(context_window),
            child_of: ChildOf(provider),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegisteredModel {
    pub entity: Entity,
    pub provider: Entity,
    pub name: String,
    pub qualified_name: String,
    pub capabilities: ModelCapabilities,
    pub context_window: usize,
}

#[derive(Resource, Default, Clone, Debug)]
pub struct ModelRegistry {
    by_entity: HashMap<Entity, RegisteredModel>,
    by_qualified_name: HashMap<String, Entity>,
    by_provider_name: HashMap<(Entity, String), Entity>,
    by_provider: HashMap<Entity, Vec<Entity>>,
}

impl ModelRegistry {
    pub fn get(&self, entity: Entity) -> Option<&RegisteredModel> {
        self.by_entity.get(&entity)
    }

    pub fn resolve_qualified(&self, qualified_name: &str) -> Option<Entity> {
        self.by_qualified_name.get(qualified_name).copied()
    }

    pub fn resolve_for_provider(&self, provider: Entity, model_name: &str) -> Option<Entity> {
        self.by_provider_name
            .get(&(provider, model_name.to_string()))
            .copied()
    }

    pub fn models_for_provider(&self, provider: Entity) -> Vec<Entity> {
        self.by_provider.get(&provider).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Error)]
pub enum ModelSpawnError {
    #[error("provider entity {0:?} does not exist or is missing ProviderSpec")]
    UnknownProvider(Entity),
    #[error("model {model:?} is already registered for provider {provider:?}")]
    DuplicateProviderModel { provider: Entity, model: String },
    #[error("qualified model name {0:?} is already registered")]
    DuplicateQualifiedName(String),
}

pub fn spawn_model(
    world: &mut World,
    provider: Entity,
    spec: ModelSpec,
    capabilities: ModelCapabilities,
    context_window: usize,
) -> Result<Entity, ModelSpawnError> {
    let provider_spec = world
        .get::<ProviderSpec>(provider)
        .cloned()
        .ok_or(ModelSpawnError::UnknownProvider(provider))?;

    {
        let registry = world.resource::<ModelRegistry>();
        if registry
            .resolve_for_provider(provider, &spec.name)
            .is_some()
        {
            return Err(ModelSpawnError::DuplicateProviderModel {
                provider,
                model: spec.name.clone(),
            });
        }
        let qualified_name = format!("{}/{}", provider_spec.label, spec.name);
        if registry.resolve_qualified(&qualified_name).is_some() {
            return Err(ModelSpawnError::DuplicateQualifiedName(qualified_name));
        }
    }

    let model = world
        .spawn(ModelBundle::new(
            provider,
            spec.clone(),
            capabilities.clone(),
            context_window,
        ))
        .id();

    let qualified_name = format!("{}/{}", provider_spec.label, spec.name);
    let mut registry = world.resource_mut::<ModelRegistry>();
    registry
        .by_provider
        .entry(provider)
        .or_default()
        .push(model);
    registry
        .by_provider_name
        .insert((provider, spec.name.clone()), model);
    registry
        .by_qualified_name
        .insert(qualified_name.clone(), model);
    registry.by_entity.insert(
        model,
        RegisteredModel {
            entity: model,
            provider,
            name: spec.name,
            qualified_name,
            capabilities,
            context_window,
        },
    );

    Ok(model)
}
