use std::{
    any::Any,
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bevy_ecs::{hierarchy::ChildOf, message::MessageReader, prelude::*};
use rig::{
    client::{CompletionClient, Nothing, ProviderClient},
    completion::{Chat, Message as RigMessage, Prompt, PromptError, ToolDefinition},
    providers::{
        anthropic, azure, cohere, deepseek, galadriel, gemini, groq, huggingface, hyperbolic,
        llamafile, mira, mistral, moonshot, ollama, openai, openrouter, perplexity, together, xai,
    },
    tool::{ToolDyn, ToolError},
    wasm_compat::WasmBoxedFuture,
};
use thiserror::Error;
use tokio::{
    runtime::Runtime,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel},
        oneshot,
    },
};

use crate::{
    agent::{AgentModelRef, AgentSpec, AgentToolRefs},
    model::{ModelCapabilities, ModelSpec},
    provider::{
        ProviderAuthState, ProviderHealth, ProviderKind, ProviderRevision, ProviderSpec,
    },
    run::{
        Run, RunFailure, RunFinalized, RunOwner, RunPrompt, RunRequest, RunResultText, RunSession,
        RunStatus, RunStreamBuffer,
    },
    session::{self, ChatMessageRole},
    tool::{
        ToolCall, ToolCallCompleted, ToolCallFailed, ToolCallRequested, ToolOutput, ToolRegistry,
        ToolSpec,
    },
};

type RigToolResponse = Result<ToolOutput, String>;

enum RigRuntimeEvent {
    RunFinished {
        run: Entity,
        result: Result<String, String>,
    },
    ToolCallRequested {
        call: ToolCall,
        respond_to: oneshot::Sender<RigToolResponse>,
    },
}

#[derive(Clone)]
enum ProviderClientHandle {
    Anthropic(anthropic::Client),
    Azure(azure::Client),
    Cohere(cohere::Client),
    DeepSeek(deepseek::Client),
    Galadriel(galadriel::Client),
    Gemini(gemini::Client),
    Groq(groq::Client),
    HuggingFace(huggingface::Client),
    Hyperbolic(hyperbolic::Client),
    Llamafile(llamafile::Client),
    Mira(mira::Client),
    Mistral(mistral::Client),
    Moonshot(moonshot::Client),
    Ollama(ollama::Client),
    OpenAi(openai::Client),
    OpenRouter(openrouter::Client),
    Perplexity(perplexity::Client),
    Together(together::Client),
    XAi(xai::Client),
}

#[derive(Clone)]
struct CachedProviderClient {
    spec: ProviderSpec,
    health: ProviderHealth,
    auth_state: ProviderAuthState,
    revision: ProviderRevision,
    client: ProviderClientHandle,
}

impl CachedProviderClient {
    fn matches_snapshot(
        &self,
        spec: &ProviderSpec,
        health: &ProviderHealth,
        auth_state: &ProviderAuthState,
        revision: ProviderRevision,
    ) -> bool {
        self.spec == *spec
            && self.health == *health
            && self.auth_state == *auth_state
            && self.revision == revision
    }
}

#[derive(Resource, Default)]
pub struct ProviderClientCache {
    entries: HashMap<Entity, CachedProviderClient>,
}

#[derive(Resource)]
pub struct RigRuntime {
    runtime: Arc<Runtime>,
    event_tx: UnboundedSender<RigRuntimeEvent>,
    event_rx: UnboundedReceiver<RigRuntimeEvent>,
    pending_tool_results: HashMap<String, oneshot::Sender<RigToolResponse>>,
    next_bridge_call_id: Arc<AtomicU64>,
}

impl Default for RigRuntime {
    fn default() -> Self {
        let runtime = Arc::new(
            Runtime::new().expect("bevy_rig could not create a Tokio runtime for Rig execution"),
        );
        let (event_tx, event_rx) = unbounded_channel();

        Self {
            runtime,
            event_tx,
            event_rx,
            pending_tool_results: HashMap::new(),
            next_bridge_call_id: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl RigRuntime {
    fn handle(&self) -> RigRuntimeHandle {
        RigRuntimeHandle {
            runtime: self.runtime.clone(),
            event_tx: self.event_tx.clone(),
            next_bridge_call_id: self.next_bridge_call_id.clone(),
        }
    }
}

#[derive(Clone)]
struct RigRuntimeHandle {
    runtime: Arc<Runtime>,
    event_tx: UnboundedSender<RigRuntimeEvent>,
    next_bridge_call_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct PreparedRigRun {
    run: Entity,
    client: ProviderClientHandle,
    model_name: String,
    max_turns: Option<usize>,
    prompt: String,
    history: Vec<RigMessage>,
    tools: Vec<AttachedRigTool>,
}

#[derive(Clone)]
struct AttachedRigTool {
    entity: Entity,
    spec: ToolSpec,
}

#[derive(Clone)]
struct BevyRigTool {
    run: Entity,
    tool: Entity,
    spec: ToolSpec,
    event_tx: UnboundedSender<RigRuntimeEvent>,
    next_bridge_call_id: Arc<AtomicU64>,
}

impl BevyRigTool {
    fn next_call_id(&self) -> String {
        let nonce = self.next_bridge_call_id.fetch_add(1, Ordering::Relaxed);
        format!(
            "run{}-tool{}-bridge{nonce}",
            self.run.index(),
            self.tool.index()
        )
    }
}

impl ToolDyn for BevyRigTool {
    fn name(&self) -> String {
        self.spec.name.clone()
    }

    fn definition<'a>(&'a self, _prompt: String) -> WasmBoxedFuture<'a, ToolDefinition> {
        let spec = self.spec.clone();
        Box::pin(async move {
            ToolDefinition {
                name: spec.name,
                description: spec.description,
                parameters: spec.schema,
            }
        })
    }

    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            let args = serde_json::from_str(&args).map_err(ToolError::JsonError)?;
            let call = ToolCall {
                run: self.run,
                tool: self.tool,
                call_id: self.next_call_id(),
                args,
            };
            let (respond_to, response) = oneshot::channel();
            self.event_tx
                .send(RigRuntimeEvent::ToolCallRequested { call, respond_to })
                .map_err(|_| ToolError::ToolCallError(Box::new(BevyRigToolBridgeError::Closed)))?;

            match response.await {
                Ok(Ok(output)) => Ok(output.value.to_string()),
                Ok(Err(error)) => Err(ToolError::ToolCallError(Box::new(
                    BevyRigToolBridgeError::Execution(error),
                ))),
                Err(_) => Err(ToolError::ToolCallError(Box::new(
                    BevyRigToolBridgeError::Dropped,
                ))),
            }
        })
    }
}

#[derive(Debug, Error)]
enum BevyRigToolBridgeError {
    #[error("bevy_rig tool bridge is closed")]
    Closed,
    #[error("bevy_rig tool bridge dropped the response channel")]
    Dropped,
    #[error("{0}")]
    Execution(String),
}

#[derive(Debug, Error)]
pub enum RigExecutionError {
    #[error("agent entity {0:?} is missing AgentSpec")]
    MissingAgentSpec(Entity),
    #[error("agent entity {0:?} is missing AgentModelRef")]
    MissingAgentModelRef(Entity),
    #[error("model entity {0:?} is missing ModelSpec")]
    MissingModelSpec(Entity),
    #[error("model entity {0:?} is missing ModelCapabilities")]
    MissingModelCapabilities(Entity),
    #[error("model entity {0:?} does not support completion runs")]
    ModelNotCompletionCapable(Entity),
    #[error("model entity {0:?} is missing a provider parent")]
    MissingModelParent(Entity),
    #[error("provider entity {0:?} is missing ProviderSpec")]
    MissingProviderSpec(Entity),
    #[error("provider entity {provider:?} is not ready: {reason}")]
    ProviderNotReady { provider: Entity, reason: String },
    #[error("provider {kind:?} on entity {provider:?} could not be initialized: {error}")]
    ProviderUnavailable {
        provider: Entity,
        kind: ProviderKind,
        error: String,
    },
    #[error("tool entity {0:?} is missing ToolSpec")]
    MissingToolSpec(Entity),
    #[error("session entity {0:?} does not exist")]
    MissingSession(Entity),
    #[error("{0}")]
    PromptFailure(#[from] PromptError),
}

pub fn execute_rig_runs(world: &mut World) {
    spawn_pending_rig_runs(world);
    drain_rig_runtime_events(world);
}

pub fn prune_provider_client_cache(
    mut cache: ResMut<ProviderClientCache>,
    providers: Query<Entity, With<ProviderSpec>>,
) {
    cache.entries
        .retain(|provider, _| providers.get(*provider).is_ok());
}

pub fn resolve_rig_tool_results(
    mut runtime: ResMut<RigRuntime>,
    mut completed: MessageReader<ToolCallCompleted>,
    mut failed: MessageReader<ToolCallFailed>,
) {
    for message in completed.read() {
        let Some(respond_to) = runtime.pending_tool_results.remove(&message.call.call_id) else {
            continue;
        };
        let _ = respond_to.send(Ok(message.output.clone()));
    }

    for message in failed.read() {
        let Some(respond_to) = runtime.pending_tool_results.remove(&message.call.call_id) else {
            continue;
        };
        let _ = respond_to.send(Err(message.error.clone()));
    }
}

fn spawn_pending_rig_runs(world: &mut World) {
    let pending_runs = {
        let mut query = world.query_filtered::<
            (
                Entity,
                &RunOwner,
                &RunSession,
                &RunRequest,
                Option<&RunPrompt>,
                &RunStatus,
            ),
            (With<Run>, Without<RunFinalized>),
        >();

        query
            .iter(world)
            .filter(|(_, _, _, _, _, status)| **status == RunStatus::Queued)
            .map(|(run, owner, session, request, prompt, _)| PendingRigRun {
                run,
                owner: owner.0,
                session: session.0,
                request_prompt: request.prompt.clone(),
                execution_prompt: prompt
                    .map(|prompt| prompt.0.clone())
                    .unwrap_or_else(|| request.prompt.clone()),
            })
            .collect::<Vec<_>>()
    };

    let runtime_handle = world.resource::<RigRuntime>().handle();

    for pending in pending_runs {
        let prepared = match prepare_rig_run(
            world,
            pending.owner,
            &pending.execution_prompt,
            Some(pending.session),
            Some(&pending.request_prompt),
            pending.run,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                insert_run_failure(world, pending.run, error.to_string());
                continue;
            }
        };

        if let Ok(mut entity) = world.get_entity_mut(pending.run) {
            entity.insert(RunStatus::Running);
        }

        let task_handle = runtime_handle.clone();
        std::thread::spawn(move || {
            let result = task_handle
                .runtime
                .block_on(async {
                    execute_prompt_with_rig(
                        prepared.client,
                        prepared.model_name,
                        prepared.max_turns,
                        prepared.prompt,
                        prepared.history,
                        prepared.tools,
                        Some(task_handle.clone()),
                        prepared.run,
                    )
                    .await
                })
                .map_err(|error| error.to_string());

            let _ = task_handle.event_tx.send(RigRuntimeEvent::RunFinished {
                run: prepared.run,
                result,
            });
        });
    }
}

fn drain_rig_runtime_events(world: &mut World) {
    loop {
        let event = {
            let mut runtime = world.resource_mut::<RigRuntime>();
            match runtime.event_rx.try_recv() {
                Ok(event) => Some(event),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
            }
        };

        let Some(event) = event else {
            break;
        };

        match event {
            RigRuntimeEvent::RunFinished { run, result } => {
                let Some(status) = world.get::<RunStatus>(run).cloned() else {
                    continue;
                };

                if !matches!(status, RunStatus::Queued | RunStatus::Running) {
                    continue;
                }

                match result {
                    Ok(text) => insert_run_success(world, run, text),
                    Err(error) => insert_run_failure(world, run, error),
                }
            }
            RigRuntimeEvent::ToolCallRequested { call, respond_to } => {
                let is_registered = {
                    let registry = world.resource::<ToolRegistry>();
                    registry.get(call.tool).is_some()
                };

                if !is_registered {
                    let _ = respond_to.send(Err(format!(
                        "tool entity {:?} is not registered",
                        call.tool
                    )));
                    continue;
                }

                world
                    .resource_mut::<RigRuntime>()
                    .pending_tool_results
                    .insert(call.call_id.clone(), respond_to);
                world.write_message(ToolCallRequested { call });
            }
        }
    }
}

#[derive(Clone)]
struct PendingRigRun {
    run: Entity,
    owner: Entity,
    session: Entity,
    request_prompt: String,
    execution_prompt: String,
}

fn insert_run_success(world: &mut World, run: Entity, text: String) {
    let Ok(mut entity) = world.get_entity_mut(run) else {
        return;
    };

    entity.insert((RunStatus::Completed, RunResultText(text)));
    entity.remove::<RunFailure>();
    entity.remove::<RunStreamBuffer>();
}

fn insert_run_failure(world: &mut World, run: Entity, error: String) {
    let Ok(mut entity) = world.get_entity_mut(run) else {
        return;
    };

    entity.insert((RunStatus::Failed, RunFailure(error)));
    entity.remove::<RunResultText>();
    entity.remove::<RunStreamBuffer>();
}

fn prepare_rig_run(
    world: &mut World,
    agent: Entity,
    prompt: &str,
    session: Option<Entity>,
    current_user_message: Option<&str>,
    run: Entity,
) -> Result<PreparedRigRun, RigExecutionError> {
    let agent_spec = world
        .get::<AgentSpec>(agent)
        .cloned()
        .ok_or(RigExecutionError::MissingAgentSpec(agent))?;
    let model = world
        .get::<AgentModelRef>(agent)
        .copied()
        .map(|model| model.0)
        .ok_or(RigExecutionError::MissingAgentModelRef(agent))?;
    let model_spec = world
        .get::<ModelSpec>(model)
        .cloned()
        .ok_or(RigExecutionError::MissingModelSpec(model))?;
    let model_capabilities = world
        .get::<ModelCapabilities>(model)
        .cloned()
        .ok_or(RigExecutionError::MissingModelCapabilities(model))?;

    if !model_capabilities.completions {
        return Err(RigExecutionError::ModelNotCompletionCapable(model));
    }

    let provider = world
        .get::<ChildOf>(model)
        .map(ChildOf::parent)
        .ok_or(RigExecutionError::MissingModelParent(model))?;
    let client = resolve_provider_client(world, provider)?;
    let history = match session {
        Some(session) => collect_rig_history(world, session, current_user_message)?,
        None => Vec::new(),
    };
    let tools = collect_attached_tools(world, agent)?;

    Ok(PreparedRigRun {
        run,
        client,
        model_name: model_spec.name,
        max_turns: agent_spec.max_turns,
        prompt: prompt.to_string(),
        history,
        tools,
    })
}

fn resolve_provider_client(
    world: &mut World,
    provider: Entity,
) -> Result<ProviderClientHandle, RigExecutionError> {
    let spec = world
        .get::<ProviderSpec>(provider)
        .cloned()
        .ok_or(RigExecutionError::MissingProviderSpec(provider))?;
    let health = world
        .get::<ProviderHealth>(provider)
        .cloned()
        .unwrap_or_default();
    let auth_state = world
        .get::<ProviderAuthState>(provider)
        .cloned()
        .unwrap_or_else(|| ProviderAuthState::for_spec(&spec));
    let revision = world
        .get::<ProviderRevision>(provider)
        .copied()
        .unwrap_or_default();

    if let Some(client) = {
        let cache = world.resource::<ProviderClientCache>();
        cache.entries.get(&provider).and_then(|cached| {
            if cached.matches_snapshot(&spec, &health, &auth_state, revision) {
                Some(cached.client.clone())
            } else {
                None
            }
        })
    } {
        return Ok(client);
    }

    ensure_provider_ready(provider, &health, &auth_state)?;

    let client = build_provider_client(provider, &spec)?;
    world.resource_mut::<ProviderClientCache>().entries.insert(
        provider,
        CachedProviderClient {
            spec,
            health,
            auth_state,
            revision,
            client: client.clone(),
        },
    );

    Ok(client)
}

fn ensure_provider_ready(
    provider: Entity,
    health: &ProviderHealth,
    auth_state: &ProviderAuthState,
) -> Result<(), RigExecutionError> {
    if !health.allows_requests() {
        return Err(RigExecutionError::ProviderNotReady {
            provider,
            reason: format!("health state is {health:?}"),
        });
    }

    if !auth_state.allows_requests() {
        return Err(RigExecutionError::ProviderNotReady {
            provider,
            reason: format!("auth state is {auth_state:?}"),
        });
    }

    Ok(())
}

fn collect_attached_tools(
    world: &World,
    agent: Entity,
) -> Result<Vec<AttachedRigTool>, RigExecutionError> {
    let refs = world
        .get::<AgentToolRefs>(agent)
        .cloned()
        .unwrap_or_default();

    refs.0
        .into_iter()
        .map(|tool| {
            let spec = world
                .get::<ToolSpec>(tool)
                .cloned()
                .ok_or(RigExecutionError::MissingToolSpec(tool))?;
            Ok(AttachedRigTool { entity: tool, spec })
        })
        .collect()
}

fn collect_rig_history(
    world: &World,
    session: Entity,
    current_user_message: Option<&str>,
) -> Result<Vec<RigMessage>, RigExecutionError> {
    if world.get_entity(session).is_err() {
        return Err(RigExecutionError::MissingSession(session));
    }

    let mut transcript = session::collect_transcript(world, session);

    if let Some(current_user_message) = current_user_message {
        let should_strip_current = matches!(
            transcript.last(),
            Some((ChatMessageRole::User, text)) if text == current_user_message
        );
        if should_strip_current {
            transcript.pop();
        }
    }

    Ok(transcript
        .into_iter()
        .map(|(role, text)| match role {
            ChatMessageRole::System => RigMessage::system(text),
            ChatMessageRole::User => RigMessage::user(text),
            ChatMessageRole::Assistant => RigMessage::assistant(text),
        })
        .collect())
}

async fn execute_prompt_with_rig(
    client: ProviderClientHandle,
    model_name: String,
    max_turns: Option<usize>,
    prompt: String,
    history: Vec<RigMessage>,
    tools: Vec<AttachedRigTool>,
    runtime_handle: Option<RigRuntimeHandle>,
    run: Entity,
) -> Result<String, RigExecutionError> {
    match client {
        ProviderClientHandle::Anthropic(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Azure(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Cohere(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::DeepSeek(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Galadriel(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Gemini(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Groq(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::HuggingFace(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Hyperbolic(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Llamafile(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Mira(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Mistral(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Moonshot(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Ollama(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::OpenAi(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::OpenRouter(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Perplexity(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::Together(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
        ProviderClientHandle::XAi(client) => {
            execute_prompt_with_client(client, &model_name, max_turns, prompt, history, tools, runtime_handle, run).await
        }
    }
}

async fn execute_prompt_with_client<C>(
    client: C,
    model_name: &str,
    max_turns: Option<usize>,
    prompt: String,
    history: Vec<RigMessage>,
    tools: Vec<AttachedRigTool>,
    runtime_handle: Option<RigRuntimeHandle>,
    run: Entity,
) -> Result<String, RigExecutionError>
where
    C: CompletionClient + Clone,
{
    let mut builder = client.agent(model_name.to_string());

    if let Some(max_turns) = max_turns {
        builder = builder.default_max_turns(max_turns);
    }

    let agent = if tools.is_empty() {
        builder.build()
    } else {
        let runtime_handle =
            runtime_handle.expect("tool-backed Rig runs always have a runtime handle");
        let tools = tools
            .into_iter()
            .map(|tool| {
                Box::new(BevyRigTool {
                    run,
                    tool: tool.entity,
                    spec: tool.spec,
                    event_tx: runtime_handle.event_tx.clone(),
                    next_bridge_call_id: runtime_handle.next_bridge_call_id.clone(),
                }) as Box<dyn ToolDyn>
            })
            .collect::<Vec<_>>();
        builder.tools(tools).build()
    };

    if history.is_empty() {
        agent.prompt(prompt).await.map_err(RigExecutionError::from)
    } else {
        agent
            .chat(prompt, history)
            .await
            .map_err(RigExecutionError::from)
    }
}

fn build_provider_client(
    provider: Entity,
    spec: &ProviderSpec,
) -> Result<ProviderClientHandle, RigExecutionError> {
    macro_rules! from_env_handle {
        ($variant:ident, $client:path) => {
            catch_unwind(AssertUnwindSafe(|| {
                ProviderClientHandle::$variant(<$client>::from_env())
            }))
        };
    }

    let result = match spec.kind {
        ProviderKind::Anthropic => from_env_handle!(Anthropic, anthropic::Client),
        ProviderKind::Azure => from_env_handle!(Azure, azure::Client),
        ProviderKind::Cohere => from_env_handle!(Cohere, cohere::Client),
        ProviderKind::DeepSeek => from_env_handle!(DeepSeek, deepseek::Client),
        ProviderKind::Galadriel => from_env_handle!(Galadriel, galadriel::Client),
        ProviderKind::Gemini => from_env_handle!(Gemini, gemini::Client),
        ProviderKind::Groq => from_env_handle!(Groq, groq::Client),
        ProviderKind::HuggingFace => from_env_handle!(HuggingFace, huggingface::Client),
        ProviderKind::Hyperbolic => from_env_handle!(Hyperbolic, hyperbolic::Client),
        ProviderKind::Llamafile => catch_unwind(AssertUnwindSafe(|| {
            ProviderClientHandle::Llamafile(build_llamafile_client(spec.endpoint.as_deref()))
        })),
        ProviderKind::Mira => from_env_handle!(Mira, mira::Client),
        ProviderKind::Mistral => from_env_handle!(Mistral, mistral::Client),
        ProviderKind::Moonshot => from_env_handle!(Moonshot, moonshot::Client),
        ProviderKind::Ollama => catch_unwind(AssertUnwindSafe(|| {
            ProviderClientHandle::Ollama(build_ollama_client(spec.endpoint.as_deref()))
        })),
        ProviderKind::OpenAi => from_env_handle!(OpenAi, openai::Client),
        ProviderKind::OpenRouter => from_env_handle!(OpenRouter, openrouter::Client),
        ProviderKind::Perplexity => from_env_handle!(Perplexity, perplexity::Client),
        ProviderKind::Together => from_env_handle!(Together, together::Client),
        ProviderKind::XAi => from_env_handle!(XAi, xai::Client),
    };

    result.map_err(|payload| RigExecutionError::ProviderUnavailable {
        provider,
        kind: spec.kind,
        error: panic_payload_to_string(payload),
    })
}

fn build_llamafile_client(endpoint: Option<&str>) -> llamafile::Client {
    if let Some(endpoint) = endpoint {
        llamafile::Client::from_url(endpoint)
    } else {
        llamafile::Client::builder()
            .api_key(Nothing)
            .build()
            .expect("llamafile client builder failed")
    }
}

fn build_ollama_client(endpoint: Option<&str>) -> ollama::Client {
    let mut builder = ollama::Client::builder().api_key(Nothing);
    if let Some(endpoint) = endpoint {
        builder = builder.base_url(endpoint);
    }
    builder.build().expect("ollama client builder failed")
}

fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "unknown provider initialization panic".to_string(),
        },
    }
}
