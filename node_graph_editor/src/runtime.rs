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

use crate::graph::{GraphEditorState, NodeId, NodeKind, PortType, ToolChoiceSetting};

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
    pub last_status: String,
    next_request_id: u64,
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
            last_status: "Local Ollama discovery pending.".into(),
            next_request_id: 1,
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
        self.last_status = format!(
            "Running {} on Ollama / {} …",
            request.agent_name.as_deref().unwrap_or("selected agent"),
            request.model
        );

        let tx = self.tx.clone();
        let runtime = self.runtime.clone();
        runtime.spawn(async move {
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

        Ok(())
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
    mut graph: ResMut<GraphEditorState>,
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
                graph.apply_ollama_models(&runtime.ollama_models);
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
                runtime.last_status = format!("Run #{request_id} finished.");
                graph.set_output_result(output_node, text, status);
            }
            RuntimeMessage::RunFailed {
                request_id,
                output_node,
                error,
            } => {
                runtime.pending_request = None;
                runtime.last_status = format!("Run #{request_id} failed: {error}");
                if let Some(output_node) = output_node {
                    graph.set_output_result(output_node, error.clone(), "failed".into());
                }
            }
        }
    }
}

pub fn compile_selected_agent_run(
    graph: &GraphEditorState,
    runtime: &RigEditorRuntime,
) -> Result<CompiledAgentRun> {
    let agent_id = graph
        .selected_node
        .filter(|node_id| matches!(graph.node_kind(*node_id), Some(NodeKind::Agent)))
        .or_else(|| {
            graph.nodes.iter().find_map(|node| match node.kind {
                NodeKind::Agent => Some(node.id),
                _ => None,
            })
        })
        .ok_or_else(|| anyhow!("select an Agent node before running the graph"))?;

    let output_node = graph
        .output_targets(agent_id, PortType::TextResponse)
        .into_iter()
        .find(|node_id| matches!(graph.node_kind(*node_id), Some(NodeKind::TextOutput { .. })))
        .ok_or_else(|| anyhow!("connect the Agent output to a Text Output node"))?;

    let model_node = required_source(graph, agent_id, PortType::Model, "model")?;
    let model = match graph.node_kind(model_node) {
        Some(NodeKind::Model { model_name, .. }) => model_name
            .clone()
            .ok_or_else(|| anyhow!("select a local Ollama model inside the Model node"))?,
        _ => return Err(anyhow!("the Agent model input must come from a Model node")),
    };

    let prompt_node = required_source(graph, agent_id, PortType::Prompt, "prompt")?;
    let prompt = match graph.node_kind(prompt_node) {
        Some(NodeKind::PromptInput { text }) if !text.trim().is_empty() => text.clone(),
        Some(NodeKind::PromptInput { .. }) => {
            return Err(anyhow!("the Prompt node is empty"));
        }
        _ => {
            return Err(anyhow!(
                "the Agent prompt input must come from a Prompt node"
            ));
        }
    };

    let agent_name = optional_text_source(graph, agent_id, PortType::AgentName)?;
    let description = optional_text_source(graph, agent_id, PortType::AgentDescription)?;
    let preamble = optional_text_source(graph, agent_id, PortType::Preamble)?;
    let static_context = multi_text_sources(graph, agent_id, PortType::StaticContext)?;
    let temperature = optional_temperature_source(graph, agent_id)?;
    let max_tokens = optional_max_tokens_source(graph, agent_id)?;
    let additional_params = optional_json_source(graph, agent_id)?;
    let tool_choice = optional_tool_choice_source(graph, agent_id)?;
    let default_max_turns = optional_default_max_turns_source(graph, agent_id)?;
    let output_schema = optional_schema_source(graph, agent_id)?;

    let mut warnings = Vec::new();
    if !graph
        .input_sources(agent_id, PortType::ToolServerHandle)
        .is_empty()
    {
        warnings.push("tool_server_handle nodes are stored but not executable in this MVP".into());
    }
    if !graph
        .input_sources(agent_id, PortType::DynamicContext)
        .is_empty()
    {
        warnings.push("dynamic_context nodes are stored but not executable in this MVP".into());
    }
    if !graph.input_sources(agent_id, PortType::Hook).is_empty() {
        warnings.push("hook nodes are stored but not executable in this MVP".into());
    }
    if tool_choice.is_some() {
        warnings
            .push("Ollama currently ignores tool_choice; it will be dropped for this run".into());
    }
    if !runtime.ollama_ready {
        return Err(anyhow!(
            "local Ollama is not reachable at {}",
            runtime.ollama_endpoint
        ));
    }

    Ok(CompiledAgentRun {
        agent_id,
        output_node,
        endpoint: runtime.ollama_endpoint.clone(),
        model,
        prompt,
        agent_name,
        description,
        preamble,
        static_context,
        temperature,
        max_tokens,
        additional_params,
        tool_choice,
        default_max_turns,
        output_schema,
        warnings,
    })
}

fn required_source(
    graph: &GraphEditorState,
    agent_id: NodeId,
    ty: PortType,
    label: &str,
) -> Result<NodeId> {
    graph
        .input_sources(agent_id, ty)
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("Agent is missing a required {label} node"))
}

fn optional_text_source(
    graph: &GraphEditorState,
    agent_id: NodeId,
    ty: PortType,
) -> Result<Option<String>> {
    let Some(source) = graph.input_sources(agent_id, ty).into_iter().next() else {
        return Ok(None);
    };

    let value = match graph.node_kind(source) {
        Some(NodeKind::Name { value })
        | Some(NodeKind::Description { value })
        | Some(NodeKind::Preamble { value }) => value.trim().to_string(),
        _ => {
            return Err(anyhow!(
                "connected node type does not match expected text field"
            ));
        }
    };

    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn multi_text_sources(
    graph: &GraphEditorState,
    agent_id: NodeId,
    ty: PortType,
) -> Result<Vec<String>> {
    let mut values = Vec::new();
    for source in graph.input_sources(agent_id, ty) {
        match graph.node_kind(source) {
            Some(NodeKind::StaticContext { value }) if !value.trim().is_empty() => {
                values.push(value.trim().to_string());
            }
            Some(NodeKind::StaticContext { .. }) => {}
            _ => return Err(anyhow!("connected node type does not match static context")),
        }
    }
    Ok(values)
}

fn optional_temperature_source(graph: &GraphEditorState, agent_id: NodeId) -> Result<Option<f64>> {
    let Some(source) = graph
        .input_sources(agent_id, PortType::Temperature)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match graph.node_kind(source) {
        Some(NodeKind::Temperature { value }) => Ok(Some(*value)),
        _ => Err(anyhow!(
            "temperature input must come from a Temperature node"
        )),
    }
}

fn optional_max_tokens_source(graph: &GraphEditorState, agent_id: NodeId) -> Result<Option<u64>> {
    let Some(source) = graph
        .input_sources(agent_id, PortType::MaxTokens)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match graph.node_kind(source) {
        Some(NodeKind::MaxTokens { value }) => Ok(Some(*value)),
        _ => Err(anyhow!("max_tokens input must come from a Max Tokens node")),
    }
}

fn optional_json_source(graph: &GraphEditorState, agent_id: NodeId) -> Result<Option<Value>> {
    let Some(source) = graph
        .input_sources(agent_id, PortType::AdditionalParams)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match graph.node_kind(source) {
        Some(NodeKind::AdditionalParams { value }) => {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                let parsed = serde_json::from_str(value)
                    .map_err(|error| anyhow!("Additional Params is not valid JSON: {error}"))?;
                Ok(Some(parsed))
            }
        }
        _ => Err(anyhow!(
            "additional_params input must come from an Additional Params node"
        )),
    }
}

fn optional_tool_choice_source(
    graph: &GraphEditorState,
    agent_id: NodeId,
) -> Result<Option<ToolChoiceSetting>> {
    let Some(source) = graph
        .input_sources(agent_id, PortType::ToolChoice)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match graph.node_kind(source) {
        Some(NodeKind::ToolChoice { value }) => Ok(Some(*value)),
        _ => Err(anyhow!(
            "tool_choice input must come from a Tool Choice node"
        )),
    }
}

fn optional_default_max_turns_source(
    graph: &GraphEditorState,
    agent_id: NodeId,
) -> Result<Option<usize>> {
    let Some(source) = graph
        .input_sources(agent_id, PortType::DefaultMaxTurns)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match graph.node_kind(source) {
        Some(NodeKind::DefaultMaxTurns { value }) => Ok(Some(*value)),
        _ => Err(anyhow!(
            "default_max_turns input must come from a Default Max Turns node"
        )),
    }
}

fn optional_schema_source(graph: &GraphEditorState, agent_id: NodeId) -> Result<Option<Schema>> {
    let Some(source) = graph
        .input_sources(agent_id, PortType::OutputSchema)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    match graph.node_kind(source) {
        Some(NodeKind::OutputSchema { value }) => {
            if value.trim().is_empty() {
                Ok(None)
            } else {
                let parsed = serde_json::from_str(value)
                    .map_err(|error| anyhow!("Output Schema is not valid JSON: {error}"))?;
                Ok(Some(parsed))
            }
        }
        _ => Err(anyhow!(
            "output_schema input must come from an Output Schema node"
        )),
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
