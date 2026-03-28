use bevy::prelude::*;

pub type NodeId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PortAddress {
    pub node: NodeId,
    pub direction: PortDirection,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortType {
    AgentName,
    AgentDescription,
    Model,
    Preamble,
    StaticContext,
    Temperature,
    MaxTokens,
    AdditionalParams,
    ToolServerHandle,
    DynamicContext,
    ToolChoice,
    DefaultMaxTurns,
    Hook,
    OutputSchema,
    Prompt,
    TextValue,
    TextResponse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueType {
    Text,
    ModelRef,
    Number,
    Integer,
    Json,
    OpaqueText,
    ToolChoice,
    Schema,
    TextResponse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortSpec {
    pub name: &'static str,
    pub ty: PortType,
    pub value_type: ValueType,
    pub required: bool,
    pub accepts_multiple: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolChoiceSetting {
    Auto,
    None,
    Required,
}

impl ToolChoiceSetting {
    pub const ALL: [ToolChoiceSetting; 3] = [Self::Auto, Self::None, Self::Required];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Required => "required",
        }
    }

    pub fn shifted(self, delta: i32) -> Self {
        let current = Self::ALL
            .iter()
            .position(|choice| *choice == self)
            .unwrap_or(0) as i32;
        let len = Self::ALL.len() as i32;
        let next = (current + delta).rem_euclid(len) as usize;
        Self::ALL[next]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeType {
    Agent,
    Text,
    TextOutput,
    Model,
    Temperature,
    MaxTokens,
    AdditionalParams,
    ToolServerHandle,
    DynamicContext,
    ToolChoice,
    DefaultMaxTurns,
    Hook,
    OutputSchema,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeValue {
    None,
    Text(String),
    TextOutput {
        text: String,
        status: String,
    },
    Model {
        provider_label: String,
        model_name: Option<String>,
    },
    Temperature(f64),
    MaxTokens(u64),
    AdditionalParams(String),
    ToolServerHandle(String),
    DynamicContext(String),
    ToolChoice(ToolChoiceSetting),
    DefaultMaxTurns(usize),
    Hook(String),
    OutputSchema(String),
}

impl NodeValue {
    pub fn editable_text(&self) -> Option<&str> {
        match self {
            Self::Text(text)
            | Self::AdditionalParams(text)
            | Self::ToolServerHandle(text)
            | Self::DynamicContext(text)
            | Self::Hook(text)
            | Self::OutputSchema(text) => Some(text.as_str()),
            Self::TextOutput { text, .. } => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn set_text_value(&mut self, value: String) -> bool {
        match self {
            Self::Text(text)
            | Self::AdditionalParams(text)
            | Self::ToolServerHandle(text)
            | Self::DynamicContext(text)
            | Self::Hook(text)
            | Self::OutputSchema(text) => {
                *text = value;
                true
            }
            Self::TextOutput { text, .. } => {
                *text = value;
                true
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeTemplate {
    Agent,
    PromptText,
    TextOutput,
    NameText,
    DescriptionText,
    Model,
    PreambleText,
    StaticContextText,
    Temperature,
    MaxTokens,
    AdditionalParams,
    ToolServerHandle,
    DynamicContext,
    ToolChoice,
    DefaultMaxTurns,
    Hook,
    OutputSchema,
}

impl NodeTemplate {
    pub const ALL: [NodeTemplate; 17] = [
        NodeTemplate::Agent,
        NodeTemplate::PromptText,
        NodeTemplate::TextOutput,
        NodeTemplate::Model,
        NodeTemplate::NameText,
        NodeTemplate::DescriptionText,
        NodeTemplate::PreambleText,
        NodeTemplate::StaticContextText,
        NodeTemplate::Temperature,
        NodeTemplate::MaxTokens,
        NodeTemplate::AdditionalParams,
        NodeTemplate::ToolServerHandle,
        NodeTemplate::DynamicContext,
        NodeTemplate::ToolChoice,
        NodeTemplate::DefaultMaxTurns,
        NodeTemplate::Hook,
        NodeTemplate::OutputSchema,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::PromptText => "Prompt",
            Self::TextOutput => "Text Output",
            Self::NameText => "Name",
            Self::DescriptionText => "Description",
            Self::Model => "Model",
            Self::PreambleText => "Preamble",
            Self::StaticContextText => "Static Context",
            Self::Temperature => "Temperature",
            Self::MaxTokens => "Max Tokens",
            Self::AdditionalParams => "Additional Params",
            Self::ToolServerHandle => "Tool Server Handle",
            Self::DynamicContext => "Dynamic Context",
            Self::ToolChoice => "Tool Choice",
            Self::DefaultMaxTurns => "Default Max Turns",
            Self::Hook => "Hook",
            Self::OutputSchema => "Output Schema",
        }
    }

    pub fn instantiate(self) -> NodeSeed {
        match self {
            Self::Agent => NodeSeed {
                node_type: NodeType::Agent,
                title: "Agent".into(),
                value: NodeValue::None,
            },
            Self::PromptText => NodeSeed {
                node_type: NodeType::Text,
                title: "Prompt".into(),
                value: NodeValue::Text(
                    "Write a warm dwarven greeting for the alehall and ask what brew the guest wants."
                        .into(),
                ),
            },
            Self::TextOutput => NodeSeed {
                node_type: NodeType::TextOutput,
                title: "Text Output".into(),
                value: NodeValue::TextOutput {
                    text: "Run the selected agent to populate this sink.".into(),
                    status: "idle".into(),
                },
            },
            Self::NameText => NodeSeed {
                node_type: NodeType::Text,
                title: "Name".into(),
                value: NodeValue::Text("Hall Greeter".into()),
            },
            Self::DescriptionText => NodeSeed {
                node_type: NodeType::Text,
                title: "Description".into(),
                value: NodeValue::Text("Greets guests and recommends a fitting brew.".into()),
            },
            Self::Model => NodeSeed {
                node_type: NodeType::Model,
                title: "Model".into(),
                value: NodeValue::Model {
                    provider_label: "Ollama".into(),
                    model_name: None,
                },
            },
            Self::PreambleText => NodeSeed {
                node_type: NodeType::Text,
                title: "Preamble".into(),
                value: NodeValue::Text(
                    "You are a merry dwarf host in a loud mountain alehall. Reply briefly, concretely, and in character."
                        .into(),
                ),
            },
            Self::StaticContextText => NodeSeed {
                node_type: NodeType::Text,
                title: "Static Context".into(),
                value: NodeValue::Text(
                    "The alehall serves frothy ale, berry mead, root cider, cave-wheat stout, and whatever else the brewers can coax into a keg."
                        .into(),
                ),
            },
            Self::Temperature => NodeSeed {
                node_type: NodeType::Temperature,
                title: "Temperature".into(),
                value: NodeValue::Temperature(0.7),
            },
            Self::MaxTokens => NodeSeed {
                node_type: NodeType::MaxTokens,
                title: "Max Tokens".into(),
                value: NodeValue::MaxTokens(192),
            },
            Self::AdditionalParams => NodeSeed {
                node_type: NodeType::AdditionalParams,
                title: "Additional Params".into(),
                value: NodeValue::AdditionalParams("{\n  \"think\": false\n}".into()),
            },
            Self::ToolServerHandle => NodeSeed {
                node_type: NodeType::ToolServerHandle,
                title: "Tool Server Handle".into(),
                value: NodeValue::ToolServerHandle("stored for future tool bridge".into()),
            },
            Self::DynamicContext => NodeSeed {
                node_type: NodeType::DynamicContext,
                title: "Dynamic Context".into(),
                value: NodeValue::DynamicContext("stored for future vector index binding".into()),
            },
            Self::ToolChoice => NodeSeed {
                node_type: NodeType::ToolChoice,
                title: "Tool Choice".into(),
                value: NodeValue::ToolChoice(ToolChoiceSetting::Auto),
            },
            Self::DefaultMaxTurns => NodeSeed {
                node_type: NodeType::DefaultMaxTurns,
                title: "Default Max Turns".into(),
                value: NodeValue::DefaultMaxTurns(4),
            },
            Self::Hook => NodeSeed {
                node_type: NodeType::Hook,
                title: "Hook".into(),
                value: NodeValue::Hook("stored for future request hook".into()),
            },
            Self::OutputSchema => NodeSeed {
                node_type: NodeType::OutputSchema,
                title: "Output Schema".into(),
                value: NodeValue::OutputSchema(
                    "{\n  \"type\": \"object\",\n  \"properties\": {\n    \"reply\": { \"type\": \"string\" }\n  },\n  \"required\": [\"reply\"]\n}"
                        .into(),
                ),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeSeed {
    pub node_type: NodeType,
    pub title: String,
    pub value: NodeValue,
}

const AGENT_INPUTS: [PortSpec; 15] = [
    PortSpec {
        name: "model",
        ty: PortType::Model,
        value_type: ValueType::ModelRef,
        required: true,
        accepts_multiple: false,
    },
    PortSpec {
        name: "prompt",
        ty: PortType::Prompt,
        value_type: ValueType::Text,
        required: true,
        accepts_multiple: false,
    },
    PortSpec {
        name: "name",
        ty: PortType::AgentName,
        value_type: ValueType::Text,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "description",
        ty: PortType::AgentDescription,
        value_type: ValueType::Text,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "preamble",
        ty: PortType::Preamble,
        value_type: ValueType::Text,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "static_context",
        ty: PortType::StaticContext,
        value_type: ValueType::Text,
        required: false,
        accepts_multiple: true,
    },
    PortSpec {
        name: "temperature",
        ty: PortType::Temperature,
        value_type: ValueType::Number,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "max_tokens",
        ty: PortType::MaxTokens,
        value_type: ValueType::Integer,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "additional_params",
        ty: PortType::AdditionalParams,
        value_type: ValueType::Json,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "tool_server_handle",
        ty: PortType::ToolServerHandle,
        value_type: ValueType::OpaqueText,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "dynamic_context",
        ty: PortType::DynamicContext,
        value_type: ValueType::OpaqueText,
        required: false,
        accepts_multiple: true,
    },
    PortSpec {
        name: "tool_choice",
        ty: PortType::ToolChoice,
        value_type: ValueType::ToolChoice,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "default_max_turns",
        ty: PortType::DefaultMaxTurns,
        value_type: ValueType::Integer,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "hook",
        ty: PortType::Hook,
        value_type: ValueType::OpaqueText,
        required: false,
        accepts_multiple: false,
    },
    PortSpec {
        name: "output_schema",
        ty: PortType::OutputSchema,
        value_type: ValueType::Schema,
        required: false,
        accepts_multiple: false,
    },
];

const AGENT_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "text",
    ty: PortType::TextResponse,
    value_type: ValueType::TextResponse,
    required: true,
    accepts_multiple: false,
}];

const TEXT_OUTPUT_INPUTS: [PortSpec; 1] = [PortSpec {
    name: "text",
    ty: PortType::TextResponse,
    value_type: ValueType::TextResponse,
    required: true,
    accepts_multiple: false,
}];

const TEXT_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "text",
    ty: PortType::TextValue,
    value_type: ValueType::Text,
    required: false,
    accepts_multiple: false,
}];

const MODEL_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "model",
    ty: PortType::Model,
    value_type: ValueType::ModelRef,
    required: true,
    accepts_multiple: false,
}];

const TEMP_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "temperature",
    ty: PortType::Temperature,
    value_type: ValueType::Number,
    required: false,
    accepts_multiple: false,
}];

const MAX_TOKENS_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "max_tokens",
    ty: PortType::MaxTokens,
    value_type: ValueType::Integer,
    required: false,
    accepts_multiple: false,
}];

const ADDITIONAL_PARAMS_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "params",
    ty: PortType::AdditionalParams,
    value_type: ValueType::Json,
    required: false,
    accepts_multiple: false,
}];

const TOOL_SERVER_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "tool_server_handle",
    ty: PortType::ToolServerHandle,
    value_type: ValueType::OpaqueText,
    required: false,
    accepts_multiple: false,
}];

const DYNAMIC_CONTEXT_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "dynamic_context",
    ty: PortType::DynamicContext,
    value_type: ValueType::OpaqueText,
    required: false,
    accepts_multiple: false,
}];

const TOOL_CHOICE_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "tool_choice",
    ty: PortType::ToolChoice,
    value_type: ValueType::ToolChoice,
    required: false,
    accepts_multiple: false,
}];

const DEFAULT_MAX_TURNS_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "default_max_turns",
    ty: PortType::DefaultMaxTurns,
    value_type: ValueType::Integer,
    required: false,
    accepts_multiple: false,
}];

const HOOK_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "hook",
    ty: PortType::Hook,
    value_type: ValueType::OpaqueText,
    required: false,
    accepts_multiple: false,
}];

const OUTPUT_SCHEMA_OUTPUTS: [PortSpec; 1] = [PortSpec {
    name: "schema",
    ty: PortType::OutputSchema,
    value_type: ValueType::Schema,
    required: false,
    accepts_multiple: false,
}];

pub fn node_inputs(node_type: NodeType) -> &'static [PortSpec] {
    match node_type {
        NodeType::Agent => &AGENT_INPUTS,
        NodeType::TextOutput => &TEXT_OUTPUT_INPUTS,
        _ => &[],
    }
}

pub fn node_outputs(node_type: NodeType) -> &'static [PortSpec] {
    match node_type {
        NodeType::Agent => &AGENT_OUTPUTS,
        NodeType::Text => &TEXT_OUTPUTS,
        NodeType::Model => &MODEL_OUTPUTS,
        NodeType::Temperature => &TEMP_OUTPUTS,
        NodeType::MaxTokens => &MAX_TOKENS_OUTPUTS,
        NodeType::AdditionalParams => &ADDITIONAL_PARAMS_OUTPUTS,
        NodeType::ToolServerHandle => &TOOL_SERVER_OUTPUTS,
        NodeType::DynamicContext => &DYNAMIC_CONTEXT_OUTPUTS,
        NodeType::ToolChoice => &TOOL_CHOICE_OUTPUTS,
        NodeType::DefaultMaxTurns => &DEFAULT_MAX_TURNS_OUTPUTS,
        NodeType::Hook => &HOOK_OUTPUTS,
        NodeType::OutputSchema => &OUTPUT_SCHEMA_OUTPUTS,
        NodeType::TextOutput => &[],
    }
}

pub fn preview_lines(value: &str, prefix: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if value.trim().is_empty() {
        lines.push(format!("{prefix} = (empty)"));
        return lines;
    }

    for line in value.lines().take(3) {
        lines.push(preview_line(line));
    }

    if value.lines().count() > 3 {
        lines.push("…".into());
    }

    lines
}

pub fn preview_line(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() > 52 {
        let preview: String = trimmed.chars().take(52).collect();
        format!("{preview}…")
    } else {
        trimmed.to_string()
    }
}
