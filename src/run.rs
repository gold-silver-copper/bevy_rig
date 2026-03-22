use std::collections::HashMap;

use bevy_ecs::{message::Messages, prelude::*};

use crate::{
    agent::{AgentContextRefs, PrimarySession},
    context::{ContextIndex, ContextPayload, ContextSource},
    session::{ChatMessageBundle, ChatMessageRole},
};

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Run;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunOwner(pub Entity);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSession(pub Entity);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    pub prompt: String,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunContextQuery {
    pub top_k: usize,
}

impl Default for RunContextQuery {
    fn default() -> Self {
        Self { top_k: 3 }
    }
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct RunResultText(pub String);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct RunFailure(pub String);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct RunPrompt(pub String);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct RunRetrievedContexts(pub Vec<Entity>);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct RunStreamBuffer(pub String);

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct RunCancellationReason(pub String);

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct RunFinalized;

#[derive(Bundle)]
pub struct RunBundle {
    pub run: Run,
    pub owner: RunOwner,
    pub session: RunSession,
    pub request: RunRequest,
    pub status: RunStatus,
}

impl RunBundle {
    pub fn new(owner: Entity, session: Entity, prompt: impl Into<String>) -> Self {
        Self {
            run: Run,
            owner: RunOwner(owner),
            session: RunSession(session),
            request: RunRequest {
                prompt: prompt.into(),
            },
            status: RunStatus::Queued,
        }
    }
}

#[derive(Message, Clone, Debug)]
pub struct RunAgent {
    pub agent: Entity,
    pub prompt: String,
}

impl RunAgent {
    pub fn new(agent: Entity, prompt: impl Into<String>) -> Self {
        Self {
            agent,
            prompt: prompt.into(),
        }
    }
}

#[derive(Message, Clone, Copy, Debug)]
pub struct RunCommitted {
    pub run: Entity,
}

#[derive(Message, Clone, Debug)]
pub struct RunFailed {
    pub run: Option<Entity>,
    pub error: String,
}

#[derive(Message, Clone, Debug)]
pub struct CancelRun {
    pub run: Entity,
    pub reason: Option<String>,
}

impl CancelRun {
    pub fn new(run: Entity, reason: Option<impl Into<String>>) -> Self {
        Self {
            run,
            reason: reason.map(Into::into),
        }
    }
}

#[derive(Message, Clone, Debug)]
pub struct TextDelta {
    pub run: Entity,
    pub delta: String,
}

impl TextDelta {
    pub fn new(run: Entity, delta: impl Into<String>) -> Self {
        Self {
            run,
            delta: delta.into(),
        }
    }
}

#[derive(Message, Clone, Copy, Debug)]
pub struct StreamCompleted {
    pub run: Entity,
}

pub fn capture_run_requests(
    mut commands: Commands,
    mut requests: MessageReader<RunAgent>,
    agents: Query<&PrimarySession>,
    mut failures: MessageWriter<RunFailed>,
) {
    for request in requests.read() {
        let Ok(primary_session) = agents.get(request.agent) else {
            failures.write(RunFailed {
                run: None,
                error: format!("agent {:?} is missing a primary session", request.agent),
            });
            continue;
        };

        commands.spawn((
            RunBundle::new(request.agent, primary_session.0, request.prompt.clone()),
            RunContextQuery::default(),
        ));
        commands.spawn(ChatMessageBundle::new(
            primary_session.0,
            ChatMessageRole::User,
            request.prompt.clone(),
        ));
    }
}

pub fn cancel_runs(
    mut commands: Commands,
    mut requests: MessageReader<CancelRun>,
    runs: Query<Entity, With<Run>>,
) {
    for request in requests.read() {
        if runs.get(request.run).is_err() {
            continue;
        }

        let mut entity = commands.entity(request.run);
        entity.insert(RunStatus::Cancelled);
        entity.remove::<RunResultText>();
        entity.remove::<RunFailure>();
        entity.remove::<RunStreamBuffer>();

        if let Some(reason) = &request.reason {
            entity.insert(RunCancellationReason(reason.clone()));
        }
    }
}

pub fn assemble_run_prompts(
    mut commands: Commands,
    runs: Query<
        (
            Entity,
            &RunOwner,
            &RunRequest,
            &RunStatus,
            Option<&RunContextQuery>,
        ),
        (With<Run>, Without<RunPrompt>),
    >,
    agents: Query<&AgentContextRefs>,
    contexts: Query<(&ContextSource, &ContextPayload)>,
    context_index: Res<ContextIndex>,
) {
    for (run, owner, request, status, context_query) in &runs {
        if *status != RunStatus::Queued {
            continue;
        }

        let candidate_contexts = agents
            .get(owner.0)
            .map(|refs| refs.0.clone())
            .unwrap_or_default();
        let top_k = context_query.copied().unwrap_or_default().top_k;

        let mut retrieved = context_index
            .search_candidates(candidate_contexts.iter().copied(), &request.prompt, top_k)
            .into_iter()
            .map(|matched| matched.entity)
            .collect::<Vec<_>>();

        if retrieved.is_empty() {
            retrieved.extend(candidate_contexts.into_iter().take(top_k));
        }

        let prompt = build_run_prompt(&request.prompt, &retrieved, &contexts);
        commands
            .entity(run)
            .insert((RunPrompt(prompt), RunRetrievedContexts(retrieved)));
    }
}

pub fn persist_completed_runs(
    mut commands: Commands,
    runs: Query<
        (Entity, &RunSession, &RunResultText, &RunStatus),
        (With<Run>, Without<RunFinalized>),
    >,
    mut committed: MessageWriter<RunCommitted>,
) {
    for (run, session, result, status) in &runs {
        if *status != RunStatus::Completed {
            continue;
        }

        commands.spawn(ChatMessageBundle::new(
            session.0,
            ChatMessageRole::Assistant,
            result.0.clone(),
        ));
        commands.entity(run).insert(RunFinalized);
        committed.write(RunCommitted { run });
    }
}

pub fn persist_cancelled_runs(
    mut commands: Commands,
    runs: Query<
        (
            Entity,
            &RunSession,
            &RunStatus,
            Option<&RunCancellationReason>,
        ),
        (With<Run>, Without<RunFinalized>),
    >,
) {
    for (run, session, status, reason) in &runs {
        if *status != RunStatus::Cancelled {
            continue;
        }

        let message = reason
            .map(|reason| format!("run cancelled: {}", reason.0))
            .unwrap_or_else(|| "run cancelled".to_string());
        commands.spawn(ChatMessageBundle::new(
            session.0,
            ChatMessageRole::System,
            message,
        ));
        commands.entity(run).insert(RunFinalized);
    }
}

pub fn persist_failed_runs(
    mut commands: Commands,
    runs: Query<(Entity, &RunSession, &RunFailure, &RunStatus), (With<Run>, Without<RunFinalized>)>,
    mut failures: MessageWriter<RunFailed>,
) {
    for (run, session, failure, status) in &runs {
        if *status != RunStatus::Failed {
            continue;
        }

        commands.spawn(ChatMessageBundle::new(
            session.0,
            ChatMessageRole::System,
            format!("run failed: {}", failure.0),
        ));
        commands.entity(run).insert(RunFinalized);
        failures.write(RunFailed {
            run: Some(run),
            error: failure.0.clone(),
        });
    }
}

pub fn mark_run_completed(commands: &mut Commands, run: Entity, text: impl Into<String>) {
    let mut entity = commands.entity(run);
    entity.insert((RunStatus::Completed, RunResultText(text.into())));
    entity.remove::<RunFailure>();
}

pub fn mark_run_failed(commands: &mut Commands, run: Entity, error: impl Into<String>) {
    let mut entity = commands.entity(run);
    entity.insert((RunStatus::Failed, RunFailure(error.into())));
    entity.remove::<RunResultText>();
}

pub fn apply_text_deltas(world: &mut World) {
    let grouped = {
        let mut messages = world.resource_mut::<Messages<TextDelta>>();
        let mut grouped = HashMap::<Entity, String>::new();
        for message in messages.drain() {
            grouped
                .entry(message.run)
                .or_default()
                .push_str(&message.delta);
        }
        grouped
    };

    for (run, delta) in grouped {
        let Ok(mut entity) = world.get_entity_mut(run) else {
            continue;
        };

        if let Some(mut buffer) = entity.get_mut::<RunStreamBuffer>() {
            buffer.0.push_str(&delta);
        } else {
            entity.insert(RunStreamBuffer(delta));
        }

        if matches!(
            entity.get::<RunStatus>(),
            Some(RunStatus::Queued | RunStatus::Running)
        ) {
            entity.insert(RunStatus::Running);
        }
    }
}

pub fn finish_streams(world: &mut World) {
    let completed_runs: Vec<Entity> = {
        let mut messages = world.resource_mut::<Messages<StreamCompleted>>();
        messages.drain().map(|message| message.run).collect()
    };

    for run in completed_runs {
        let Ok(mut entity) = world.get_entity_mut(run) else {
            continue;
        };

        let text = entity
            .get::<RunStreamBuffer>()
            .map(|buffer| buffer.0.clone())
            .unwrap_or_default();

        entity.insert((RunStatus::Completed, RunResultText(text)));
        entity.remove::<RunFailure>();
    }
}

fn build_run_prompt(
    user_prompt: &str,
    contexts: &[Entity],
    context_query: &Query<(&ContextSource, &ContextPayload)>,
) -> String {
    let mut prompt = String::new();

    if !contexts.is_empty() {
        prompt.push_str("Context:\n");
        for entity in contexts {
            if let Ok((source, payload)) = context_query.get(*entity) {
                prompt.push_str("- ");
                prompt.push_str(&format_context_source(source));
                prompt.push_str(": ");
                prompt.push_str(&payload.text);
                prompt.push('\n');
            }
        }
        prompt.push('\n');
    }

    prompt.push_str("User:\n");
    prompt.push_str(user_prompt);
    prompt
}

fn format_context_source(source: &ContextSource) -> String {
    match source {
        ContextSource::Inline => "inline".to_string(),
        ContextSource::File(path) => format!("file({path})"),
        ContextSource::Generated(label) => format!("generated({label})"),
    }
}
