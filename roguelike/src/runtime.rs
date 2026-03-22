use std::{
    env,
    net::{TcpStream, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use reqwest::Client as HttpClient;
use rig::{
    client::{Nothing, ProviderClient},
    completion::Chat,
    message::Message,
    prelude::{CompletionClient, TypedPrompt},
    providers::{anthropic, gemini, llamafile, ollama, openai},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use crate::components::{Position, Speaker};

const OLLAMA_API_BASE_URL: &str = "http://localhost:11434";
const LLAMAFILE_API_BASE_URL: &str = "http://localhost:8080";
const OLLAMA_FALLBACK_MODEL: &str = "llama3.2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Ollama,
    Llamafile,
    OpenAi,
    Anthropic,
    Gemini,
}

#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub kind: ProviderKind,
    pub label: &'static str,
    pub default_model: String,
    pub detail: String,
    pub ready: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeMessage {
    pub speaker: Speaker,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct MovementCandidate {
    pub id: u16,
    pub position: Position,
    pub metadata: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MovePlannerBackend {
    StructuredOutput,
    HeuristicOnly,
}

#[derive(Debug, Clone)]
pub struct DispatchInfo {
    pub request_id: u64,
    pub provider_label: &'static str,
    pub model: String,
}

#[derive(Debug, Clone)]
struct ChatRequest {
    request_id: u64,
    entity: Entity,
    provider: ProviderKind,
    model: String,
    system_prompt: String,
    history: Vec<RuntimeMessage>,
    prompt: String,
}

#[derive(Debug, Clone)]
struct MoveRequest {
    request_id: u64,
    entity: Entity,
    provider: ProviderKind,
    model: String,
    system_prompt: String,
    context: String,
    candidates: Vec<MovementCandidate>,
}

#[derive(Debug, Clone)]
struct MoveChoice {
    candidate_id: u16,
    position: Position,
    reason: String,
    source: &'static str,
}

#[derive(Debug, Clone)]
struct MoveDecision {
    destination: Position,
    summary: String,
    trace: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Serialize)]
#[schemars(deny_unknown_fields)]
struct MoveDecisionProposal {
    candidate_id: u16,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RigResponse {
    ChatSuccess {
        request_id: u64,
        entity: Entity,
        content: String,
    },
    ChatFailure {
        request_id: u64,
        entity: Entity,
        error: String,
    },
    MoveSuccess {
        request_id: u64,
        entity: Entity,
        destination: Position,
        summary: String,
        trace: String,
    },
    MoveFailure {
        request_id: u64,
        entity: Entity,
        error: String,
        trace: String,
    },
}

#[derive(Resource)]
pub struct RigRuntime {
    runtime: Arc<Runtime>,
    tx: Sender<RigResponse>,
    rx: Receiver<RigResponse>,
    providers: Vec<ProviderEntry>,
    selected: usize,
    next_request_id: u64,
}

impl RigRuntime {
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let providers = build_provider_registry(&runtime);
        let selected = providers
            .iter()
            .position(|provider| provider.ready)
            .unwrap_or(0);
        let (tx, rx) = unbounded();
        Self {
            runtime: Arc::new(runtime),
            tx,
            rx,
            providers,
            selected,
            next_request_id: 1,
        }
    }

    pub fn current_provider(&self) -> &ProviderEntry {
        &self.providers[self.selected]
    }

    pub fn providers(&self) -> &[ProviderEntry] {
        &self.providers
    }

    pub fn cycle_provider(&mut self, delta: i32) {
        if self.providers.is_empty() {
            return;
        }

        let len = self.providers.len() as i32;
        let mut next = self.selected as i32 + delta;
        if next < 0 {
            next += len;
        }
        self.selected = (next % len) as usize;
        self.refresh_selected_provider();
    }

    pub fn spawn_chat(
        &mut self,
        entity: Entity,
        system_prompt: String,
        preferred_model: Option<&str>,
        history: Vec<RuntimeMessage>,
        prompt: String,
    ) -> Option<DispatchInfo> {
        self.refresh_selected_provider();
        let provider = self.current_provider().clone();
        if !provider.ready {
            return None;
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let model = preferred_model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(provider.default_model.as_str())
            .to_string();

        let tx = self.tx.clone();
        let request = ChatRequest {
            request_id,
            entity,
            provider: provider.kind,
            model: model.clone(),
            system_prompt,
            history,
            prompt,
        };

        self.runtime.spawn(async move {
            let response = execute_chat(request.clone())
                .await
                .map(|content| RigResponse::ChatSuccess {
                    request_id: request.request_id,
                    entity: request.entity,
                    content,
                })
                .unwrap_or_else(|error| RigResponse::ChatFailure {
                    request_id: request.request_id,
                    entity: request.entity,
                    error: error.to_string(),
                });
            let _ = tx.send(response);
        });

        Some(DispatchInfo {
            request_id,
            provider_label: provider.label,
            model,
        })
    }

    pub fn spawn_move_decision(
        &mut self,
        entity: Entity,
        system_prompt: String,
        preferred_model: Option<&str>,
        context: String,
        candidates: Vec<MovementCandidate>,
    ) -> Option<DispatchInfo> {
        if candidates.is_empty() {
            return None;
        }

        self.refresh_selected_provider();
        let provider = self.current_provider().clone();
        if !provider.ready
            || provider.kind.move_planner_backend() == MovePlannerBackend::HeuristicOnly
        {
            return None;
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let model = preferred_model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(provider.default_model.as_str())
            .to_string();

        let request = MoveRequest {
            request_id,
            entity,
            provider: provider.kind,
            model: model.clone(),
            system_prompt,
            context,
            candidates,
        };
        let tx = self.tx.clone();

        self.runtime.spawn(async move {
            let response = execute_move_decision(request.clone())
                .await
                .map(|decision| RigResponse::MoveSuccess {
                    request_id: request.request_id,
                    entity: request.entity,
                    destination: decision.destination,
                    summary: decision.summary,
                    trace: decision.trace,
                })
                .unwrap_or_else(|error| RigResponse::MoveFailure {
                    request_id: request.request_id,
                    entity: request.entity,
                    error: error.to_string(),
                    trace: build_move_failure_trace(&request, &error.to_string()),
                });
            let _ = tx.send(response);
        });

        Some(DispatchInfo {
            request_id,
            provider_label: provider.label,
            model,
        })
    }

    pub fn try_recv(&self) -> Option<RigResponse> {
        self.rx.try_recv().ok()
    }

    fn refresh_selected_provider(&mut self) {
        let Some(kind) = self
            .providers
            .get(self.selected)
            .map(|provider| provider.kind)
        else {
            return;
        };

        self.providers[self.selected] = build_provider_entry(self.runtime.as_ref(), kind);
    }
}

async fn execute_chat(request: ChatRequest) -> Result<String> {
    match request.provider {
        ProviderKind::Ollama => run_with_client(ollama::Client::from_val(Nothing), &request).await,
        ProviderKind::Llamafile => {
            run_with_client(llamafile::Client::from_val(Nothing), &request).await
        }
        ProviderKind::OpenAi => run_with_client(openai::Client::from_env(), &request).await,
        ProviderKind::Anthropic => run_with_client(anthropic::Client::from_env(), &request).await,
        ProviderKind::Gemini => run_with_client(gemini::Client::from_env(), &request).await,
    }
}

async fn execute_move_decision(request: MoveRequest) -> Result<MoveDecision> {
    match request.provider {
        ProviderKind::Ollama => {
            run_structured_move_decision_with_client(ollama::Client::from_val(Nothing), &request)
                .await
        }
        ProviderKind::Llamafile => Err(anyhow!(
            "move planner backend {} is unavailable for {}",
            MovePlannerBackend::HeuristicOnly.label(),
            request.provider.label()
        )),
        ProviderKind::OpenAi => {
            run_structured_move_decision_with_client(openai::Client::from_env(), &request).await
        }
        ProviderKind::Anthropic => {
            run_structured_move_decision_with_client(anthropic::Client::from_env(), &request).await
        }
        ProviderKind::Gemini => {
            run_structured_move_decision_with_client(gemini::Client::from_env(), &request).await
        }
    }
}

async fn run_with_client<C>(client: C, request: &ChatRequest) -> Result<String>
where
    C: CompletionClient,
{
    let agent = client
        .agent(request.model.clone())
        .preamble(&request.system_prompt)
        .build();

    let history = request
        .history
        .iter()
        .map(runtime_message_to_rig)
        .collect::<Vec<_>>();

    agent
        .chat(request.prompt.clone(), history)
        .await
        .map_err(|error| anyhow!(error.to_string()))
}

async fn run_structured_move_decision_with_client<C>(
    client: C,
    request: &MoveRequest,
) -> Result<MoveDecision>
where
    C: CompletionClient,
{
    let agent = client
        .agent(request.model.clone())
        .preamble(&request.system_prompt)
        .build();
    let mut repair_note = None;

    for attempt in 1..=2 {
        let proposal: MoveDecisionProposal = match agent
            .prompt_typed(build_move_prompt(request, repair_note.as_deref()))
            .await
        {
            Ok(proposal) => proposal,
            Err(error) => {
                let failure = format!(
                    "structured output attempt {} failed: {}",
                    attempt,
                    normalize_runtime_text(&error.to_string())
                );
                if attempt == 2 {
                    return Err(anyhow!(failure));
                }
                repair_note = Some(failure);
                continue;
            }
        };

        match validate_move_proposal(request, &proposal, attempt) {
            Ok(choice) => {
                return Ok(MoveDecision {
                    destination: choice.position,
                    summary: summarize_move_decision(&choice),
                    trace: build_move_success_trace(request, &choice, attempt),
                });
            }
            Err(validation_error) => {
                if attempt == 2 {
                    return Err(anyhow!(validation_error));
                }
                repair_note = Some(validation_error);
            }
        }
    }

    Err(anyhow!(
        "movement planner exhausted structured decision retries"
    ))
}

fn runtime_message_to_rig(message: &RuntimeMessage) -> Message {
    match message.speaker {
        Speaker::Player => Message::user(message.content.clone()),
        Speaker::Npc => Message::assistant(message.content.clone()),
    }
}

fn build_move_prompt(request: &MoveRequest, repair_note: Option<&str>) -> String {
    let mut prompt = String::from(
        "Return a structured movement decision by choosing exactly one legal candidate_id.\n\
         Rules:\n\
         - candidate_id must match one of the listed legal candidates.\n\
         - Choose the current tile if staying put is best.\n\
         - reason is optional and should be brief and in character.\n\
         - Never mention tools, schemas, debugging, or coordinate repair.\n",
    );

    if let Some(note) = repair_note {
        prompt.push_str("\nPrevious attempt was invalid:\n- ");
        prompt.push_str(note);
        prompt.push('\n');
    }

    prompt.push('\n');
    prompt.push_str(&request.context);
    prompt.push_str("\n\nLegal candidates:\n");

    for candidate in &request.candidates {
        prompt.push_str(&format!("- id={} {}\n", candidate.id, candidate.metadata));
    }

    prompt
}

fn validate_move_proposal(
    request: &MoveRequest,
    proposal: &MoveDecisionProposal,
    attempt: usize,
) -> std::result::Result<MoveChoice, String> {
    let Some(candidate) = request
        .candidates
        .iter()
        .find(|candidate| candidate.id == proposal.candidate_id)
    else {
        let valid_ids = request
            .candidates
            .iter()
            .map(|candidate| candidate.id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "attempt {} proposed invalid candidate_id {}; valid ids are [{}]",
            attempt, proposal.candidate_id, valid_ids
        ));
    };

    Ok(MoveChoice {
        candidate_id: candidate.id,
        position: candidate.position,
        reason: sanitize_reason(proposal.reason.as_deref()),
        source: if attempt == 1 {
            "structured_output"
        } else {
            "structured_output_repair"
        },
    })
}

fn summarize_move_decision(choice: &MoveChoice) -> String {
    format!(
        "heading to ({}, {}) because {}",
        choice.position.x, choice.position.y, choice.reason
    )
}

fn sanitize_reason(reason: Option<&str>) -> String {
    let trimmed = reason.unwrap_or_default().trim();
    if trimmed.is_empty() {
        "it suits the moment".to_string()
    } else {
        normalize_runtime_text(trimmed)
    }
}

fn normalize_runtime_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_move_success_trace(request: &MoveRequest, choice: &MoveChoice, attempts: usize) -> String {
    format!(
        "req#{} provider={} model={} backend={} attempts={} candidates={} source={} candidate_id={} x={} y={} reason={}",
        request.request_id,
        request.provider.label(),
        request.model,
        request.provider.move_planner_backend().label(),
        attempts,
        request.candidates.len(),
        choice.source,
        choice.candidate_id,
        choice.position.x,
        choice.position.y,
        choice.reason
    )
}

fn build_move_failure_trace(request: &MoveRequest, error: &str) -> String {
    format!(
        "req#{} provider={} model={} backend={} candidates={} failure={}",
        request.request_id,
        request.provider.label(),
        request.model,
        request.provider.move_planner_backend().label(),
        request.candidates.len(),
        normalize_runtime_text(error)
    )
}

fn build_provider_registry(runtime: &Runtime) -> Vec<ProviderEntry> {
    [
        build_provider_entry(runtime, ProviderKind::Ollama),
        build_provider_entry(runtime, ProviderKind::Llamafile),
        build_provider_entry(runtime, ProviderKind::OpenAi),
        build_provider_entry(runtime, ProviderKind::Anthropic),
        build_provider_entry(runtime, ProviderKind::Gemini),
    ]
    .into_iter()
    .collect()
}

fn build_provider_entry(runtime: &Runtime, kind: ProviderKind) -> ProviderEntry {
    match kind {
        ProviderKind::Ollama => build_ollama_entry(runtime),
        ProviderKind::Llamafile => {
            let endpoint = env_base_url("LLAMAFILE_API_BASE_URL", LLAMAFILE_API_BASE_URL);
            ProviderEntry {
                kind,
                label: "Llamafile",
                default_model: "LLaMA_CPP".to_string(),
                detail: endpoint.clone(),
                ready: endpoint_reachable(&endpoint),
            }
        }
        ProviderKind::OpenAi => ProviderEntry {
            kind,
            label: "OpenAI",
            default_model: "gpt-4o-mini".to_string(),
            detail: "OPENAI_API_KEY".to_string(),
            ready: env::var_os("OPENAI_API_KEY").is_some(),
        },
        ProviderKind::Anthropic => ProviderEntry {
            kind,
            label: "Anthropic",
            default_model: "claude-sonnet-4-5".to_string(),
            detail: "ANTHROPIC_API_KEY".to_string(),
            ready: env::var_os("ANTHROPIC_API_KEY").is_some(),
        },
        ProviderKind::Gemini => ProviderEntry {
            kind,
            label: "Gemini",
            default_model: "gemini-2.5-flash".to_string(),
            detail: "GEMINI_API_KEY".to_string(),
            ready: env::var_os("GEMINI_API_KEY").is_some(),
        },
    }
}

fn build_ollama_entry(runtime: &Runtime) -> ProviderEntry {
    let endpoint = env_base_url("OLLAMA_API_BASE_URL", OLLAMA_API_BASE_URL);
    let ready = endpoint_reachable(&endpoint);
    let detection = if ready {
        runtime
            .block_on(detect_ollama_model(&endpoint))
            .ok()
            .flatten()
    } else {
        None
    };

    let (default_model, detail) = if let Some(detection) = detection {
        (
            detection.model,
            format!("{endpoint} ({})", detection.source.label()),
        )
    } else {
        (OLLAMA_FALLBACK_MODEL.to_string(), endpoint)
    };

    ProviderEntry {
        kind: ProviderKind::Ollama,
        label: "Ollama",
        default_model,
        detail,
        ready,
    }
}

fn env_base_url(key: &str, fallback: &str) -> String {
    env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

#[derive(Debug, Clone, Copy)]
enum OllamaModelSource {
    Running,
    Installed,
}

impl OllamaModelSource {
    fn label(self) -> &'static str {
        match self {
            Self::Running => "running model",
            Self::Installed => "installed model",
        }
    }
}

impl MovePlannerBackend {
    fn label(self) -> &'static str {
        match self {
            Self::StructuredOutput => "structured_output",
            Self::HeuristicOnly => "heuristic_only",
        }
    }
}

impl ProviderKind {
    fn move_planner_backend(self) -> MovePlannerBackend {
        match self {
            Self::Llamafile => MovePlannerBackend::HeuristicOnly,
            Self::Ollama | Self::OpenAi | Self::Anthropic | Self::Gemini => {
                MovePlannerBackend::StructuredOutput
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::Llamafile => "Llamafile",
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
        }
    }
}

#[derive(Debug, Clone)]
struct OllamaModelDetection {
    model: String,
    source: OllamaModelSource,
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

async fn detect_ollama_model(endpoint: &str) -> Result<Option<OllamaModelDetection>> {
    let client = HttpClient::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .map_err(|error| anyhow!(error.to_string()))?;

    if let Some(model) = detect_ollama_running_model(&client, endpoint).await? {
        return Ok(Some(OllamaModelDetection {
            model,
            source: OllamaModelSource::Running,
        }));
    }

    if let Some(model) = detect_ollama_installed_model(&client, endpoint).await? {
        return Ok(Some(OllamaModelDetection {
            model,
            source: OllamaModelSource::Installed,
        }));
    }

    Ok(None)
}

async fn detect_ollama_running_model(
    client: &HttpClient,
    endpoint: &str,
) -> Result<Option<String>> {
    let models = fetch_ollama_models(client, endpoint, "api/ps").await?;
    Ok(choose_preferred_ollama_model(&models))
}

async fn detect_ollama_installed_model(
    client: &HttpClient,
    endpoint: &str,
) -> Result<Option<String>> {
    let models = fetch_ollama_models(client, endpoint, "api/tags").await?;
    Ok(choose_preferred_ollama_model(&models))
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

fn choose_preferred_ollama_model(models: &[String]) -> Option<String> {
    models
        .iter()
        .find(|model| looks_like_chat_model(model))
        .cloned()
        .or_else(|| models.first().cloned())
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

fn endpoint_reachable(endpoint: &str) -> bool {
    let Some((host, port)) = parse_endpoint_socket(endpoint) else {
        return false;
    };

    let Ok(addrs) = (host.as_str(), port).to_socket_addrs() else {
        return false;
    };

    addrs
        .into_iter()
        .any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::{
        MoveDecisionProposal, MoveRequest, MovementCandidate, OllamaModelRecord, ProviderKind,
        choose_preferred_ollama_model, looks_like_chat_model, ollama_model_name,
        validate_move_proposal,
    };
    use crate::components::Position;
    use bevy::prelude::Entity;

    #[test]
    fn prefers_chat_models_over_embeddings() {
        let models = vec![
            "nomic-embed-text".to_string(),
            "llama3.2".to_string(),
            "bge-m3".to_string(),
        ];

        assert_eq!(
            choose_preferred_ollama_model(&models).as_deref(),
            Some("llama3.2")
        );
    }

    #[test]
    fn falls_back_to_first_model_when_no_chat_model_matches() {
        let models = vec!["nomic-embed-text".to_string(), "bge-m3".to_string()];

        assert_eq!(
            choose_preferred_ollama_model(&models).as_deref(),
            Some("nomic-embed-text")
        );
    }

    #[test]
    fn extracts_name_then_model_fields() {
        assert_eq!(
            ollama_model_name(OllamaModelRecord {
                name: Some("llama3.2".to_string()),
                model: Some("ignored".to_string()),
            })
            .as_deref(),
            Some("llama3.2")
        );
        assert_eq!(
            ollama_model_name(OllamaModelRecord {
                name: None,
                model: Some("qwen3:8b".to_string()),
            })
            .as_deref(),
            Some("qwen3:8b")
        );
    }

    #[test]
    fn embedding_models_are_not_treated_as_chat_models() {
        assert!(!looks_like_chat_model("nomic-embed-text"));
        assert!(!looks_like_chat_model("bge-m3"));
        assert!(looks_like_chat_model("qwen3:8b"));
    }

    #[test]
    fn validates_structured_candidate_ids() {
        let request = MoveRequest {
            request_id: 7,
            entity: Entity::PLACEHOLDER,
            provider: ProviderKind::Ollama,
            model: "llama3.2".to_string(),
            system_prompt: "system".to_string(),
            context: "context".to_string(),
            candidates: vec![MovementCandidate {
                id: 3,
                position: Position::new(41, 15),
                metadata: "tile=(41,15)".to_string(),
            }],
        };

        let choice = validate_move_proposal(
            &request,
            &MoveDecisionProposal {
                candidate_id: 3,
                reason: Some("heading home".to_string()),
            },
            1,
        )
        .expect("proposal should validate");

        assert_eq!(choice.candidate_id, 3);
        assert_eq!(choice.position, Position::new(41, 15));
        assert_eq!(choice.source, "structured_output");
    }

    #[test]
    fn rejects_unknown_structured_candidate_ids() {
        let request = MoveRequest {
            request_id: 8,
            entity: Entity::PLACEHOLDER,
            provider: ProviderKind::Ollama,
            model: "llama3.2".to_string(),
            system_prompt: "system".to_string(),
            context: "context".to_string(),
            candidates: vec![
                MovementCandidate {
                    id: 0,
                    position: Position::new(26, 10),
                    metadata: "tile=(26,10)".to_string(),
                },
                MovementCandidate {
                    id: 1,
                    position: Position::new(26, 11),
                    metadata: "tile=(26,11)".to_string(),
                },
            ],
        };

        let error = validate_move_proposal(
            &request,
            &MoveDecisionProposal {
                candidate_id: 9,
                reason: None,
            },
            2,
        )
        .expect_err("candidate id should be rejected");

        assert!(error.contains("invalid candidate_id 9"));
        assert!(error.contains("[0, 1]"));
    }
}

fn parse_endpoint_socket(endpoint: &str) -> Option<(String, u16)> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return None;
    }

    let default_port = if trimmed.starts_with("https://") {
        443
    } else {
        80
    };

    let authority = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed)
        .split('/')
        .next()
        .unwrap_or_default()
        .trim();

    if authority.is_empty() {
        return None;
    }

    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(default_port);
        return Some((host.to_string(), port));
    }

    if let Some((host, port)) = authority.rsplit_once(':') {
        if let Ok(parsed) = port.parse::<u16>() {
            return Some((host.to_string(), parsed));
        }
    }

    Some((authority.to_string(), default_port))
}
