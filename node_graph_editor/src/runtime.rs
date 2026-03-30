use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use reqwest::Client as LocalHttpClient;
use rig::{
    client::{
        Client as RigClient, DebugExt as RigDebugExt, Nothing, Provider as RigProvider,
        VerifyClient,
    },
    completion::{CompletionModel as RigCompletionModel, Prompt},
    http_client::{self as rig_http, HttpClientExt as RigHttpClientExt, NoBody},
    message::ToolChoice as RigToolChoice,
    prelude::CompletionClient,
    providers::{
        anthropic, azure, cohere, deepseek, galadriel, gemini, groq, huggingface, hyperbolic,
        llamafile, mira, mistral, moonshot, ollama, openai, openrouter, perplexity, together, xai,
    },
    wasm_compat::WasmCompatSync,
};
use schemars::Schema;
use serde::Deserialize;
use serde_json::Value;
use tokio::{runtime::Runtime, task::JoinHandle};

use crate::{
    catalog::{NodeId, ToolChoiceSetting},
    document::GraphDocument,
    providers::{
        ApiKeyProviderConfig, AzureAuthKind, AzureProviderConfig, EndpointProviderConfig,
        GaladrielProviderConfig, GeminiVariant, HuggingFaceProviderConfig, HuggingFaceSubprovider,
        OpenAiVariant, ProviderConfig, ProviderId, ProviderKind, ProviderRefreshResult,
        ProviderRegistration, ProviderRegistry, ProviderStatus, ProviderVariant,
    },
};

pub struct NodeGraphRuntimePlugin;

impl Plugin for NodeGraphRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProviderRegistry>()
            .init_resource::<RigEditorRuntime>()
            .add_systems(
                Startup,
                (
                    seed_document_provider_defaults,
                    kickoff_initial_provider_refresh,
                ),
            )
            .add_systems(
                Update,
                (poll_runtime_messages, sync_document_provider_defaults),
            );
    }
}

#[derive(Resource)]
pub struct RigEditorRuntime {
    runtime: Arc<Runtime>,
    tx: Sender<RuntimeMessage>,
    rx: Receiver<RuntimeMessage>,
    pub pending_request: Option<u64>,
    pub pending_output_node: Option<NodeId>,
    pub last_status: String,
    next_request_id: u64,
    pending_task: Option<JoinHandle<()>>,
}

impl Default for RigEditorRuntime {
    fn default() -> Self {
        let runtime = Runtime::new().expect("tokio runtime should initialize");
        let (tx, rx) = unbounded();

        Self {
            runtime: Arc::new(runtime),
            tx,
            rx,
            pending_request: None,
            pending_output_node: None,
            last_status: "Provider refresh pending.".into(),
            next_request_id: 1,
            pending_task: None,
        }
    }
}

impl RigEditorRuntime {
    pub fn request_provider_refresh(&mut self, provider: ProviderRegistration) {
        let provider_label = provider.display_name().to_string();
        let provider_id = provider.id.clone();
        let tx = self.tx.clone();
        let runtime = self.runtime.clone();
        self.last_status = format!("Refreshing {provider_label} …");

        runtime.spawn(async move {
            let result = refresh_provider(provider).await;
            let _ = tx.send(RuntimeMessage::ProviderRefreshed {
                provider_id,
                result,
            });
        });
    }

    pub fn request_run(&mut self, request: CompiledAgentRun) -> Result<()> {
        if self.pending_request.is_some() {
            return Err(anyhow!("a graph run is already in flight"));
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;
        self.pending_request = Some(request_id);
        self.pending_output_node = Some(request.output_node);
        self.last_status = format!(
            "Running {} on {} / {} …",
            request.agent_name.as_deref().unwrap_or("selected agent"),
            request.provider.display_name(),
            request.model
        );

        let tx = self.tx.clone();
        let runtime = self.runtime.clone();
        let handle = runtime.spawn(async move {
            let output_node = request.output_node;
            let result = execute_provider_request(request_id, request).await;
            let message = match result {
                Ok(message) => message,
                Err(error) => RuntimeMessage::RunFailed {
                    request_id,
                    output_node: Some(output_node),
                    error: error.to_string(),
                },
            };
            let _ = tx.send(message);
        });
        self.pending_task = Some(handle);

        Ok(())
    }

    pub fn stop_run(&mut self) -> Option<NodeId> {
        if let Some(handle) = self.pending_task.take() {
            handle.abort();
        }

        let request = self.pending_request.take();
        let output_node = self.pending_output_node.take();
        self.last_status = match request {
            Some(request_id) => format!("Run #{request_id} stopped."),
            None => "No graph run is in flight.".into(),
        };
        output_node
    }
}

#[derive(Debug, Clone)]
pub struct CompiledAgentRun {
    pub agent_id: NodeId,
    pub output_node: NodeId,
    pub provider: ProviderRegistration,
    pub model: String,
    pub prompt: String,
    pub agent_name: Option<String>,
    pub description: Option<String>,
    pub preamble: Option<String>,
    pub static_context: Vec<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub additional_params: Option<Value>,
    pub tool_choice: Option<ToolChoiceSetting>,
    pub default_max_turns: Option<usize>,
    pub output_schema: Option<Schema>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
enum RuntimeMessage {
    ProviderRefreshed {
        provider_id: ProviderId,
        result: ProviderRefreshResult,
    },
    RunComplete {
        request_id: u64,
        output_node: NodeId,
        text: String,
        status: String,
    },
    RunFailed {
        request_id: u64,
        output_node: Option<NodeId>,
        error: String,
    },
}

fn seed_document_provider_defaults(
    mut document: ResMut<GraphDocument>,
    providers: Res<ProviderRegistry>,
) {
    document.apply_provider_registry(&providers);
}

fn kickoff_initial_provider_refresh(
    mut runtime: ResMut<RigEditorRuntime>,
    mut providers: ResMut<ProviderRegistry>,
) {
    let registrations = providers
        .ordered_registrations()
        .cloned()
        .collect::<Vec<_>>();
    if registrations.is_empty() {
        runtime.last_status = "No providers registered. Add one from any Model node.".into();
        return;
    }

    for provider in registrations {
        providers.mark_refreshing(&provider.id);
        runtime.request_provider_refresh(provider);
    }
}

fn poll_runtime_messages(
    mut runtime: ResMut<RigEditorRuntime>,
    mut providers: ResMut<ProviderRegistry>,
    mut document: ResMut<GraphDocument>,
) {
    while let Ok(message) = runtime.rx.try_recv() {
        match message {
            RuntimeMessage::ProviderRefreshed {
                provider_id,
                result,
            } => {
                let provider_name = providers
                    .provider(&provider_id)
                    .map(|provider| provider.display_name().to_string())
                    .unwrap_or_else(|| provider_id.clone());
                let detail = result.status.detail.clone();
                if providers.apply_refresh_result(&provider_id, result) {
                    if let Err(error) = providers.save_to_disk() {
                        runtime.last_status =
                            format!("Saved provider update failed for {provider_name}: {error}");
                    }
                    document.apply_provider_registry(&providers);
                }
                runtime.last_status = format!("{provider_name}: {detail}");
            }
            RuntimeMessage::RunComplete {
                request_id,
                output_node,
                text,
                status,
            } => {
                runtime.pending_request = None;
                runtime.pending_output_node = None;
                runtime.pending_task = None;
                runtime.last_status = format!("Run #{request_id} finished.");
                document.set_output_result(output_node, text, status);
            }
            RuntimeMessage::RunFailed {
                request_id,
                output_node,
                error,
            } => {
                runtime.pending_request = None;
                runtime.pending_output_node = None;
                runtime.pending_task = None;
                runtime.last_status = format!("Run #{request_id} failed: {error}");
                if let Some(output_node) = output_node {
                    document.set_output_result(output_node, error.clone(), "failed".into());
                }
            }
        }
    }
}

fn sync_document_provider_defaults(
    mut document: ResMut<GraphDocument>,
    providers: Res<ProviderRegistry>,
) {
    if providers.is_changed() {
        document.apply_provider_registry(&providers);
    }
}

async fn execute_provider_request(
    request_id: u64,
    request: CompiledAgentRun,
) -> Result<RuntimeMessage> {
    match (
        &request.provider.kind,
        &request.provider.variant,
        &request.provider.config,
    ) {
        (ProviderKind::Anthropic, ProviderVariant::Standard, ProviderConfig::Anthropic(config)) => {
            let client = build_anthropic_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Azure, ProviderVariant::Standard, ProviderConfig::Azure(config)) => {
            let client = build_azure_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Cohere, ProviderVariant::Standard, ProviderConfig::Cohere(config)) => {
            let client = build_cohere_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Deepseek, ProviderVariant::Standard, ProviderConfig::Deepseek(config)) => {
            let client = build_deepseek_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Galadriel, ProviderVariant::Standard, ProviderConfig::Galadriel(config)) => {
            let client = build_galadriel_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::Gemini,
            ProviderVariant::Gemini(GeminiVariant::GenerateContent),
            ProviderConfig::Gemini(config),
        ) => {
            let client = build_gemini_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::Gemini,
            ProviderVariant::Gemini(GeminiVariant::Interactions),
            ProviderConfig::Gemini(config),
        ) => {
            let client = build_gemini_client(config)?.interactions_api();
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Groq, ProviderVariant::Standard, ProviderConfig::Groq(config)) => {
            let client = build_groq_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::HuggingFace,
            ProviderVariant::Standard,
            ProviderConfig::HuggingFace(config),
        ) => {
            let client = build_huggingface_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::Hyperbolic,
            ProviderVariant::Standard,
            ProviderConfig::Hyperbolic(config),
        ) => {
            let client = build_hyperbolic_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Llamafile, ProviderVariant::Standard, ProviderConfig::Llamafile(config)) => {
            let client = build_llamafile_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Mira, ProviderVariant::Standard, ProviderConfig::Mira(config)) => {
            let client = build_mira_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Mistral, ProviderVariant::Standard, ProviderConfig::Mistral(config)) => {
            let client = build_mistral_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Moonshot, ProviderVariant::Standard, ProviderConfig::Moonshot(config)) => {
            let client = build_moonshot_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Ollama, ProviderVariant::Standard, ProviderConfig::Ollama(config)) => {
            let client = build_ollama_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::OpenAi,
            ProviderVariant::OpenAi(OpenAiVariant::ResponsesApi),
            ProviderConfig::OpenAi(config),
        ) => {
            let client = build_openai_responses_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::OpenAi,
            ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi),
            ProviderConfig::OpenAi(config),
        ) => {
            let client = build_openai_completions_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::OpenRouter,
            ProviderVariant::Standard,
            ProviderConfig::OpenRouter(config),
        ) => {
            let client = build_openrouter_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (
            ProviderKind::Perplexity,
            ProviderVariant::Standard,
            ProviderConfig::Perplexity(config),
        ) => {
            let client = build_perplexity_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Together, ProviderVariant::Standard, ProviderConfig::Together(config)) => {
            let client = build_together_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        (ProviderKind::Xai, ProviderVariant::Standard, ProviderConfig::Xai(config)) => {
            let client = build_xai_client(config)?;
            execute_request_with_client(request_id, request, client).await
        }
        _ => Err(anyhow!("provider registration is internally inconsistent")),
    }
}

async fn execute_request_with_client<C>(
    request_id: u64,
    request: CompiledAgentRun,
    client: C,
) -> Result<RuntimeMessage>
where
    C: CompletionClient + Send + Sync + 'static,
    C::CompletionModel: RigCompletionModel<Client = C> + Send + Sync + 'static,
{
    let mut builder = client.agent(request.model.clone());

    if let Some(name) = &request.agent_name {
        builder = builder.name(name);
    }
    if let Some(description) = &request.description {
        builder = builder.description(description);
    }
    if let Some(preamble) = &request.preamble {
        builder = builder.preamble(preamble);
    }
    for document in &request.static_context {
        builder = builder.context(document);
    }
    if let Some(temperature) = request.temperature {
        builder = builder.temperature(temperature);
    }
    if let Some(max_tokens) = request.max_tokens {
        builder = builder.max_tokens(max_tokens);
    }
    if let Some(params) = request.additional_params.clone() {
        builder = builder.additional_params(params);
    }
    if let Some(choice) = request.tool_choice {
        builder = builder.tool_choice(to_rig_tool_choice(choice));
    }
    if let Some(max_turns) = request.default_max_turns {
        builder = builder.default_max_turns(max_turns);
    }
    if let Some(schema) = request.output_schema.clone() {
        builder = builder.output_schema_raw(schema);
    }

    let provider_label = request.provider.display_name().to_string();
    let agent = builder.build();
    let response = agent
        .prompt(request.prompt.as_str())
        .await
        .map_err(|error| anyhow!(error.to_string()))?;

    let provider_family = request.provider.family_label();
    let status = if request.warnings.is_empty() {
        format!(
            "completed via {provider_label} ({provider_family}) / {}",
            request.model
        )
    } else {
        format!(
            "completed via {provider_label} ({provider_family}) / {} (warnings: {})",
            request.model,
            request.warnings.join("; ")
        )
    };

    Ok(RuntimeMessage::RunComplete {
        request_id,
        output_node: request.output_node,
        text: response,
        status,
    })
}

async fn refresh_provider(provider: ProviderRegistration) -> ProviderRefreshResult {
    if let Some(error) = provider.config_error() {
        return ProviderRefreshResult::new(ProviderStatus::needs_config(error), Vec::new());
    }

    let cached = provider.cached_models.clone();
    let label = provider.display_name().to_string();

    match (&provider.kind, &provider.variant, &provider.config) {
        (ProviderKind::Anthropic, ProviderVariant::Standard, ProviderConfig::Anthropic(config)) => {
            match build_anthropic_client(config) {
                Ok(client) => {
                    refresh_with_model_listing(client, "/v1/models", provider.kind, &label, cached)
                        .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Azure, ProviderVariant::Standard, ProviderConfig::Azure(_)) => {
            ProviderRefreshResult::new(
                ProviderStatus::ready(
                    "Azure deployment discovery is manual; enter the deployment name in the model field",
                ),
                cached,
            )
        }
        (ProviderKind::Cohere, ProviderVariant::Standard, ProviderConfig::Cohere(config)) => {
            match build_cohere_client(config) {
                Ok(client) => {
                    refresh_with_model_listing(client, "/models", provider.kind, &label, cached)
                        .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Deepseek, ProviderVariant::Standard, ProviderConfig::Deepseek(config)) => {
            match build_deepseek_client(config) {
                Ok(client) => {
                    refresh_with_verification(
                        client,
                        &label,
                        "DeepSeek verified; enter models manually",
                        cached,
                    )
                    .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Galadriel, ProviderVariant::Standard, ProviderConfig::Galadriel(_)) => {
            ProviderRefreshResult::new(
                ProviderStatus::ready(
                    "Galadriel verification is not exposed by Rig; enter models manually",
                ),
                cached,
            )
        }
        (ProviderKind::Gemini, ProviderVariant::Gemini(_), ProviderConfig::Gemini(config)) => {
            match build_gemini_client(config) {
                Ok(client) => {
                    refresh_with_model_listing(
                        client,
                        "/v1beta/models",
                        provider.kind,
                        &label,
                        cached,
                    )
                    .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Groq, ProviderVariant::Standard, ProviderConfig::Groq(config)) => {
            match build_groq_client(config) {
                Ok(client) => {
                    refresh_with_model_listing(client, "/models", provider.kind, &label, cached)
                        .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (
            ProviderKind::HuggingFace,
            ProviderVariant::Standard,
            ProviderConfig::HuggingFace(config),
        ) => match build_huggingface_client(config) {
            Ok(client) => {
                refresh_with_verification(
                    client,
                    &label,
                    "Hugging Face verified; model entry is manual in this editor",
                    cached,
                )
                .await
            }
            Err(error) => {
                ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
            }
        },
        (
            ProviderKind::Hyperbolic,
            ProviderVariant::Standard,
            ProviderConfig::Hyperbolic(config),
        ) => match build_hyperbolic_client(config) {
            Ok(client) => {
                refresh_with_model_listing(client, "/models", provider.kind, &label, cached).await
            }
            Err(error) => {
                ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
            }
        },
        (ProviderKind::Llamafile, ProviderVariant::Standard, ProviderConfig::Llamafile(config)) => {
            match build_llamafile_client(config) {
                Ok(client) => {
                    refresh_with_model_listing(client, "/v1/models", provider.kind, &label, cached)
                        .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Mira, ProviderVariant::Standard, ProviderConfig::Mira(config)) => {
            match build_mira_client(config) {
                Ok(client) => match client.list_models().await {
                    Ok(models) => {
                        let detail = if models.is_empty() {
                            "Mira reachable; enter models manually".to_string()
                        } else {
                            format!("Mira ready with {} discovered model(s)", models.len())
                        };
                        ProviderRefreshResult::new(ProviderStatus::ready(detail), models)
                    }
                    Err(error) => ProviderRefreshResult::new(
                        ProviderStatus::error(format!("Mira refresh failed: {error}")),
                        cached,
                    ),
                },
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Mistral, ProviderVariant::Standard, ProviderConfig::Mistral(config)) => {
            match build_mistral_client(config) {
                Ok(client) => {
                    refresh_with_model_listing(client, "/models", provider.kind, &label, cached)
                        .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Moonshot, ProviderVariant::Standard, ProviderConfig::Moonshot(config)) => {
            match build_moonshot_client(config) {
                Ok(client) => {
                    refresh_with_model_listing(client, "/models", provider.kind, &label, cached)
                        .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Ollama, ProviderVariant::Standard, ProviderConfig::Ollama(config)) => {
            match discover_ollama_models(&config.base_url).await {
                Ok(models) => ProviderRefreshResult::new(
                    ProviderStatus::ready(format!(
                        "{} ready with {} discovered model(s)",
                        label,
                        models.len()
                    )),
                    models,
                ),
                Err(error) => ProviderRefreshResult::new(
                    ProviderStatus::error(format!("{} refresh failed: {error}", label)),
                    cached,
                ),
            }
        }
        (
            ProviderKind::OpenAi,
            ProviderVariant::OpenAi(OpenAiVariant::ResponsesApi),
            ProviderConfig::OpenAi(config),
        ) => match build_openai_responses_client(config) {
            Ok(client) => {
                refresh_with_model_listing(client, "/models", provider.kind, &label, cached).await
            }
            Err(error) => {
                ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
            }
        },
        (
            ProviderKind::OpenAi,
            ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi),
            ProviderConfig::OpenAi(config),
        ) => match build_openai_completions_client(config) {
            Ok(client) => {
                refresh_with_model_listing(client, "/models", provider.kind, &label, cached).await
            }
            Err(error) => {
                ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
            }
        },
        (
            ProviderKind::OpenRouter,
            ProviderVariant::Standard,
            ProviderConfig::OpenRouter(config),
        ) => match build_openrouter_client(config) {
            Ok(client) => {
                refresh_with_model_listing(client, "/models", provider.kind, &label, cached).await
            }
            Err(error) => {
                ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
            }
        },
        (ProviderKind::Perplexity, ProviderVariant::Standard, ProviderConfig::Perplexity(_)) => {
            ProviderRefreshResult::new(
                ProviderStatus::ready(
                    "Perplexity verification is not exposed by Rig; enter models manually",
                ),
                cached,
            )
        }
        (ProviderKind::Together, ProviderVariant::Standard, ProviderConfig::Together(config)) => {
            match build_together_client(config) {
                Ok(client) => {
                    refresh_with_model_listing_no_verify(
                        client,
                        "/models",
                        provider.kind,
                        &label,
                        cached,
                    )
                    .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        (ProviderKind::Xai, ProviderVariant::Standard, ProviderConfig::Xai(config)) => {
            match build_xai_client(config) {
                Ok(client) => {
                    refresh_with_verification(
                        client,
                        &label,
                        "xAI verified; enter models manually",
                        cached,
                    )
                    .await
                }
                Err(error) => {
                    ProviderRefreshResult::new(ProviderStatus::error(error.to_string()), cached)
                }
            }
        }
        _ => ProviderRefreshResult::new(
            ProviderStatus::error("provider registration is internally inconsistent"),
            cached,
        ),
    }
}

async fn refresh_with_verification<C>(
    client: C,
    label: &str,
    ready_detail: &str,
    cached_models: Vec<String>,
) -> ProviderRefreshResult
where
    C: VerifyClient,
{
    match client.verify().await {
        Ok(()) => ProviderRefreshResult::new(ProviderStatus::ready(ready_detail), cached_models),
        Err(error) => ProviderRefreshResult::new(
            ProviderStatus::error(format!("{label} verification failed: {error}")),
            cached_models,
        ),
    }
}

async fn refresh_with_model_listing<Ext>(
    client: RigClient<Ext>,
    path: &str,
    kind: ProviderKind,
    label: &str,
    cached_models: Vec<String>,
) -> ProviderRefreshResult
where
    Ext: RigProvider + RigDebugExt + WasmCompatSync + Send + Sync + 'static,
{
    match fetch_model_listing(&client, path, kind).await {
        Ok(models) => {
            if models.is_empty() {
                match client.verify().await {
                    Ok(()) => ProviderRefreshResult::new(
                        ProviderStatus::ready(format!(
                            "{label} verified; model discovery returned no chat models, enter one manually"
                        )),
                        cached_models,
                    ),
                    Err(error) => ProviderRefreshResult::new(
                        ProviderStatus::error(format!("{label} verification failed: {error}")),
                        cached_models,
                    ),
                }
            } else {
                ProviderRefreshResult::new(
                    ProviderStatus::ready(format!(
                        "{label} ready with {} discovered model(s)",
                        models.len()
                    )),
                    models,
                )
            }
        }
        Err(list_error) => match client.verify().await {
            Ok(()) => ProviderRefreshResult::new(
                ProviderStatus::ready(format!(
                    "{label} verified; model discovery is unavailable ({list_error}), enter models manually"
                )),
                cached_models,
            ),
            Err(error) => ProviderRefreshResult::new(
                ProviderStatus::error(format!(
                    "{label} refresh failed: {list_error}; verification also failed: {error}"
                )),
                cached_models,
            ),
        },
    }
}

async fn refresh_with_model_listing_no_verify<Ext>(
    client: RigClient<Ext>,
    path: &str,
    kind: ProviderKind,
    label: &str,
    cached_models: Vec<String>,
) -> ProviderRefreshResult
where
    Ext: RigProvider + Send + Sync + 'static,
{
    match fetch_model_listing(&client, path, kind).await {
        Ok(models) if models.is_empty() => ProviderRefreshResult::new(
            ProviderStatus::ready(format!(
                "{label} reachable; model discovery returned no chat models, enter one manually"
            )),
            cached_models,
        ),
        Ok(models) => ProviderRefreshResult::new(
            ProviderStatus::ready(format!(
                "{label} ready with {} discovered model(s)",
                models.len()
            )),
            models,
        ),
        Err(error) => ProviderRefreshResult::new(
            ProviderStatus::error(format!("{label} refresh failed: {error}")),
            cached_models,
        ),
    }
}

async fn fetch_model_listing<Ext>(
    client: &RigClient<Ext>,
    path: &str,
    kind: ProviderKind,
) -> Result<Vec<String>>
where
    Ext: RigProvider + Send + Sync + 'static,
{
    let request = client
        .get(path)?
        .body(NoBody)
        .map_err(rig_http::Error::from)
        .map_err(|error| anyhow!(error.to_string()))?;
    let response = client
        .send::<_, Vec<u8>>(request)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let body = rig_http::text(response).await.unwrap_or_default();
        return Err(anyhow!("{} {}", status, body.trim()));
    }

    let body = rig_http::text(response)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    parse_model_listing_json(kind, &body)
}

fn parse_model_listing_json(kind: ProviderKind, body: &str) -> Result<Vec<String>> {
    let value = serde_json::from_str::<Value>(body)
        .map_err(|error| anyhow!("invalid model listing response: {error}"))?;
    let mut models = Vec::new();
    for key in ["data", "models", "items"] {
        if let Some(entries) = value.get(key).and_then(Value::as_array) {
            for entry in entries {
                if let Some(model) = extract_model_name(kind, entry) {
                    if !models.contains(&model) {
                        models.push(model);
                    }
                }
            }
        }
    }
    Ok(models)
}

fn extract_model_name(kind: ProviderKind, entry: &Value) -> Option<String> {
    let raw = ["id", "name", "model"]
        .into_iter()
        .find_map(|key| entry.get(key).and_then(Value::as_str))?;
    normalize_discovered_model(kind, raw)
}

fn normalize_discovered_model(kind: ProviderKind, value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = match kind {
        ProviderKind::Gemini => trimmed.strip_prefix("models/").unwrap_or(trimmed),
        _ => trimmed,
    };

    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn to_rig_tool_choice(choice: ToolChoiceSetting) -> RigToolChoice {
    match choice {
        ToolChoiceSetting::Auto => RigToolChoice::Auto,
        ToolChoiceSetting::None => RigToolChoice::None,
        ToolChoiceSetting::Required => RigToolChoice::Required,
    }
}

fn build_anthropic_client(config: &ApiKeyProviderConfig) -> Result<anthropic::Client> {
    let mut builder = anthropic::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_azure_client(config: &AzureProviderConfig) -> Result<azure::Client> {
    let auth = match config.auth_kind {
        AzureAuthKind::ApiKey => azure::AzureOpenAIAuth::ApiKey(config.credential.clone()),
        AzureAuthKind::Token => azure::AzureOpenAIAuth::Token(config.credential.clone()),
    };
    azure::Client::builder()
        .api_key(auth)
        .azure_endpoint(config.endpoint.clone())
        .api_version(config.api_version.as_str())
        .build()
        .map_err(|error| anyhow!(error.to_string()))
}

fn build_cohere_client(config: &ApiKeyProviderConfig) -> Result<cohere::Client> {
    let mut builder = cohere::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_deepseek_client(config: &ApiKeyProviderConfig) -> Result<deepseek::Client> {
    let mut builder = deepseek::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_galadriel_client(config: &GaladrielProviderConfig) -> Result<galadriel::Client> {
    let mut builder = galadriel::Client::builder().api_key(config.api_key.as_str());
    if let Some(fine_tune_key) = config.fine_tune_api_key.as_deref() {
        builder = builder.fine_tune_api_key(fine_tune_key);
    }
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_gemini_client(config: &ApiKeyProviderConfig) -> Result<gemini::Client> {
    let mut builder = gemini::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_groq_client(config: &ApiKeyProviderConfig) -> Result<groq::Client> {
    let mut builder = groq::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_huggingface_client(config: &HuggingFaceProviderConfig) -> Result<huggingface::Client> {
    let mut builder = huggingface::Client::builder()
        .api_key(config.api_key.as_str())
        .subprovider(to_huggingface_subprovider(config.subprovider));
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_hyperbolic_client(config: &ApiKeyProviderConfig) -> Result<hyperbolic::Client> {
    let mut builder = hyperbolic::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_llamafile_client(config: &EndpointProviderConfig) -> Result<llamafile::Client> {
    llamafile::Client::builder()
        .api_key(Nothing)
        .base_url(config.base_url.as_str())
        .build()
        .map_err(|error| anyhow!(error.to_string()))
}

fn build_mira_client(config: &ApiKeyProviderConfig) -> Result<mira::Client> {
    let mut builder = mira::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_mistral_client(config: &ApiKeyProviderConfig) -> Result<mistral::Client> {
    let mut builder = mistral::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_moonshot_client(config: &ApiKeyProviderConfig) -> Result<moonshot::Client> {
    let mut builder = moonshot::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_ollama_client(config: &EndpointProviderConfig) -> Result<ollama::Client> {
    ollama::Client::builder()
        .api_key(Nothing)
        .base_url(config.base_url.as_str())
        .build()
        .map_err(|error| anyhow!(error.to_string()))
}

fn build_openai_responses_client(config: &ApiKeyProviderConfig) -> Result<openai::Client> {
    let mut builder = openai::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_openai_completions_client(
    config: &ApiKeyProviderConfig,
) -> Result<openai::CompletionsClient> {
    let mut builder = openai::CompletionsClient::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_openrouter_client(config: &ApiKeyProviderConfig) -> Result<openrouter::Client> {
    let mut builder = openrouter::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_perplexity_client(config: &ApiKeyProviderConfig) -> Result<perplexity::Client> {
    let mut builder = perplexity::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_together_client(config: &ApiKeyProviderConfig) -> Result<together::Client> {
    let mut builder = together::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn build_xai_client(config: &ApiKeyProviderConfig) -> Result<xai::Client> {
    let mut builder = xai::Client::builder().api_key(config.api_key.as_str());
    if let Some(base_url) = config.base_url.as_deref() {
        builder = builder.base_url(base_url);
    }
    builder.build().map_err(|error| anyhow!(error.to_string()))
}

fn to_huggingface_subprovider(
    subprovider: HuggingFaceSubprovider,
) -> huggingface::client::SubProvider {
    match subprovider {
        HuggingFaceSubprovider::HfInference => huggingface::client::SubProvider::HFInference,
        HuggingFaceSubprovider::Together => huggingface::client::SubProvider::Together,
        HuggingFaceSubprovider::SambaNova => huggingface::client::SubProvider::SambaNova,
        HuggingFaceSubprovider::Fireworks => huggingface::client::SubProvider::Fireworks,
        HuggingFaceSubprovider::Hyperbolic => huggingface::client::SubProvider::Hyperbolic,
        HuggingFaceSubprovider::Nebius => huggingface::client::SubProvider::Nebius,
        HuggingFaceSubprovider::Novita => huggingface::client::SubProvider::Novita,
    }
}

#[derive(Debug, Deserialize)]
struct OllamaModelEnvelope {
    #[serde(default)]
    models: Vec<OllamaModelRecord>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelRecord {
    name: Option<String>,
    model: Option<String>,
}

async fn discover_ollama_models(endpoint: &str) -> Result<Vec<String>> {
    let client = LocalHttpClient::builder()
        .timeout(Duration::from_millis(700))
        .build()
        .map_err(|error| anyhow!(error.to_string()))?;

    let running = fetch_ollama_models(&client, endpoint, "api/ps").await?;
    let installed = fetch_ollama_models(&client, endpoint, "api/tags").await?;

    let mut combined = Vec::new();
    for model in running.into_iter().chain(installed.into_iter()) {
        if looks_like_chat_model(&model) && !combined.contains(&model) {
            combined.push(model);
        }
    }
    if combined.is_empty() {
        return Err(anyhow!("no local chat-capable Ollama models discovered"));
    }
    Ok(combined)
}

async fn fetch_ollama_models(
    client: &LocalHttpClient,
    endpoint: &str,
    path: &str,
) -> Result<Vec<String>> {
    let url = join_api_url(endpoint, path)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let response = response
        .error_for_status()
        .map_err(|error| anyhow!(error.to_string()))?;
    let payload = response
        .json::<OllamaModelEnvelope>()
        .await
        .map_err(|error| anyhow!(error.to_string()))?;

    Ok(payload
        .models
        .into_iter()
        .filter_map(ollama_model_name)
        .collect())
}

fn join_api_url(endpoint: &str, path: &str) -> Result<String> {
    let base = endpoint.trim().trim_end_matches('/');
    let path = path.trim().trim_start_matches('/');
    if base.is_empty() || path.is_empty() {
        return Err(anyhow!("invalid Ollama endpoint"));
    }
    Ok(format!("{base}/{path}"))
}

fn ollama_model_name(record: OllamaModelRecord) -> Option<String> {
    record
        .name
        .or(record.model)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn looks_like_chat_model(model: &str) -> bool {
    let lowered = model.trim().to_ascii_lowercase();
    if lowered.is_empty() {
        return false;
    }

    !["embed", "embedding", "nomic-embed", "bge-", "e5-", "rerank"]
        .iter()
        .any(|needle| lowered.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_variants_remain_distinct() {
        let responses = ProviderRegistration {
            id: "provider-openai-responses".into(),
            name: "Responses".into(),
            kind: ProviderKind::OpenAi,
            variant: ProviderVariant::OpenAi(OpenAiVariant::ResponsesApi),
            config: ProviderConfig::OpenAi(ApiKeyProviderConfig {
                api_key: "key".into(),
                base_url: None,
            }),
            cached_models: Vec::new(),
            status: ProviderStatus::ready("ready"),
        };
        let completions = ProviderRegistration {
            variant: ProviderVariant::OpenAi(OpenAiVariant::CompletionsApi),
            ..responses.clone()
        };
        let gemini_standard = ProviderRegistration {
            id: "provider-gemini-standard".into(),
            name: "Gemini".into(),
            kind: ProviderKind::Gemini,
            variant: ProviderVariant::Gemini(GeminiVariant::GenerateContent),
            config: ProviderConfig::Gemini(ApiKeyProviderConfig {
                api_key: "key".into(),
                base_url: None,
            }),
            cached_models: Vec::new(),
            status: ProviderStatus::ready("ready"),
        };
        let gemini_interactions = ProviderRegistration {
            variant: ProviderVariant::Gemini(GeminiVariant::Interactions),
            ..gemini_standard.clone()
        };

        assert_ne!(responses.variant, completions.variant);
        assert_ne!(gemini_standard.variant, gemini_interactions.variant);
    }

    #[test]
    fn gemini_model_listing_strips_models_prefix() {
        let payload =
            r#"{"models":[{"name":"models/gemini-2.5-flash"},{"name":"models/gemini-2.5-pro"}]}"#;
        let models =
            parse_model_listing_json(ProviderKind::Gemini, payload).expect("payload should parse");
        assert_eq!(models, vec!["gemini-2.5-flash", "gemini-2.5-pro"]);
    }
}
