use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use bevy_ecs::prelude::*;
#[allow(deprecated)]
use rig::client::builder::AnyClient;
use rig::{
    client::ProviderClient,
    completion::{Chat, Message as RigMessage, Prompt, PromptError},
    providers::{
        anthropic, azure, cohere, deepseek, galadriel, gemini, groq, huggingface, hyperbolic,
        llamafile, mira, mistral, moonshot, ollama, openai, openrouter, perplexity, together, xai,
    },
};
use thiserror::Error;
use tokio::runtime::Runtime;

use crate::{
    agent::{AgentSpec, AgentToolRefs},
    provider::{ProviderKind, ProviderSpec},
    run::{
        Run, RunFailure, RunFinalized, RunOwner, RunPrompt, RunRequest, RunResultText, RunSession,
        RunStatus, RunStreamBuffer,
    },
    session::{self, ChatMessageRole},
};

#[derive(Resource, Clone)]
pub struct RigRuntime(pub Arc<Runtime>);

impl Default for RigRuntime {
    fn default() -> Self {
        Self(Arc::new(Runtime::new().expect(
            "bevy_rig could not create a Tokio runtime for Rig execution",
        )))
    }
}

#[derive(Debug, Error)]
pub enum RigExecutionError {
    #[error("agent entity {0:?} is missing AgentSpec")]
    MissingAgentSpec(Entity),
    #[error("agent entity {0:?} is not configured with a provider")]
    MissingProvider(Entity),
    #[error("provider entity {0:?} is missing ProviderSpec")]
    MissingProviderSpec(Entity),
    #[error(
        "agent entity {agent:?} has {tool_count} attached Bevy tool(s), but the Rig tool bridge is not implemented yet"
    )]
    UnsupportedTools { agent: Entity, tool_count: usize },
    #[error("provider {provider:?} could not be initialized from the environment: {error}")]
    ProviderUnavailable {
        provider: ProviderKind,
        error: String,
    },
    #[error("provider {provider:?} does not expose Rig completion capabilities")]
    CompletionUnavailable { provider: ProviderKind },
    #[error("session entity {0:?} does not exist")]
    MissingSession(Entity),
    #[error("{0}")]
    PromptFailure(#[from] PromptError),
}

pub(crate) fn execute_agent_prompt(
    world: &mut World,
    agent: Entity,
    prompt: &str,
    session: Option<Entity>,
    current_user_message: Option<&str>,
) -> Result<String, RigExecutionError> {
    let runtime = world.resource::<RigRuntime>().0.clone();
    let agent_spec = world
        .get::<AgentSpec>(agent)
        .cloned()
        .ok_or(RigExecutionError::MissingAgentSpec(agent))?;
    let provider = agent_spec
        .provider
        .ok_or(RigExecutionError::MissingProvider(agent))?;
    let provider_spec = world
        .get::<ProviderSpec>(provider)
        .cloned()
        .ok_or(RigExecutionError::MissingProviderSpec(provider))?;
    let tool_count = world
        .get::<AgentToolRefs>(agent)
        .map(|refs| refs.0.len())
        .unwrap_or_default();

    if tool_count > 0 {
        return Err(RigExecutionError::UnsupportedTools { agent, tool_count });
    }

    let history = match session {
        Some(session) => collect_rig_history(world, session, current_user_message)?,
        None => Vec::new(),
    };

    execute_prompt_with_rig(runtime, provider_spec.kind, &agent_spec, prompt, history)
}

pub fn execute_rig_runs(world: &mut World) {
    let pending_runs = {
        let mut query = world.query_filtered::<(
            Entity,
            &RunOwner,
            &RunSession,
            &RunRequest,
            Option<&RunPrompt>,
            &RunStatus,
        ), (With<Run>, Without<RunFinalized>)>();

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

    for pending in pending_runs {
        let should_execute = match world.get::<AgentSpec>(pending.owner) {
            Some(spec) => spec.provider.is_some(),
            None => {
                insert_run_failure(
                    world,
                    pending.run,
                    RigExecutionError::MissingAgentSpec(pending.owner).to_string(),
                );
                continue;
            }
        };

        if !should_execute {
            continue;
        }

        if let Ok(mut entity) = world.get_entity_mut(pending.run) {
            entity.insert(RunStatus::Running);
        }

        match execute_agent_prompt(
            world,
            pending.owner,
            &pending.execution_prompt,
            Some(pending.session),
            Some(&pending.request_prompt),
        ) {
            Ok(text) => insert_run_success(world, pending.run, text),
            Err(error) => insert_run_failure(world, pending.run, error.to_string()),
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

#[allow(deprecated)]
fn execute_prompt_with_rig(
    runtime: Arc<Runtime>,
    provider: ProviderKind,
    agent_spec: &AgentSpec,
    prompt: &str,
    history: Vec<RigMessage>,
) -> Result<String, RigExecutionError> {
    let client = load_provider_client(provider)?;
    let completion = client
        .as_completion()
        .ok_or(RigExecutionError::CompletionUnavailable { provider })?;
    let mut builder = completion.agent(&agent_spec.model);

    if let Some(max_turns) = agent_spec.max_turns {
        builder = builder.default_max_turns(max_turns);
    }

    let agent = builder.build();
    if history.is_empty() {
        runtime
            .block_on(async { agent.prompt(prompt.to_owned()).await })
            .map_err(RigExecutionError::from)
    } else {
        runtime
            .block_on(async { agent.chat(prompt.to_owned(), history).await })
            .map_err(RigExecutionError::from)
    }
}

#[allow(deprecated)]
fn load_provider_client(provider: ProviderKind) -> Result<AnyClient, RigExecutionError> {
    macro_rules! from_env_client {
        ($client:path) => {
            catch_unwind(AssertUnwindSafe(|| AnyClient::new(<$client>::from_env())))
        };
    }

    let result = match provider {
        ProviderKind::Anthropic => from_env_client!(anthropic::Client),
        ProviderKind::Azure => from_env_client!(azure::Client),
        ProviderKind::Cohere => from_env_client!(cohere::Client),
        ProviderKind::DeepSeek => from_env_client!(deepseek::Client),
        ProviderKind::Galadriel => from_env_client!(galadriel::Client),
        ProviderKind::Gemini => from_env_client!(gemini::Client),
        ProviderKind::Groq => from_env_client!(groq::Client),
        ProviderKind::HuggingFace => from_env_client!(huggingface::Client),
        ProviderKind::Hyperbolic => from_env_client!(hyperbolic::Client),
        ProviderKind::Llamafile => from_env_client!(llamafile::Client),
        ProviderKind::Mira => from_env_client!(mira::Client),
        ProviderKind::Mistral => from_env_client!(mistral::Client),
        ProviderKind::Moonshot => from_env_client!(moonshot::Client),
        ProviderKind::Ollama => from_env_client!(ollama::Client),
        ProviderKind::OpenAi => from_env_client!(openai::Client),
        ProviderKind::OpenRouter => from_env_client!(openrouter::Client),
        ProviderKind::Perplexity => from_env_client!(perplexity::Client),
        ProviderKind::Together => from_env_client!(together::Client),
        ProviderKind::XAi => from_env_client!(xai::Client),
    };

    result.map_err(|payload| RigExecutionError::ProviderUnavailable {
        provider,
        error: panic_payload_to_string(payload),
    })
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
