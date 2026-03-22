use bevy::prelude::Resource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Anthropic,
    Azure,
    Cohere,
    DeepSeek,
    Galadriel,
    Gemini,
    Groq,
    HuggingFace,
    Hyperbolic,
    Llamafile,
    Mira,
    Mistral,
    Moonshot,
    Ollama,
    OpenAi,
    OpenRouter,
    Perplexity,
    Together,
    XAi,
}

#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub kind: ProviderKind,
    pub label: &'static str,
    pub default_model: &'static str,
    pub detail: String,
    pub is_local: bool,
    pub ready: bool,
}

#[derive(Resource, Default)]
pub struct ProviderRegistry {
    pub providers: Vec<ProviderEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Resource)]
pub struct ChatState {
    pub selected_provider: Option<ProviderKind>,
    pub draft: String,
    pub history: Vec<ChatMessage>,
    pub selected_message: Option<usize>,
    pub sending: bool,
    pub status: Option<String>,
    pub logs: Vec<String>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            selected_provider: None,
            draft: String::new(),
            history: Vec::new(),
            selected_message: None,
            sending: false,
            status: None,
            logs: Vec::new(),
        }
    }
}

impl ChatState {
    pub fn push_log(&mut self, line: impl Into<String>) {
        self.logs.push(line.into());
        if self.logs.len() > 32 {
            let overflow = self.logs.len() - 32;
            self.logs.drain(..overflow);
        }
    }
}
