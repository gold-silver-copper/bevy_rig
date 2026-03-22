use std::{
    env,
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use crate::domain::{ProviderEntry, ProviderKind, ProviderRegistry};

pub struct ProviderSeed {
    pub kind: ProviderKind,
    pub label: &'static str,
    pub default_model: &'static str,
    pub env_hint: &'static str,
    pub is_local: bool,
}

pub const CHAT_PROVIDERS: &[ProviderSeed] = &[
    ProviderSeed {
        kind: ProviderKind::Anthropic,
        label: "Anthropic",
        default_model: "claude-sonnet-4-5",
        env_hint: "ANTHROPIC_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Azure,
        label: "Azure OpenAI",
        default_model: "gpt-4o",
        env_hint: "AZURE_ENDPOINT + AZURE_API_VERSION + (AZURE_API_KEY or AZURE_TOKEN)",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Cohere,
        label: "Cohere",
        default_model: "command-r",
        env_hint: "COHERE_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::DeepSeek,
        label: "DeepSeek",
        default_model: "deepseek-chat",
        env_hint: "DEEPSEEK_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Galadriel,
        label: "Galadriel",
        default_model: "gpt-4o",
        env_hint: "GALADRIEL_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Gemini,
        label: "Gemini",
        default_model: "gemini-2.5-flash",
        env_hint: "GEMINI_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Groq,
        label: "Groq",
        default_model: "deepseek-r1-distill-llama-70b",
        env_hint: "GROQ_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::HuggingFace,
        label: "HuggingFace",
        default_model: "deepseek-ai/DeepSeek-R1-Distill-Qwen-32B",
        env_hint: "HUGGINGFACE_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Hyperbolic,
        label: "Hyperbolic",
        default_model: "deepseek-ai/DeepSeek-R1",
        env_hint: "HYPERBOLIC_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Llamafile,
        label: "Llamafile",
        default_model: "LLaMA_CPP",
        env_hint: "optional LLAMAFILE_API_BASE_URL",
        is_local: true,
    },
    ProviderSeed {
        kind: ProviderKind::Mira,
        label: "Mira",
        default_model: "gpt-4o",
        env_hint: "MIRA_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Mistral,
        label: "Mistral",
        default_model: "mistral-large-latest",
        env_hint: "MISTRAL_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Moonshot,
        label: "Moonshot",
        default_model: "moonshot-v1-128k",
        env_hint: "MOONSHOT_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Ollama,
        label: "Ollama",
        default_model: "qwen2.5:14b",
        env_hint: "optional OLLAMA_API_BASE_URL",
        is_local: true,
    },
    ProviderSeed {
        kind: ProviderKind::OpenAi,
        label: "OpenAI",
        default_model: "gpt-4o",
        env_hint: "OPENAI_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::OpenRouter,
        label: "OpenRouter",
        default_model: "openai/gpt-4o-mini",
        env_hint: "OPENROUTER_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Perplexity,
        label: "Perplexity",
        default_model: "sonar",
        env_hint: "PERPLEXITY_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::Together,
        label: "Together",
        default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        env_hint: "TOGETHER_API_KEY",
        is_local: false,
    },
    ProviderSeed {
        kind: ProviderKind::XAi,
        label: "xAI",
        default_model: "grok-3-mini",
        env_hint: "XAI_API_KEY",
        is_local: false,
    },
];

pub fn build_registry() -> ProviderRegistry {
    ProviderRegistry {
        providers: CHAT_PROVIDERS
            .iter()
            .map(|seed| ProviderEntry {
                kind: seed.kind,
                label: seed.label,
                default_model: seed.default_model,
                detail: provider_detail(seed.kind, seed.env_hint),
                is_local: seed.is_local,
                ready: provider_ready(seed.kind),
            })
            .collect(),
    }
}

pub fn provider_ready(kind: ProviderKind) -> bool {
    match kind {
        ProviderKind::Azure => {
            env::var_os("AZURE_ENDPOINT").is_some()
                && env::var_os("AZURE_API_VERSION").is_some()
                && (env::var_os("AZURE_API_KEY").is_some() || env::var_os("AZURE_TOKEN").is_some())
        }
        ProviderKind::Llamafile | ProviderKind::Ollama => {
            provider_runtime_target(kind).is_some_and(|endpoint| endpoint_reachable(&endpoint))
        }
        ProviderKind::Anthropic => env::var_os("ANTHROPIC_API_KEY").is_some(),
        ProviderKind::Cohere => env::var_os("COHERE_API_KEY").is_some(),
        ProviderKind::DeepSeek => env::var_os("DEEPSEEK_API_KEY").is_some(),
        ProviderKind::Galadriel => env::var_os("GALADRIEL_API_KEY").is_some(),
        ProviderKind::Gemini => env::var_os("GEMINI_API_KEY").is_some(),
        ProviderKind::Groq => env::var_os("GROQ_API_KEY").is_some(),
        ProviderKind::HuggingFace => env::var_os("HUGGINGFACE_API_KEY").is_some(),
        ProviderKind::Hyperbolic => env::var_os("HYPERBOLIC_API_KEY").is_some(),
        ProviderKind::Mira => env::var_os("MIRA_API_KEY").is_some(),
        ProviderKind::Mistral => env::var_os("MISTRAL_API_KEY").is_some(),
        ProviderKind::Moonshot => env::var_os("MOONSHOT_API_KEY").is_some(),
        ProviderKind::OpenAi => env::var_os("OPENAI_API_KEY").is_some(),
        ProviderKind::OpenRouter => env::var_os("OPENROUTER_API_KEY").is_some(),
        ProviderKind::Perplexity => env::var_os("PERPLEXITY_API_KEY").is_some(),
        ProviderKind::Together => env::var_os("TOGETHER_API_KEY").is_some(),
        ProviderKind::XAi => env::var_os("XAI_API_KEY").is_some(),
    }
}

pub fn provider_runtime_target(kind: ProviderKind) -> Option<String> {
    match kind {
        ProviderKind::Llamafile => Some(
            env::var("LLAMAFILE_API_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "http://localhost:8080".to_string()),
        ),
        ProviderKind::Ollama => Some(
            env::var("OLLAMA_API_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
        ),
        _ => None,
    }
}

pub fn provider_detail(kind: ProviderKind, env_hint: &str) -> String {
    provider_runtime_target(kind).unwrap_or_else(|| env_hint.to_string())
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

    if let Some((host, port_text)) = authority.rsplit_once(':')
        && !host.contains(':')
        && let Ok(port) = port_text.parse::<u16>()
    {
        return Some((host.to_string(), port));
    }

    Some((authority.to_string(), default_port))
}
