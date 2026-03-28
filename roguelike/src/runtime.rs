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
    prelude::{CompletionClient, TypedPrompt},
    providers::{anthropic, gemini, llamafile, ollama, openai},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Runtime;

use crate::{
    components::{Position, Speaker},
    map::PropKind,
};

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

#[derive(Debug, Clone)]
pub struct DrinkCandidate {
    pub id: u16,
    pub position: Position,
    pub prop: PropKind,
    pub metadata: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionPlannerBackend {
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

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ChatReplyAction {
    Speak,
    StaySilent,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Serialize)]
#[schemars(deny_unknown_fields)]
struct ChatReplyProposal {
    action: ChatReplyAction,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Clone)]
struct ActionRequest {
    request_id: u64,
    entity: Entity,
    provider: ProviderKind,
    model: String,
    system_prompt: String,
    context: String,
    move_candidates: Vec<MovementCandidate>,
    drink_candidates: Vec<DrinkCandidate>,
}

#[derive(Debug, Clone)]
struct MoveChoice {
    candidate_id: u16,
    position: Position,
    reason: String,
    source: &'static str,
}

#[derive(Debug, Clone)]
struct DrinkChoice {
    candidate_id: u16,
    position: Position,
    prop: PropKind,
    reason: String,
    source: &'static str,
}

#[derive(Debug, Clone)]
enum ActionChoice {
    Move(MoveChoice),
    Speak {
        text: String,
        reason: String,
        source: &'static str,
    },
    Drink(DrinkChoice),
    Idle {
        reason: String,
        source: &'static str,
    },
}

#[derive(Debug, Clone)]
struct ActionDecision {
    outcome: NpcActionOutcome,
    trace: String,
}

#[derive(Debug, Clone)]
pub enum NpcActionOutcome {
    Move {
        destination: Position,
        summary: String,
    },
    Speak {
        text: String,
    },
    Drink {
        position: Position,
        summary: String,
    },
    Idle {
        summary: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NpcActionKind {
    PathfindSomewhere,
    Speak,
    DrinkBeer,
    DoNothing,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, Serialize)]
#[schemars(deny_unknown_fields)]
struct NpcActionProposal {
    action: NpcActionKind,
    #[serde(default)]
    move_candidate_id: Option<u16>,
    #[serde(default)]
    drink_candidate_id: Option<u16>,
    #[serde(default)]
    text: Option<String>,
    reason: String,
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
    ActionSuccess {
        request_id: u64,
        entity: Entity,
        outcome: NpcActionOutcome,
        trace: String,
    },
    ActionFailure {
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

    pub fn spawn_action_decision(
        &mut self,
        entity: Entity,
        system_prompt: String,
        preferred_model: Option<&str>,
        context: String,
        move_candidates: Vec<MovementCandidate>,
        drink_candidates: Vec<DrinkCandidate>,
    ) -> Option<DispatchInfo> {
        self.refresh_selected_provider();
        let provider = self.current_provider().clone();
        if !provider.ready
            || provider.kind.action_planner_backend() == ActionPlannerBackend::HeuristicOnly
        {
            return None;
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let model = preferred_model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(provider.default_model.as_str())
            .to_string();

        let request = ActionRequest {
            request_id,
            entity,
            provider: provider.kind,
            model: model.clone(),
            system_prompt,
            context,
            move_candidates,
            drink_candidates,
        };
        let tx = self.tx.clone();

        self.runtime.spawn(async move {
            let response = execute_action_decision(request.clone())
                .await
                .map(|decision| RigResponse::ActionSuccess {
                    request_id: request.request_id,
                    entity: request.entity,
                    outcome: decision.outcome,
                    trace: decision.trace,
                })
                .unwrap_or_else(|error| RigResponse::ActionFailure {
                    request_id: request.request_id,
                    entity: request.entity,
                    error: error.to_string(),
                    trace: build_action_failure_trace(&request, &error.to_string()),
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
        ProviderKind::Ollama => {
            run_structured_chat_with_ollama(ollama::Client::from_val(Nothing), &request).await
        }
        ProviderKind::Llamafile => {
            run_structured_chat_with_client(llamafile::Client::from_val(Nothing), &request).await
        }
        ProviderKind::OpenAi => {
            run_structured_chat_with_client(openai::Client::from_env(), &request).await
        }
        ProviderKind::Anthropic => {
            run_structured_chat_with_client(anthropic::Client::from_env(), &request).await
        }
        ProviderKind::Gemini => {
            run_structured_chat_with_client(gemini::Client::from_env(), &request).await
        }
    }
}

async fn execute_action_decision(request: ActionRequest) -> Result<ActionDecision> {
    match request.provider {
        ProviderKind::Ollama => {
            run_structured_action_decision_with_ollama(ollama::Client::from_val(Nothing), &request)
                .await
        }
        ProviderKind::Llamafile => Err(anyhow!(
            "action planner backend {} is unavailable for {}",
            ActionPlannerBackend::HeuristicOnly.label(),
            request.provider.label()
        )),
        ProviderKind::OpenAi => {
            run_structured_action_decision_with_client(openai::Client::from_env(), &request).await
        }
        ProviderKind::Anthropic => {
            run_structured_action_decision_with_client(anthropic::Client::from_env(), &request)
                .await
        }
        ProviderKind::Gemini => {
            run_structured_action_decision_with_client(gemini::Client::from_env(), &request).await
        }
    }
}

async fn run_structured_chat_with_client<C>(client: C, request: &ChatRequest) -> Result<String>
where
    C: CompletionClient,
{
    let agent = client
        .agent(request.model.clone())
        .preamble(&request.system_prompt)
        .build();
    run_structured_chat_prompt(&agent, request).await
}

async fn run_structured_chat_with_ollama(
    client: ollama::Client,
    request: &ChatRequest,
) -> Result<String> {
    let agent = client
        .agent(request.model.clone())
        .preamble(&request.system_prompt)
        .additional_params(json!({ "think": false }))
        .build();
    run_structured_chat_prompt(&agent, request).await
}

async fn run_structured_chat_prompt<A>(agent: &A, request: &ChatRequest) -> Result<String>
where
    A: TypedPrompt,
{
    let mut repair_note = None;

    for attempt in 1..=2 {
        let proposal: ChatReplyProposal = match agent
            .prompt_typed(build_chat_prompt(request, repair_note.as_deref()))
            .await
        {
            Ok(proposal) => proposal,
            Err(error) => {
                let failure = format!(
                    "structured chat attempt {} failed: {}",
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

        match validate_chat_proposal(&proposal, attempt) {
            Ok(text) => return Ok(text),
            Err(validation_error) => {
                if attempt == 2 {
                    return Err(anyhow!(validation_error));
                }
                repair_note = Some(validation_error);
            }
        }
    }

    Err(anyhow!("chat planner exhausted structured reply retries"))
}

async fn run_structured_action_decision_with_client<C>(
    client: C,
    request: &ActionRequest,
) -> Result<ActionDecision>
where
    C: CompletionClient,
{
    let agent = client
        .agent(request.model.clone())
        .preamble(&request.system_prompt)
        .build();
    let mut repair_note = None;

    for attempt in 1..=2 {
        let proposal: NpcActionProposal = match agent
            .prompt_typed(build_action_prompt(request, repair_note.as_deref()))
            .await
        {
            Ok(proposal) => proposal,
            Err(error) => {
                let failure = format!(
                    "structured action attempt {} failed: {}",
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

        match validate_action_proposal(request, &proposal, attempt) {
            Ok(choice) => return Ok(build_action_decision(request, choice, attempt)),
            Err(validation_error) => {
                if attempt == 2 {
                    return Err(anyhow!(validation_error));
                }
                repair_note = Some(validation_error);
            }
        }
    }

    Err(anyhow!(
        "npc action planner exhausted structured decision retries"
    ))
}

async fn run_structured_action_decision_with_ollama(
    client: ollama::Client,
    request: &ActionRequest,
) -> Result<ActionDecision> {
    let agent = client
        .agent(request.model.clone())
        .preamble(&request.system_prompt)
        .additional_params(json!({ "think": false }))
        .build();
    let mut repair_note = None;

    for attempt in 1..=2 {
        let proposal: NpcActionProposal = match agent
            .prompt_typed(build_action_prompt(request, repair_note.as_deref()))
            .await
        {
            Ok(proposal) => proposal,
            Err(error) => {
                let failure = format!(
                    "structured action attempt {} failed: {}",
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

        match validate_action_proposal(request, &proposal, attempt) {
            Ok(choice) => return Ok(build_action_decision(request, choice, attempt)),
            Err(validation_error) => {
                if attempt == 2 {
                    return Err(anyhow!(validation_error));
                }
                repair_note = Some(validation_error);
            }
        }
    }

    Err(anyhow!(
        "npc action planner exhausted structured decision retries"
    ))
}

fn build_action_prompt(request: &ActionRequest, repair_note: Option<&str>) -> String {
    let mut prompt = String::from(
        "Return a structured NPC action decision.\n\
         Rules:\n\
         - action must be exactly one of: pathfind_somewhere, speak, drink_beer, do_nothing.\n\
         - If action=pathfind_somewhere, move_candidate_id must match one of the listed legal move candidates.\n\
         - Use pathfind_somewhere when you want to walk to an interesting visible place; the simulation will pathfind there with A*.\n\
         - If action=speak, text must contain only the exact short in-character words spoken aloud.\n\
         - If action=drink_beer, drink_candidate_id must match one of the listed legal drink candidates already within reach.\n\
         - If action=do_nothing, omit candidate ids and text.\n\
         - reason is required for every action and should be brief and in character.\n\
         - Never mention tools, schemas, debugging, hidden rules, or coordinate repair.\n\
         - Never narrate your reasoning.\n",
    );

    if let Some(note) = repair_note {
        prompt.push_str("\nPrevious attempt was invalid:\n- ");
        prompt.push_str(note);
        prompt.push('\n');
    }

    prompt.push('\n');
    prompt.push_str(&request.context);
    prompt.push_str("\n\nLegal move candidates:\n");
    if request.move_candidates.is_empty() {
        prompt.push_str("- none\n");
    } else {
        for candidate in &request.move_candidates {
            prompt.push_str(&format!("- id={} {}\n", candidate.id, candidate.metadata));
        }
    }

    prompt.push_str("\nLegal drink candidates:\n");
    if request.drink_candidates.is_empty() {
        prompt.push_str("- none\n");
    } else {
        for candidate in &request.drink_candidates {
            prompt.push_str(&format!("- id={} {}\n", candidate.id, candidate.metadata));
        }
    }

    prompt.push_str("\nReturn only the structured decision.");
    prompt
}

fn build_chat_prompt(request: &ChatRequest, repair_note: Option<&str>) -> String {
    let mut prompt = String::from(
        "Return a structured speech decision.\n\
         Rules:\n\
         - action must be either speak or stay_silent.\n\
         - If action=speak, text must contain only the exact short in-character words spoken.\n\
         - Use 1-3 short sentences.\n\
         - Do not narrate, explain, analyze, or mention the prompt, user, memory, tools, or hidden rules.\n\
         - Do not include speaker labels, quotation marks, stage directions, or thought process.\n\
         - If staying silent is best, use action=stay_silent and omit text.\n",
    );

    if let Some(note) = repair_note {
        prompt.push_str("\nPrevious attempt was invalid:\n- ");
        prompt.push_str(note);
        prompt.push('\n');
    }

    prompt.push_str("\nRecent conversation:\n");
    if request.history.is_empty() {
        prompt.push_str("- none\n");
    } else {
        for message in &request.history {
            let speaker = match message.speaker {
                Speaker::Player => "player",
                Speaker::Npc => "you",
            };
            prompt.push_str(&format!(
                "- {}: {}\n",
                speaker,
                normalize_runtime_text(&message.content)
            ));
        }
    }

    prompt.push_str("\nLatest line heard:\n");
    prompt.push_str(&request.prompt);
    prompt.push('\n');
    prompt.push_str("\nReturn only the structured decision.");
    prompt
}

fn validate_chat_proposal(
    proposal: &ChatReplyProposal,
    attempt: usize,
) -> std::result::Result<String, String> {
    match proposal.action {
        ChatReplyAction::StaySilent => Ok(String::new()),
        ChatReplyAction::Speak => {
            let raw = proposal.text.as_deref().unwrap_or_default().trim();
            if raw.is_empty() {
                return Err(format!(
                    "attempt {} requested action=speak but provided no text",
                    attempt
                ));
            }

            let text = sanitize_spoken_text(raw);
            if text.is_empty() {
                return Err(format!(
                    "attempt {} produced empty spoken text after sanitization",
                    attempt
                ));
            }

            if looks_like_internal_reasoning(&text) {
                return Err(format!(
                    "attempt {} leaked internal reasoning instead of spoken dialogue",
                    attempt
                ));
            }

            Ok(text)
        }
    }
}

fn validate_action_proposal(
    request: &ActionRequest,
    proposal: &NpcActionProposal,
    attempt: usize,
) -> std::result::Result<ActionChoice, String> {
    let source = if attempt == 1 {
        "structured_output"
    } else {
        "structured_output_repair"
    };

    match proposal.action {
        NpcActionKind::PathfindSomewhere => {
            let reason = require_reason(&proposal.reason, attempt, "pathfind_somewhere")?;
            let Some(candidate_id) = proposal.move_candidate_id else {
                return Err(format!(
                    "attempt {} requested action=pathfind_somewhere but omitted move_candidate_id",
                    attempt
                ));
            };

            let Some(candidate) = request
                .move_candidates
                .iter()
                .find(|candidate| candidate.id == candidate_id)
            else {
                let valid_ids = request
                    .move_candidates
                    .iter()
                    .map(|candidate| candidate.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "attempt {} proposed invalid move_candidate_id {}; valid ids are [{}]",
                    attempt, candidate_id, valid_ids
                ));
            };

            Ok(ActionChoice::Move(MoveChoice {
                candidate_id: candidate.id,
                position: candidate.position,
                reason,
                source,
            }))
        }
        NpcActionKind::Speak => {
            let reason = require_reason(&proposal.reason, attempt, "speak")?;
            let raw = proposal.text.as_deref().unwrap_or_default().trim();
            if raw.is_empty() {
                return Err(format!(
                    "attempt {} requested action=speak but provided no text",
                    attempt
                ));
            }

            let text = sanitize_spoken_text(raw);
            if text.is_empty() {
                return Err(format!(
                    "attempt {} produced empty spoken text after sanitization",
                    attempt
                ));
            }

            if looks_like_internal_reasoning(&text) {
                return Err(format!(
                    "attempt {} leaked internal reasoning instead of spoken dialogue",
                    attempt
                ));
            }

            Ok(ActionChoice::Speak {
                text,
                reason,
                source,
            })
        }
        NpcActionKind::DrinkBeer => {
            let reason = require_reason(&proposal.reason, attempt, "drink_beer")?;
            let Some(candidate_id) = proposal.drink_candidate_id else {
                return Err(format!(
                    "attempt {} requested action=drink_beer but omitted drink_candidate_id",
                    attempt
                ));
            };

            let Some(candidate) = request
                .drink_candidates
                .iter()
                .find(|candidate| candidate.id == candidate_id)
            else {
                let valid_ids = request
                    .drink_candidates
                    .iter()
                    .map(|candidate| candidate.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "attempt {} proposed invalid drink_candidate_id {}; valid ids are [{}]",
                    attempt, candidate_id, valid_ids
                ));
            };

            Ok(ActionChoice::Drink(DrinkChoice {
                candidate_id: candidate.id,
                position: candidate.position,
                prop: candidate.prop,
                reason,
                source,
            }))
        }
        NpcActionKind::DoNothing => Ok(ActionChoice::Idle {
            reason: require_reason(&proposal.reason, attempt, "do_nothing")?,
            source,
        }),
    }
}

fn build_action_decision(
    request: &ActionRequest,
    choice: ActionChoice,
    attempts: usize,
) -> ActionDecision {
    let trace = build_action_success_trace(request, &choice, attempts);
    let outcome = match choice {
        ActionChoice::Move(choice) => NpcActionOutcome::Move {
            destination: choice.position,
            summary: summarize_move_decision(&choice),
        },
        ActionChoice::Speak { text, .. } => NpcActionOutcome::Speak { text },
        ActionChoice::Drink(choice) => NpcActionOutcome::Drink {
            position: choice.position,
            summary: summarize_drink_decision(&choice),
        },
        ActionChoice::Idle { reason, .. } => NpcActionOutcome::Idle {
            summary: format!("stays put for now because {}", reason),
        },
    };

    ActionDecision { outcome, trace }
}

fn summarize_move_decision(choice: &MoveChoice) -> String {
    format!(
        "heads to ({}, {}) because {}",
        choice.position.x, choice.position.y, choice.reason
    )
}

fn summarize_drink_decision(choice: &DrinkChoice) -> String {
    format!(
        "takes a drink by the {} because {}",
        choice.prop.label(),
        choice.reason
    )
}

fn require_reason(
    reason: &str,
    attempt: usize,
    action: &str,
) -> std::result::Result<String, String> {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        Err(format!(
            "attempt {} requested action={} but provided no reason",
            attempt, action
        ))
    } else {
        Ok(normalize_runtime_text(trimmed))
    }
}

fn normalize_runtime_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_spoken_text(input: &str) -> String {
    let mut text = normalize_runtime_text(input);

    for prefix in ["you say:", "you:", "npc:", "assistant:"] {
        if text.to_lowercase().starts_with(prefix) {
            text = text[prefix.len()..].trim().to_string();
            break;
        }
    }

    if let Some((prefix, rest)) = text.split_once(':') {
        let prefix = prefix.trim();
        if !rest.trim().is_empty()
            && prefix.len() <= 40
            && (1..=4).contains(&prefix.split_whitespace().count())
            && prefix
                .chars()
                .all(|ch| ch.is_ascii_alphabetic() || ch == ' ' || ch == '\'' || ch == '-')
        {
            text = rest.trim().to_string();
        }
    }

    text = text
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '[' | ']'))
        .trim()
        .to_string();

    if let Some(first_line) = text.lines().next() {
        text = normalize_runtime_text(first_line);
    }

    text
}

fn looks_like_internal_reasoning(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "the user said",
        "i should",
        "i need to respond",
        "my reply",
        "stay in character",
        "meta commentary",
        "first,",
        "as stukos",
        "as domas",
        "as zasit",
        "the hall memory",
        "the prompt",
        "the system prompt",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn build_action_success_trace(
    request: &ActionRequest,
    choice: &ActionChoice,
    attempts: usize,
) -> String {
    let choice_fragment = match choice {
        ActionChoice::Move(choice) => format!(
            "action=pathfind_somewhere source={} move_candidate_id={} x={} y={} reason={}",
            choice.source, choice.candidate_id, choice.position.x, choice.position.y, choice.reason
        ),
        ActionChoice::Speak {
            text,
            reason,
            source,
        } => format!(
            "action=speak source={} reason={} text={}",
            source,
            reason,
            normalize_runtime_text(text)
        ),
        ActionChoice::Drink(choice) => format!(
            "action=drink_beer source={} drink_candidate_id={} x={} y={} prop={} reason={}",
            choice.source,
            choice.candidate_id,
            choice.position.x,
            choice.position.y,
            choice.prop.label(),
            choice.reason
        ),
        ActionChoice::Idle { reason, source } => {
            format!("action=do_nothing source={} reason={}", source, reason)
        }
    };

    format!(
        "req#{} provider={} model={} backend={} attempts={} move_candidates={} drink_candidates={} {}",
        request.request_id,
        request.provider.label(),
        request.model,
        request.provider.action_planner_backend().label(),
        attempts,
        request.move_candidates.len(),
        request.drink_candidates.len(),
        choice_fragment
    )
}

fn build_action_failure_trace(request: &ActionRequest, error: &str) -> String {
    format!(
        "req#{} provider={} model={} backend={} move_candidates={} drink_candidates={} failure={}",
        request.request_id,
        request.provider.label(),
        request.model,
        request.provider.action_planner_backend().label(),
        request.move_candidates.len(),
        request.drink_candidates.len(),
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

impl ActionPlannerBackend {
    fn label(self) -> &'static str {
        match self {
            Self::StructuredOutput => "structured_output",
            Self::HeuristicOnly => "heuristic_only",
        }
    }
}

impl ProviderKind {
    fn action_planner_backend(self) -> ActionPlannerBackend {
        match self {
            Self::Llamafile => ActionPlannerBackend::HeuristicOnly,
            Self::Ollama | Self::OpenAi | Self::Anthropic | Self::Gemini => {
                ActionPlannerBackend::StructuredOutput
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
        ActionChoice, ActionRequest, DrinkCandidate, MovementCandidate, NpcActionKind,
        NpcActionProposal, OllamaModelRecord, ProviderKind, choose_preferred_ollama_model,
        looks_like_chat_model, ollama_model_name, validate_action_proposal,
    };
    use crate::{components::Position, map::PropKind};
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
    fn validates_structured_pathfind_candidate_ids() {
        let request = ActionRequest {
            request_id: 7,
            entity: Entity::PLACEHOLDER,
            provider: ProviderKind::Ollama,
            model: "llama3.2".to_string(),
            system_prompt: "system".to_string(),
            context: "context".to_string(),
            move_candidates: vec![MovementCandidate {
                id: 3,
                position: Position::new(41, 15),
                metadata: "tile=(41,15)".to_string(),
            }],
            drink_candidates: Vec::new(),
        };

        let choice = validate_action_proposal(
            &request,
            &NpcActionProposal {
                action: NpcActionKind::PathfindSomewhere,
                move_candidate_id: Some(3),
                drink_candidate_id: None,
                text: None,
                reason: "heading home".to_string(),
            },
            1,
        )
        .expect("proposal should validate");

        match choice {
            ActionChoice::Move(choice) => {
                assert_eq!(choice.candidate_id, 3);
                assert_eq!(choice.position, Position::new(41, 15));
                assert_eq!(choice.source, "structured_output");
            }
            other => panic!("expected move choice, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_structured_pathfind_candidate_ids() {
        let request = ActionRequest {
            request_id: 8,
            entity: Entity::PLACEHOLDER,
            provider: ProviderKind::Ollama,
            model: "llama3.2".to_string(),
            system_prompt: "system".to_string(),
            context: "context".to_string(),
            move_candidates: vec![
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
            drink_candidates: Vec::new(),
        };

        let error = validate_action_proposal(
            &request,
            &NpcActionProposal {
                action: NpcActionKind::PathfindSomewhere,
                move_candidate_id: Some(9),
                drink_candidate_id: None,
                text: None,
                reason: "looking for a new seat".to_string(),
            },
            2,
        )
        .expect_err("candidate id should be rejected");

        assert!(error.contains("invalid move_candidate_id 9"));
        assert!(error.contains("[0, 1]"));
    }

    #[test]
    fn validates_drink_candidates() {
        let request = ActionRequest {
            request_id: 9,
            entity: Entity::PLACEHOLDER,
            provider: ProviderKind::Ollama,
            model: "llama3.2".to_string(),
            system_prompt: "system".to_string(),
            context: "context".to_string(),
            move_candidates: Vec::new(),
            drink_candidates: vec![DrinkCandidate {
                id: 2,
                position: Position::new(20, 8),
                prop: PropKind::Bottle,
                metadata: "tile=(20,8)".to_string(),
            }],
        };

        let choice = validate_action_proposal(
            &request,
            &NpcActionProposal {
                action: NpcActionKind::DrinkBeer,
                move_candidate_id: None,
                drink_candidate_id: Some(2),
                text: None,
                reason: "throat feels dry".to_string(),
            },
            1,
        )
        .expect("drink proposal should validate");

        match choice {
            ActionChoice::Drink(choice) => {
                assert_eq!(choice.candidate_id, 2);
                assert_eq!(choice.position, Position::new(20, 8));
                assert_eq!(choice.prop, PropKind::Bottle);
            }
            other => panic!("expected drink choice, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_required_reason() {
        let request = ActionRequest {
            request_id: 10,
            entity: Entity::PLACEHOLDER,
            provider: ProviderKind::Ollama,
            model: "llama3.2".to_string(),
            system_prompt: "system".to_string(),
            context: "context".to_string(),
            move_candidates: vec![MovementCandidate {
                id: 0,
                position: Position::new(10, 10),
                metadata: "tile=(10,10)".to_string(),
            }],
            drink_candidates: Vec::new(),
        };

        let error = validate_action_proposal(
            &request,
            &NpcActionProposal {
                action: NpcActionKind::PathfindSomewhere,
                move_candidate_id: Some(0),
                drink_candidate_id: None,
                text: None,
                reason: "   ".to_string(),
            },
            1,
        )
        .expect_err("empty reason should be rejected");

        assert!(error.contains("provided no reason"));
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
