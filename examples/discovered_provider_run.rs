use std::{
    env,
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use anyhow::{Result, anyhow};
use bevy_app::App;
use bevy_ecs::hierarchy::ChildOf;
use bevy_rig::prelude::*;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use tokio::runtime::Runtime;

const OLLAMA_API_BASE_URL: &str = "http://localhost:11434";
const LLAMAFILE_API_BASE_URL: &str = "http://localhost:8080";
const OLLAMA_FALLBACK_MODEL: &str = "llama3.2";

fn main() {
    let runtime = Runtime::new().expect("tokio runtime should build");
    let discovered = build_provider_registry(&runtime);

    let mut app = App::new();
    app.add_plugins(BevyRigPlugin);

    let mut chosen_agent = None;

    {
        let world = app.world_mut();

        for entry in &discovered {
            let provider_entity = spawn_provider(
                world,
                ProviderSpec::new(entry.kind, entry.label.to_ascii_lowercase())
                    .with_endpoint(entry.detail.clone()),
                ProviderCapabilities::text_tooling(),
            );

            let model_entity = spawn_model(
                world,
                provider_entity,
                ModelSpec::new(entry.default_model.clone()),
                ModelCapabilities::chat_with_tools(),
                128_000,
            )
            .expect("discovered model should register");

            if chosen_agent.is_none() && entry.ready {
                chosen_agent = Some(
                    spawn_agent_from_model(world, format!("{} agent", entry.label), model_entity)
                        .expect("agent should bind to discovered model")
                        .agent,
                );
            }
        }
    }

    println!("Discovered providers:");
    for entry in &discovered {
        println!(
            "- {} [{}] model={} detail={}",
            entry.label,
            if entry.ready { "ready" } else { "offline" },
            entry.default_model,
            entry.detail
        );
    }

    if let Some(agent) = chosen_agent {
        let world = app.world();
        let model = world
            .get::<AgentModelRef>(agent)
            .expect("agent should have a model ref")
            .0;
        let model_spec = world.get::<ModelSpec>(model).expect("model spec");
        let provider = world
            .get::<ChildOf>(model)
            .expect("model should be parented to provider")
            .parent();
        let provider_spec = world
            .get::<ProviderSpec>(provider)
            .expect("provider spec should exist");

        println!(
            "\nSelected route: {}/{}",
            provider_spec.label, model_spec.name
        );
        println!(
            "Example agent entity: {:?}",
            world
                .get::<AgentSpec>(agent)
                .expect("agent spec should exist")
                .name
        );
        println!("Provider entity: {provider:?}");
        println!("Model entity: {model:?}");
    } else {
        println!("\nNo reachable provider was discovered.");
    }
}

#[derive(Debug, Clone)]
struct DiscoveredProvider {
    kind: ProviderKind,
    label: &'static str,
    default_model: String,
    detail: String,
    ready: bool,
}

fn build_provider_registry(runtime: &Runtime) -> Vec<DiscoveredProvider> {
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

fn build_provider_entry(runtime: &Runtime, kind: ProviderKind) -> DiscoveredProvider {
    match kind {
        ProviderKind::Ollama => build_ollama_entry(runtime),
        ProviderKind::Llamafile => {
            let endpoint = env_base_url("LLAMAFILE_API_BASE_URL", LLAMAFILE_API_BASE_URL);
            DiscoveredProvider {
                kind,
                label: "Llamafile",
                default_model: "LLaMA_CPP".to_string(),
                detail: endpoint.clone(),
                ready: endpoint_reachable(&endpoint),
            }
        }
        ProviderKind::OpenAi => DiscoveredProvider {
            kind,
            label: "OpenAI",
            default_model: "gpt-4o-mini".to_string(),
            detail: "OPENAI_API_KEY".to_string(),
            ready: env::var_os("OPENAI_API_KEY").is_some(),
        },
        ProviderKind::Anthropic => DiscoveredProvider {
            kind,
            label: "Anthropic",
            default_model: "claude-sonnet-4-5".to_string(),
            detail: "ANTHROPIC_API_KEY".to_string(),
            ready: env::var_os("ANTHROPIC_API_KEY").is_some(),
        },
        ProviderKind::Gemini => DiscoveredProvider {
            kind,
            label: "Gemini",
            default_model: "gemini-2.5-flash".to_string(),
            detail: "GEMINI_API_KEY".to_string(),
            ready: env::var_os("GEMINI_API_KEY").is_some(),
        },
        other => DiscoveredProvider {
            kind: other,
            label: "Unsupported",
            default_model: "unknown".to_string(),
            detail: "not probed by this example".to_string(),
            ready: false,
        },
    }
}

fn build_ollama_entry(runtime: &Runtime) -> DiscoveredProvider {
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

    DiscoveredProvider {
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
