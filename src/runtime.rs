use std::sync::Arc;

use anyhow::{Result, anyhow};
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use rig::{
    client::{Nothing, ProviderClient},
    completion::Chat,
    message::Message,
    prelude::CompletionClient,
    providers::{
        anthropic, azure, cohere, deepseek, galadriel, gemini, groq, huggingface, hyperbolic,
        llamafile, mira, mistral, moonshot, ollama, openai, openrouter, xai,
    },
};
use tokio::runtime::Runtime;

use crate::domain::{ChatRole, ProviderKind};

const SYSTEM_PROMPT: &str = "You are a helpful chat assistant. Respect the supplied conversation history exactly as written.";

#[derive(Resource)]
pub struct RuntimeBridge {
    runtime: Arc<Runtime>,
    tx: Sender<RuntimeEvent>,
    rx: Receiver<RuntimeEvent>,
}

impl RuntimeBridge {
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("tokio runtime should build");
        let (tx, rx) = unbounded();
        Self {
            runtime: Arc::new(runtime),
            tx,
            rx,
        }
    }

    pub fn spawn_chat(&self, request: RuntimeRequest) {
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let response = execute_chat(request)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(RuntimeEvent::ChatFinished(response));
        });
    }

    pub fn receiver(&self) -> &Receiver<RuntimeEvent> {
        &self.rx
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeRequest {
    pub provider: ProviderKind,
    pub model: String,
    pub prompt: String,
    pub history: Vec<RuntimeMessage>,
}

#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    ChatFinished(std::result::Result<String, String>),
}

async fn execute_chat(request: RuntimeRequest) -> Result<String> {
    match request.provider {
        ProviderKind::Anthropic => run_with_client(anthropic::Client::from_env(), &request).await,
        ProviderKind::Azure => run_with_client(azure::Client::from_env(), &request).await,
        ProviderKind::Cohere => run_with_client(cohere::Client::from_env(), &request).await,
        ProviderKind::DeepSeek => run_with_client(deepseek::Client::from_env(), &request).await,
        ProviderKind::Galadriel => run_with_client(galadriel::Client::from_env(), &request).await,
        ProviderKind::Gemini => run_with_client(gemini::Client::from_env(), &request).await,
        ProviderKind::Groq => run_with_client(groq::Client::from_env(), &request).await,
        ProviderKind::HuggingFace => {
            run_with_client(huggingface::Client::from_env(), &request).await
        }
        ProviderKind::Hyperbolic => run_with_client(hyperbolic::Client::from_env(), &request).await,
        ProviderKind::Llamafile => {
            run_with_client(llamafile::Client::from_val(Nothing), &request).await
        }
        ProviderKind::Mira => run_with_client(mira::Client::from_env(), &request).await,
        ProviderKind::Mistral => run_with_client(mistral::Client::from_env(), &request).await,
        ProviderKind::Moonshot => run_with_client(moonshot::Client::from_env(), &request).await,
        ProviderKind::Ollama => run_with_client(ollama::Client::from_val(Nothing), &request).await,
        ProviderKind::OpenAi => run_with_client(openai::Client::from_env(), &request).await,
        ProviderKind::OpenRouter => run_with_client(openrouter::Client::from_env(), &request).await,
        ProviderKind::Perplexity => {
            run_with_client(rig::providers::perplexity::Client::from_env(), &request).await
        }
        ProviderKind::Together => {
            run_with_client(rig::providers::together::Client::from_env(), &request).await
        }
        ProviderKind::XAi => run_with_client(xai::Client::from_env(), &request).await,
    }
}

async fn run_with_client<C>(client: C, request: &RuntimeRequest) -> Result<String>
where
    C: CompletionClient,
{
    let agent = client
        .agent(request.model.clone())
        .preamble(SYSTEM_PROMPT)
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

fn runtime_message_to_rig(message: &RuntimeMessage) -> Message {
    match message.role {
        ChatRole::User => Message::user(message.content.clone()),
        ChatRole::Assistant => Message::assistant(message.content.clone()),
    }
}
