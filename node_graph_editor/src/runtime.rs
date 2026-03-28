use std::{env, sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use reqwest::Client as HttpClient;
use rig::{client::Nothing, completion::Prompt, prelude::CompletionClient, providers::ollama};
use schemars::Schema;
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use crate::{
    catalog::{NodeId, ToolChoiceSetting},
    document::GraphDocument,
};

const OLLAMA_API_BASE_URL: &str = "http://localhost:11434";

pub struct NodeGraphRuntimePlugin;

impl Plugin for NodeGraphRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RigEditorRuntime>()
            .add_systems(Startup, kickoff_initial_model_refresh)
            .add_systems(Update, poll_runtime_messages);
    }
}

#[derive(Resource)]
pub struct RigEditorRuntime {
    runtime: Arc<Runtime>,
    tx: Sender<RuntimeMessage>,
    rx: Receiver<RuntimeMessage>,
    pub ollama_endpoint: String,
    pub ollama_ready: bool,
    pub ollama_detail: String,
    pub ollama_models: Vec<String>,
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
        let endpoint = env::var("OLLAMA_API_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| OLLAMA_API_BASE_URL.to_string());

        Self {
            runtime: Arc::new(runtime),
            tx,
            rx,
            ollama_endpoint: endpoint,
            ollama_ready: false,
            ollama_detail: "discovering local models".into(),
            ollama_models: Vec::new(),
            pending_request: None,
            pending_output_node: None,
            last_status: "Local Ollama discovery pending.".into(),
            next_request_id: 1,
            pending_task: None,
        }
    }
}

impl RigEditorRuntime {
    pub fn request_model_refresh(&mut self) {
        let tx = self.tx.clone();
        let endpoint = self.ollama_endpoint.clone();
        let runtime = self.runtime.clone();
        self.last_status = format!("Refreshing local Ollama models from {endpoint} …");

        runtime.spawn(async move {
            let message = match discover_ollama_models(&endpoint).await {
                Ok(models) => RuntimeMessage::DiscoveryComplete {
                    ready: true,
                    detail: if models.is_empty() {
                        format!("{endpoint} (reachable, no chat models found)")
                    } else {
                        format!("{endpoint} ({} chat models)", models.len())
                    },
                    models,
                },
                Err(error) => RuntimeMessage::DiscoveryComplete {
                    ready: false,
                    detail: format!("{endpoint} ({})", error),
                    models: Vec::new(),
                },
            };

            let _ = tx.send(message);
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
            "Running {} on Ollama / {} …",
            request.agent_name.as_deref().unwrap_or("selected agent"),
            request.model
        );

        let tx = self.tx.clone();
        let runtime = self.runtime.clone();
        let handle = runtime.spawn(async move {
            let result = execute_ollama_request(request_id, request).await;
            let message = match result {
                Ok(message) => message,
                Err(error) => RuntimeMessage::RunFailed {
                    request_id,
                    output_node: None,
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
    pub endpoint: String,
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
    DiscoveryComplete {
        ready: bool,
        detail: String,
        models: Vec<String>,
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

fn kickoff_initial_model_refresh(mut runtime: ResMut<RigEditorRuntime>) {
    runtime.request_model_refresh();
}

fn poll_runtime_messages(
    mut runtime: ResMut<RigEditorRuntime>,
    mut document: ResMut<GraphDocument>,
) {
    while let Ok(message) = runtime.rx.try_recv() {
        match message {
            RuntimeMessage::DiscoveryComplete {
                ready,
                detail,
                models,
            } => {
                runtime.ollama_ready = ready;
                runtime.ollama_detail = detail.clone();
                runtime.ollama_models = models;
                document.apply_ollama_models(&runtime.ollama_models);
                runtime.last_status = if runtime.ollama_ready {
                    format!(
                        "Ollama ready at {} with {} discovered model(s).",
                        runtime.ollama_endpoint,
                        runtime.ollama_models.len()
                    )
                } else {
                    format!("Ollama unavailable: {detail}")
                };
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

async fn execute_ollama_request(
    request_id: u64,
    request: CompiledAgentRun,
) -> Result<RuntimeMessage> {
    let client = ollama::Client::builder()
        .api_key(Nothing)
        .base_url(&request.endpoint)
        .build()
        .map_err(|error| anyhow!(error.to_string()))?;
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
    if let Some(schema) = request.output_schema.clone() {
        builder = builder.output_schema_raw(schema);
    }
    if request.tool_choice.is_some() {
        // Ollama does not support forcing tool choice today.
    }

    let agent = builder.build();
    let mut prompt_request = agent.prompt(request.prompt.as_str());
    if let Some(max_turns) = request.default_max_turns {
        prompt_request = prompt_request.max_turns(max_turns);
    }
    let response = prompt_request
        .await
        .map_err(|error| anyhow!(error.to_string()))?;

    let status = if request.warnings.is_empty() {
        format!(
            "completed via Ollama / {}{}",
            request.model,
            request
                .agent_name
                .as_deref()
                .map(|name| format!(" as {}", name))
                .unwrap_or_default()
        )
    } else {
        format!(
            "completed via Ollama / {} (warnings: {})",
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
    let client = HttpClient::builder()
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
    client: &HttpClient,
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
