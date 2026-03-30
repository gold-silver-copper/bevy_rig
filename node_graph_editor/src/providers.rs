use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub type ProviderId = String;

const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";
const DEFAULT_LLAMAFILE_ENDPOINT: &str = "http://localhost:8080";
const DEFAULT_AZURE_API_VERSION: &str = "2024-10-21";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Anthropic,
    Azure,
    Cohere,
    Deepseek,
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
    Xai,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 19] = [
        ProviderKind::Anthropic,
        ProviderKind::Azure,
        ProviderKind::Cohere,
        ProviderKind::Deepseek,
        ProviderKind::Galadriel,
        ProviderKind::Gemini,
        ProviderKind::Groq,
        ProviderKind::HuggingFace,
        ProviderKind::Hyperbolic,
        ProviderKind::Llamafile,
        ProviderKind::Mira,
        ProviderKind::Mistral,
        ProviderKind::Moonshot,
        ProviderKind::Ollama,
        ProviderKind::OpenAi,
        ProviderKind::OpenRouter,
        ProviderKind::Perplexity,
        ProviderKind::Together,
        ProviderKind::Xai,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::Azure => "Azure OpenAI",
            Self::Cohere => "Cohere",
            Self::Deepseek => "DeepSeek",
            Self::Galadriel => "Galadriel",
            Self::Gemini => "Gemini",
            Self::Groq => "Groq",
            Self::HuggingFace => "Hugging Face",
            Self::Hyperbolic => "Hyperbolic",
            Self::Llamafile => "Llamafile",
            Self::Mira => "Mira",
            Self::Mistral => "Mistral",
            Self::Moonshot => "Moonshot",
            Self::Ollama => "Ollama",
            Self::OpenAi => "OpenAI",
            Self::OpenRouter => "OpenRouter",
            Self::Perplexity => "Perplexity",
            Self::Together => "Together",
            Self::Xai => "xAI",
        }
    }

    pub fn shifted(self, delta: i32) -> Self {
        shift_in_slice(self, &Self::ALL, delta)
    }

    pub const fn default_variant(self) -> ProviderVariant {
        match self {
            Self::OpenAi => ProviderVariant::OpenAi(OpenAiVariant::ResponsesApi),
            Self::Gemini => ProviderVariant::Gemini(GeminiVariant::GenerateContent),
            _ => ProviderVariant::Standard,
        }
    }

    pub fn supported_variants(self) -> &'static [ProviderVariant] {
        match self {
            Self::OpenAi => &OPENAI_VARIANTS,
            Self::Gemini => &GEMINI_VARIANTS,
            _ => &STANDARD_VARIANTS,
        }
    }

    pub const fn supports_model_discovery(self) -> bool {
        matches!(
            self,
            Self::Anthropic
                | Self::Cohere
                | Self::Gemini
                | Self::Groq
                | Self::Hyperbolic
                | Self::Llamafile
                | Self::Mira
                | Self::Mistral
                | Self::Moonshot
                | Self::Ollama
                | Self::OpenAi
                | Self::OpenRouter
                | Self::Together
        )
    }

    pub fn default_name(self) -> String {
        match self {
            Self::Ollama => "Local Ollama".into(),
            Self::Llamafile => "Local Llamafile".into(),
            _ => format!("{} Provider", self.label()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OpenAiVariant {
    ResponsesApi,
    CompletionsApi,
}

impl OpenAiVariant {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ResponsesApi => "Responses API",
            Self::CompletionsApi => "Completions API",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeminiVariant {
    GenerateContent,
    Interactions,
}

impl GeminiVariant {
    pub const fn label(self) -> &'static str {
        match self {
            Self::GenerateContent => "Generate Content",
            Self::Interactions => "Interactions",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderVariant {
    Standard,
    OpenAi(OpenAiVariant),
    Gemini(GeminiVariant),
}

impl ProviderVariant {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::OpenAi(variant) => variant.label(),
            Self::Gemini(variant) => variant.label(),
        }
    }

    pub fn shifted_for_kind(self, kind: ProviderKind, delta: i32) -> Self {
        let supported = kind.supported_variants();
        if supported.len() <= 1 {
            return kind.default_variant();
        }
        shift_in_slice(self, supported, delta)
    }

    pub fn normalize_for_kind(self, kind: ProviderKind) -> Self {
        if kind.supported_variants().contains(&self) {
            self
        } else {
            kind.default_variant()
        }
    }
}

const STANDARD_VARIANTS: [ProviderVariant; 1] = [ProviderVariant::Standard];
const OPENAI_VARIANTS: [ProviderVariant; 2] = [
    ProviderVariant::OpenAi(OpenAiVariant::ResponsesApi),
    ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi),
];
const GEMINI_VARIANTS: [ProviderVariant; 2] = [
    ProviderVariant::Gemini(GeminiVariant::GenerateContent),
    ProviderVariant::Gemini(GeminiVariant::Interactions),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AzureAuthKind {
    ApiKey,
    Token,
}

impl AzureAuthKind {
    pub const ALL: [AzureAuthKind; 2] = [Self::ApiKey, Self::Token];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ApiKey => "API Key",
            Self::Token => "Bearer Token",
        }
    }

    pub fn shifted(self, delta: i32) -> Self {
        shift_in_slice(self, &Self::ALL, delta)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HuggingFaceSubprovider {
    HfInference,
    Together,
    SambaNova,
    Fireworks,
    Hyperbolic,
    Nebius,
    Novita,
}

impl HuggingFaceSubprovider {
    pub const ALL: [HuggingFaceSubprovider; 7] = [
        Self::HfInference,
        Self::Together,
        Self::SambaNova,
        Self::Fireworks,
        Self::Hyperbolic,
        Self::Nebius,
        Self::Novita,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::HfInference => "HF Inference",
            Self::Together => "Together",
            Self::SambaNova => "SambaNova",
            Self::Fireworks => "Fireworks",
            Self::Hyperbolic => "Hyperbolic",
            Self::Nebius => "Nebius",
            Self::Novita => "Novita",
        }
    }

    pub fn shifted(self, delta: i32) -> Self {
        shift_in_slice(self, &Self::ALL, delta)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ApiKeyProviderConfig {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureProviderConfig {
    pub endpoint: String,
    pub auth_kind: AzureAuthKind,
    pub credential: String,
    pub api_version: String,
}

impl Default for AzureProviderConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            auth_kind: AzureAuthKind::ApiKey,
            credential: String::new(),
            api_version: DEFAULT_AZURE_API_VERSION.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EndpointProviderConfig {
    pub base_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GaladrielProviderConfig {
    pub api_key: String,
    pub fine_tune_api_key: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HuggingFaceProviderConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    pub subprovider: HuggingFaceSubprovider,
}

impl Default for HuggingFaceProviderConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            subprovider: HuggingFaceSubprovider::HfInference,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderConfig {
    Anthropic(ApiKeyProviderConfig),
    Azure(AzureProviderConfig),
    Cohere(ApiKeyProviderConfig),
    Deepseek(ApiKeyProviderConfig),
    Galadriel(GaladrielProviderConfig),
    Gemini(ApiKeyProviderConfig),
    Groq(ApiKeyProviderConfig),
    HuggingFace(HuggingFaceProviderConfig),
    Hyperbolic(ApiKeyProviderConfig),
    Llamafile(EndpointProviderConfig),
    Mira(ApiKeyProviderConfig),
    Mistral(ApiKeyProviderConfig),
    Moonshot(ApiKeyProviderConfig),
    Ollama(EndpointProviderConfig),
    OpenAi(ApiKeyProviderConfig),
    OpenRouter(ApiKeyProviderConfig),
    Perplexity(ApiKeyProviderConfig),
    Together(ApiKeyProviderConfig),
    Xai(ApiKeyProviderConfig),
}

impl ProviderConfig {
    pub fn for_kind(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Anthropic => Self::Anthropic(ApiKeyProviderConfig::default()),
            ProviderKind::Azure => Self::Azure(AzureProviderConfig::default()),
            ProviderKind::Cohere => Self::Cohere(ApiKeyProviderConfig::default()),
            ProviderKind::Deepseek => Self::Deepseek(ApiKeyProviderConfig::default()),
            ProviderKind::Galadriel => Self::Galadriel(GaladrielProviderConfig::default()),
            ProviderKind::Gemini => Self::Gemini(ApiKeyProviderConfig::default()),
            ProviderKind::Groq => Self::Groq(ApiKeyProviderConfig::default()),
            ProviderKind::HuggingFace => Self::HuggingFace(HuggingFaceProviderConfig::default()),
            ProviderKind::Hyperbolic => Self::Hyperbolic(ApiKeyProviderConfig::default()),
            ProviderKind::Llamafile => Self::Llamafile(EndpointProviderConfig {
                base_url: DEFAULT_LLAMAFILE_ENDPOINT.into(),
            }),
            ProviderKind::Mira => Self::Mira(ApiKeyProviderConfig::default()),
            ProviderKind::Mistral => Self::Mistral(ApiKeyProviderConfig::default()),
            ProviderKind::Moonshot => Self::Moonshot(ApiKeyProviderConfig::default()),
            ProviderKind::Ollama => Self::Ollama(EndpointProviderConfig {
                base_url: DEFAULT_OLLAMA_ENDPOINT.into(),
            }),
            ProviderKind::OpenAi => Self::OpenAi(ApiKeyProviderConfig::default()),
            ProviderKind::OpenRouter => Self::OpenRouter(ApiKeyProviderConfig::default()),
            ProviderKind::Perplexity => Self::Perplexity(ApiKeyProviderConfig::default()),
            ProviderKind::Together => Self::Together(ApiKeyProviderConfig::default()),
            ProviderKind::Xai => Self::Xai(ApiKeyProviderConfig::default()),
        }
    }

    pub fn config_error(&self) -> Option<String> {
        match self {
            Self::Anthropic(config)
            | Self::Cohere(config)
            | Self::Deepseek(config)
            | Self::Gemini(config)
            | Self::Groq(config)
            | Self::Hyperbolic(config)
            | Self::Mira(config)
            | Self::Mistral(config)
            | Self::Moonshot(config)
            | Self::OpenAi(config)
            | Self::OpenRouter(config)
            | Self::Perplexity(config)
            | Self::Together(config)
            | Self::Xai(config) => require_api_key(config.api_key.as_str()),
            Self::Galadriel(config) => require_api_key(config.api_key.as_str()),
            Self::HuggingFace(config) => require_api_key(config.api_key.as_str()),
            Self::Azure(config) => {
                if config.endpoint.trim().is_empty() {
                    Some("Azure endpoint is required".into())
                } else if config.credential.trim().is_empty() {
                    Some(format!("Azure {} is required", config.auth_kind.label()))
                } else {
                    None
                }
            }
            Self::Llamafile(config) | Self::Ollama(config) => {
                if config.base_url.trim().is_empty() {
                    Some("Endpoint is required".into())
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderStatusKind {
    Pending,
    Ready,
    Error,
    NeedsConfig,
}

impl ProviderStatusKind {
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Error => "error",
            Self::NeedsConfig => "needs config",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub kind: ProviderStatusKind,
    pub detail: String,
}

impl ProviderStatus {
    pub fn pending(detail: impl Into<String>) -> Self {
        Self {
            kind: ProviderStatusKind::Pending,
            detail: detail.into(),
        }
    }

    pub fn ready(detail: impl Into<String>) -> Self {
        Self {
            kind: ProviderStatusKind::Ready,
            detail: detail.into(),
        }
    }

    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            kind: ProviderStatusKind::Error,
            detail: detail.into(),
        }
    }

    pub fn needs_config(detail: impl Into<String>) -> Self {
        Self {
            kind: ProviderStatusKind::NeedsConfig,
            detail: detail.into(),
        }
    }
}

impl Default for ProviderStatus {
    fn default() -> Self {
        Self::pending("refresh pending")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRegistration {
    pub id: ProviderId,
    pub name: String,
    pub kind: ProviderKind,
    pub variant: ProviderVariant,
    pub config: ProviderConfig,
    #[serde(default)]
    pub cached_models: Vec<String>,
    #[serde(default)]
    pub status: ProviderStatus,
}

impl ProviderRegistration {
    pub fn new(id: ProviderId, kind: ProviderKind) -> Self {
        let config = ProviderConfig::for_kind(kind);
        let status = config
            .config_error()
            .map(ProviderStatus::needs_config)
            .unwrap_or_else(|| ProviderStatus::pending("refresh pending"));

        Self {
            id,
            name: kind.default_name(),
            kind,
            variant: kind.default_variant(),
            config,
            cached_models: Vec::new(),
            status,
        }
    }

    pub fn display_name(&self) -> &str {
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            self.kind.label()
        } else {
            trimmed
        }
    }

    pub fn family_label(&self) -> String {
        if self.kind.supported_variants().len() > 1 {
            format!("{} / {}", self.kind.label(), self.variant.label())
        } else {
            self.kind.label().into()
        }
    }

    pub fn supports_model_discovery(&self) -> bool {
        self.kind.supports_model_discovery()
    }

    pub fn manual_model_only(&self) -> bool {
        !self.supports_model_discovery()
    }

    pub fn config_error(&self) -> Option<String> {
        self.config.config_error()
    }

    pub fn invalidate_runtime_state(&mut self) {
        self.cached_models.clear();
        self.status = self
            .config_error()
            .map(ProviderStatus::needs_config)
            .unwrap_or_else(|| ProviderStatus::pending("refresh pending"));
    }

    pub fn set_kind(&mut self, kind: ProviderKind) {
        let previous_default = self.kind.default_name();
        self.kind = kind;
        self.variant = kind.default_variant();
        self.config = ProviderConfig::for_kind(kind);
        if self.name == previous_default {
            self.name = kind.default_name();
        }
        self.invalidate_runtime_state();
    }

    pub fn cycle_variant(&mut self, delta: i32) {
        self.variant = self.variant.shifted_for_kind(self.kind, delta);
        self.invalidate_runtime_state();
    }

    pub fn normalized(mut self) -> Self {
        self.variant = self.variant.normalize_for_kind(self.kind);
        if !config_matches_kind(self.kind, &self.config) {
            self.config = ProviderConfig::for_kind(self.kind);
        }
        if self.name.trim().is_empty() {
            self.name = self.kind.default_name();
        }
        if let Some(error) = self.config_error() {
            self.status = ProviderStatus::needs_config(error);
        } else if self.status.detail.trim().is_empty() {
            self.status = ProviderStatus::pending("refresh pending");
        }
        self.cached_models = dedupe_models(self.cached_models);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRefreshResult {
    pub status: ProviderStatus,
    pub cached_models: Vec<String>,
}

impl ProviderRefreshResult {
    pub fn new(status: ProviderStatus, cached_models: Vec<String>) -> Self {
        Self {
            status,
            cached_models: dedupe_models(cached_models),
        }
    }
}

#[derive(Resource, Clone, Debug)]
pub struct ProviderRegistry {
    providers: BTreeMap<ProviderId, ProviderRegistration>,
    order: Vec<ProviderId>,
    next_id: u64,
    #[allow(dead_code)]
    storage_path: PathBuf,
    pub revision: u64,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        let path = default_storage_path();
        Self::load_or_seed(path)
    }
}

impl ProviderRegistry {
    pub fn load_or_seed(path: PathBuf) -> Self {
        match Self::load_from_path(&path) {
            Ok(Some(registry)) => registry,
            Ok(None) => Self::with_default_ollama(path),
            Err(_) => Self::with_default_ollama(path),
        }
    }

    pub fn load_from_path(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }

        let payload = fs::read_to_string(path)
            .with_context(|| format!("failed to read provider registry at {}", path.display()))?;
        let persisted: ProviderRegistryFile = serde_json::from_str(&payload)
            .with_context(|| format!("failed to parse provider registry at {}", path.display()))?;
        Ok(Some(Self::from_persisted(path.to_path_buf(), persisted)))
    }

    pub fn save_to_disk(&self) -> Result<()> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create provider registry directory {}",
                    parent.display()
                )
            })?;
        }

        let persisted = ProviderRegistryFile::from_registry(self);
        let json = serde_json::to_string_pretty(&persisted)
            .context("failed to serialize provider registry")?;
        fs::write(&self.storage_path, json).with_context(|| {
            format!(
                "failed to write provider registry to {}",
                self.storage_path.display()
            )
        })?;
        Ok(())
    }

    pub fn ordered_registrations(&self) -> impl Iterator<Item = &ProviderRegistration> {
        self.order.iter().filter_map(|id| self.providers.get(id))
    }

    pub fn provider(&self, provider_id: &str) -> Option<&ProviderRegistration> {
        self.providers.get(provider_id)
    }

    pub fn provider_mut(&mut self, provider_id: &str) -> Option<&mut ProviderRegistration> {
        self.providers.get_mut(provider_id)
    }

    pub fn first_provider_id(&self) -> Option<ProviderId> {
        self.order.first().cloned()
    }

    pub fn cycle_provider_id(&self, current: Option<&str>, delta: i32) -> Option<ProviderId> {
        if self.order.is_empty() {
            return None;
        }

        let current_index = current
            .and_then(|id| self.order.iter().position(|candidate| candidate == id))
            .unwrap_or(0);
        let next = ((current_index as i32 + delta).rem_euclid(self.order.len() as i32)) as usize;
        self.order.get(next).cloned()
    }

    pub fn add_provider(&mut self, kind: ProviderKind) -> ProviderId {
        let id = format!("provider-{}", self.next_id);
        self.next_id += 1;
        let provider = ProviderRegistration::new(id.clone(), kind);
        self.order.push(id.clone());
        self.providers.insert(id.clone(), provider);
        self.touch();
        id
    }

    pub fn remove_provider(&mut self, provider_id: &str) -> bool {
        if self.providers.remove(provider_id).is_none() {
            return false;
        }
        self.order.retain(|id| id != provider_id);
        self.touch();
        true
    }

    pub fn apply_refresh_result(
        &mut self,
        provider_id: &str,
        result: ProviderRefreshResult,
    ) -> bool {
        let Some(provider) = self.providers.get_mut(provider_id) else {
            return false;
        };

        let changed =
            provider.status != result.status || provider.cached_models != result.cached_models;
        if changed {
            provider.status = result.status;
            provider.cached_models = result.cached_models;
            self.touch();
        }
        changed
    }

    pub fn mark_refreshing(&mut self, provider_id: &str) -> bool {
        let Some(provider) = self.providers.get_mut(provider_id) else {
            return false;
        };
        let next = provider
            .config_error()
            .map(ProviderStatus::needs_config)
            .unwrap_or_else(|| ProviderStatus::pending("refreshing provider"));
        if provider.status == next {
            return false;
        }
        provider.status = next;
        self.touch();
        true
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn with_default_ollama(path: PathBuf) -> Self {
        let provider = ProviderRegistration::new("provider-1".into(), ProviderKind::Ollama);
        let order = vec![provider.id.clone()];
        let mut providers = BTreeMap::new();
        providers.insert(provider.id.clone(), provider);
        Self {
            providers,
            order,
            next_id: 2,
            storage_path: path,
            revision: 1,
        }
    }

    fn from_persisted(path: PathBuf, persisted: ProviderRegistryFile) -> Self {
        let mut providers = BTreeMap::new();
        for provider in persisted.providers {
            providers.insert(provider.id.clone(), provider.normalized());
        }

        let mut order = persisted
            .order
            .into_iter()
            .filter(|id| providers.contains_key(id))
            .collect::<Vec<_>>();

        for id in providers.keys() {
            if !order.contains(id) {
                order.push(id.clone());
            }
        }

        Self {
            providers,
            order,
            next_id: persisted.next_id.max(1),
            storage_path: path,
            revision: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProviderRegistryFile {
    next_id: u64,
    order: Vec<ProviderId>,
    providers: Vec<ProviderRegistration>,
}

impl ProviderRegistryFile {
    fn from_registry(registry: &ProviderRegistry) -> Self {
        Self {
            next_id: registry.next_id,
            order: registry.order.clone(),
            providers: registry.ordered_registrations().cloned().collect(),
        }
    }
}

fn default_storage_path() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("node_graph_editor/Cargo.toml").exists() {
        cwd.join("node_graph_editor/.state/providers.json")
    } else if cwd.join("Cargo.toml").exists()
        && cwd
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value == "node_graph_editor")
    {
        cwd.join(".state/providers.json")
    } else {
        cwd.join(".node_graph_editor/providers.json")
    }
}

fn require_api_key(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        Some("API key is required".into())
    } else {
        None
    }
}

fn config_matches_kind(kind: ProviderKind, config: &ProviderConfig) -> bool {
    matches!(
        (kind, config),
        (ProviderKind::Anthropic, ProviderConfig::Anthropic(_))
            | (ProviderKind::Azure, ProviderConfig::Azure(_))
            | (ProviderKind::Cohere, ProviderConfig::Cohere(_))
            | (ProviderKind::Deepseek, ProviderConfig::Deepseek(_))
            | (ProviderKind::Galadriel, ProviderConfig::Galadriel(_))
            | (ProviderKind::Gemini, ProviderConfig::Gemini(_))
            | (ProviderKind::Groq, ProviderConfig::Groq(_))
            | (ProviderKind::HuggingFace, ProviderConfig::HuggingFace(_))
            | (ProviderKind::Hyperbolic, ProviderConfig::Hyperbolic(_))
            | (ProviderKind::Llamafile, ProviderConfig::Llamafile(_))
            | (ProviderKind::Mira, ProviderConfig::Mira(_))
            | (ProviderKind::Mistral, ProviderConfig::Mistral(_))
            | (ProviderKind::Moonshot, ProviderConfig::Moonshot(_))
            | (ProviderKind::Ollama, ProviderConfig::Ollama(_))
            | (ProviderKind::OpenAi, ProviderConfig::OpenAi(_))
            | (ProviderKind::OpenRouter, ProviderConfig::OpenRouter(_))
            | (ProviderKind::Perplexity, ProviderConfig::Perplexity(_))
            | (ProviderKind::Together, ProviderConfig::Together(_))
            | (ProviderKind::Xai, ProviderConfig::Xai(_))
    )
}

fn dedupe_models(models: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for model in models {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = trimmed.to_string();
        if !deduped.contains(&candidate) {
            deduped.push(candidate);
        }
    }
    deduped
}

fn shift_in_slice<T>(current: T, all: &[T], delta: i32) -> T
where
    T: Copy + PartialEq,
{
    let index = all.iter().position(|item| *item == current).unwrap_or(0);
    let next = ((index as i32 + delta).rem_euclid(all.len() as i32)) as usize;
    all[next]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        let unique = format!(
            "providers-{}-{}-{}.json",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn persistence_round_trip_keeps_provider_metadata() {
        let path = test_path("round-trip");
        let mut registry = ProviderRegistry::with_default_ollama(path.clone());
        let provider_id = registry.add_provider(ProviderKind::OpenAi);
        let provider = registry
            .provider_mut(&provider_id)
            .expect("provider should exist");
        provider.name = "Primary OpenAI".into();
        provider.variant = ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi);
        provider.config = ProviderConfig::OpenAi(ApiKeyProviderConfig {
            api_key: "test-key".into(),
            base_url: Some("https://example.com/v1".into()),
        });
        provider.cached_models = vec!["gpt-4.1".into(), "gpt-4.1".into(), "gpt-4o".into()];
        provider.status = ProviderStatus::ready("verified");

        registry.save_to_disk().expect("save should succeed");
        let reloaded = ProviderRegistry::load_from_path(&path)
            .expect("load should succeed")
            .expect("registry should exist");

        let reloaded = reloaded
            .provider(&provider_id)
            .expect("provider should round trip");
        assert_eq!(reloaded.name, "Primary OpenAI");
        assert_eq!(
            reloaded.variant,
            ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi)
        );
        assert_eq!(reloaded.cached_models, vec!["gpt-4.1", "gpt-4o"]);
        assert_eq!(reloaded.status.kind, ProviderStatusKind::Ready);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn removing_provider_updates_order() {
        let path = test_path("remove");
        let mut registry = ProviderRegistry::with_default_ollama(path);
        let provider_id = registry.add_provider(ProviderKind::Gemini);
        assert!(registry.remove_provider(&provider_id));
        assert!(registry.provider(&provider_id).is_none());
        assert!(!registry.order.contains(&provider_id));
    }

    #[test]
    fn kind_change_resets_variant_and_config() {
        let mut provider = ProviderRegistration::new("provider-9".into(), ProviderKind::OpenAi);
        provider.variant = ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi);
        provider.config = ProviderConfig::OpenAi(ApiKeyProviderConfig {
            api_key: "test".into(),
            base_url: Some("https://example.com/v1".into()),
        });

        provider.set_kind(ProviderKind::Ollama);

        assert_eq!(provider.kind, ProviderKind::Ollama);
        assert_eq!(provider.variant, ProviderVariant::Standard);
        assert!(matches!(provider.config, ProviderConfig::Ollama(_)));
        assert!(provider.cached_models.is_empty());
    }
}
