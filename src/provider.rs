use std::collections::HashMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Provider;

#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderHealth {
    #[default]
    Unknown,
    Reachable,
    Unreachable,
}

impl ProviderHealth {
    pub fn allows_requests(&self) -> bool {
        !matches!(self, Self::Unreachable)
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderAuthState {
    #[default]
    Unknown,
    NotRequired,
    Ready,
    Missing,
    Invalid,
}

impl ProviderAuthState {
    pub fn for_spec(spec: &ProviderSpec) -> Self {
        if spec.is_local {
            Self::NotRequired
        } else {
            Self::Unknown
        }
    }

    pub fn allows_requests(&self) -> bool {
        matches!(self, Self::Unknown | Self::NotRequired | Self::Ready)
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRevision(pub u64);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Anthropic,
    Azure,
    Cohere,
    DeepSeek,
    Galadriel,
    Gemini,
    Groq,
    HuggingFace,
    Hyperbolic,
    Llamafile,
    Mira,
    Mistral,
    Moonshot,
    Ollama,
    OpenAi,
    OpenRouter,
    Perplexity,
    Together,
    XAi,
}

#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSpec {
    pub kind: ProviderKind,
    pub label: String,
    pub endpoint: Option<String>,
    pub is_local: bool,
}

impl ProviderSpec {
    pub fn new(kind: ProviderKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.into(),
            endpoint: None,
            is_local: matches!(kind, ProviderKind::Llamafile | ProviderKind::Ollama),
        }
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }
}

#[derive(Component, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub completions: bool,
    pub embeddings: bool,
    pub tools: bool,
    pub streaming: bool,
    pub transcription: bool,
    pub image_generation: bool,
    pub audio_generation: bool,
}

impl ProviderCapabilities {
    pub fn text_tooling() -> Self {
        Self {
            completions: true,
            embeddings: true,
            tools: true,
            streaming: true,
            transcription: false,
            image_generation: false,
            audio_generation: false,
        }
    }
}

#[derive(Bundle)]
pub struct ProviderBundle {
    pub provider: Provider,
    pub spec: ProviderSpec,
    pub capabilities: ProviderCapabilities,
    pub health: ProviderHealth,
    pub auth_state: ProviderAuthState,
    pub revision: ProviderRevision,
}

impl ProviderBundle {
    pub fn new(spec: ProviderSpec, capabilities: ProviderCapabilities) -> Self {
        let auth_state = ProviderAuthState::for_spec(&spec);
        Self {
            provider: Provider,
            spec,
            capabilities,
            health: ProviderHealth::default(),
            auth_state,
            revision: ProviderRevision::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CatalogProvider {
    pub kind: ProviderKind,
    pub default_label: &'static str,
    pub capabilities: ProviderCapabilities,
}

#[derive(Resource, Clone, Debug)]
pub struct ProviderCatalog {
    by_kind: HashMap<ProviderKind, CatalogProvider>,
}

impl Default for ProviderCatalog {
    fn default() -> Self {
        let mut registry = Self {
            by_kind: HashMap::new(),
        };

        for (kind, label) in [
            (ProviderKind::Anthropic, "anthropic"),
            (ProviderKind::Azure, "azure-openai"),
            (ProviderKind::Cohere, "cohere"),
            (ProviderKind::DeepSeek, "deepseek"),
            (ProviderKind::Galadriel, "galadriel"),
            (ProviderKind::Gemini, "gemini"),
            (ProviderKind::Groq, "groq"),
            (ProviderKind::HuggingFace, "huggingface"),
            (ProviderKind::Hyperbolic, "hyperbolic"),
            (ProviderKind::Llamafile, "llamafile"),
            (ProviderKind::Mira, "mira"),
            (ProviderKind::Mistral, "mistral"),
            (ProviderKind::Moonshot, "moonshot"),
            (ProviderKind::Ollama, "ollama"),
            (ProviderKind::OpenAi, "openai"),
            (ProviderKind::OpenRouter, "openrouter"),
            (ProviderKind::Perplexity, "perplexity"),
            (ProviderKind::Together, "together"),
            (ProviderKind::XAi, "xai"),
        ] {
            registry.by_kind.insert(
                kind,
                CatalogProvider {
                    kind,
                    default_label: label,
                    capabilities: ProviderCapabilities::text_tooling(),
                },
            );
        }

        registry
    }
}

impl ProviderCatalog {
    pub fn get(&self, kind: ProviderKind) -> Option<&CatalogProvider> {
        self.by_kind.get(&kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = ProviderKind> + '_ {
        self.by_kind.keys().copied()
    }
}

pub fn spawn_provider(
    world: &mut World,
    spec: ProviderSpec,
    capabilities: ProviderCapabilities,
) -> Entity {
    world.spawn(ProviderBundle::new(spec, capabilities)).id()
}
