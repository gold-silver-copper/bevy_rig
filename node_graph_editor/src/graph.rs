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
    TextResponse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortSpec {
    pub name: &'static str,
    pub ty: PortType,
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

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Agent,
    PromptInput {
        text: String,
    },
    TextOutput {
        title: String,
        text: String,
        status: String,
    },
    Name {
        value: String,
    },
    Description {
        value: String,
    },
    Model {
        provider_label: String,
        model_name: Option<String>,
    },
    Preamble {
        value: String,
    },
    StaticContext {
        value: String,
    },
    Temperature {
        value: f64,
    },
    MaxTokens {
        value: u64,
    },
    AdditionalParams {
        value: String,
    },
    ToolServerHandle {
        value: String,
    },
    DynamicContext {
        value: String,
    },
    ToolChoice {
        value: ToolChoiceSetting,
    },
    DefaultMaxTurns {
        value: usize,
    },
    Hook {
        value: String,
    },
    OutputSchema {
        value: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeTemplate {
    Agent,
    PromptInput,
    TextOutput,
    Name,
    Description,
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
}

impl NodeTemplate {
    pub const ALL: [NodeTemplate; 17] = [
        NodeTemplate::Agent,
        NodeTemplate::PromptInput,
        NodeTemplate::TextOutput,
        NodeTemplate::Model,
        NodeTemplate::Name,
        NodeTemplate::Description,
        NodeTemplate::Preamble,
        NodeTemplate::StaticContext,
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
            Self::PromptInput => "Prompt",
            Self::TextOutput => "Text Output",
            Self::Name => "Name",
            Self::Description => "Description",
            Self::Model => "Model",
            Self::Preamble => "Preamble",
            Self::StaticContext => "Static Context",
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

    pub fn instantiate(self) -> NodeKind {
        match self {
            Self::Agent => NodeKind::Agent,
            Self::PromptInput => NodeKind::PromptInput {
                text: "Write a warm dwarven greeting for the alehall and ask what brew the guest wants.".into(),
            },
            Self::TextOutput => NodeKind::TextOutput {
                title: "Text Output".into(),
                text: "Run the selected agent to populate this sink.".into(),
                status: "idle".into(),
            },
            Self::Name => NodeKind::Name {
                value: "Hall Greeter".into(),
            },
            Self::Description => NodeKind::Description {
                value: "Greets guests and recommends a fitting brew.".into(),
            },
            Self::Model => NodeKind::Model {
                provider_label: "Ollama".into(),
                model_name: None,
            },
            Self::Preamble => NodeKind::Preamble {
                value: "You are a merry dwarf host in a loud mountain alehall. Reply briefly, concretely, and in character.".into(),
            },
            Self::StaticContext => NodeKind::StaticContext {
                value: "The alehall serves frothy ale, berry mead, root cider, cave-wheat stout, and whatever else the brewers can coax into a keg.".into(),
            },
            Self::Temperature => NodeKind::Temperature { value: 0.7 },
            Self::MaxTokens => NodeKind::MaxTokens { value: 192 },
            Self::AdditionalParams => NodeKind::AdditionalParams {
                value: "{\n  \"think\": false\n}".into(),
            },
            Self::ToolServerHandle => NodeKind::ToolServerHandle {
                value: "stored for future tool bridge".into(),
            },
            Self::DynamicContext => NodeKind::DynamicContext {
                value: "stored for future vector index binding".into(),
            },
            Self::ToolChoice => NodeKind::ToolChoice {
                value: ToolChoiceSetting::Auto,
            },
            Self::DefaultMaxTurns => NodeKind::DefaultMaxTurns { value: 4 },
            Self::Hook => NodeKind::Hook {
                value: "stored for future request hook".into(),
            },
            Self::OutputSchema => NodeKind::OutputSchema {
                value: "{\n  \"type\": \"object\",\n  \"properties\": {\n    \"reply\": { \"type\": \"string\" }\n  },\n  \"required\": [\"reply\"]\n}".into(),
            },
        }
    }
}

impl NodeKind {
    pub const fn title(&self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::PromptInput { .. } => "Prompt",
            Self::TextOutput { .. } => "Text Output",
            Self::Name { .. } => "Name",
            Self::Description { .. } => "Description",
            Self::Model { .. } => "Model",
            Self::Preamble { .. } => "Preamble",
            Self::StaticContext { .. } => "Static Context",
            Self::Temperature { .. } => "Temperature",
            Self::MaxTokens { .. } => "Max Tokens",
            Self::AdditionalParams { .. } => "Additional Params",
            Self::ToolServerHandle { .. } => "Tool Server Handle",
            Self::DynamicContext { .. } => "Dynamic Context",
            Self::ToolChoice { .. } => "Tool Choice",
            Self::DefaultMaxTurns { .. } => "Default Max Turns",
            Self::Hook { .. } => "Hook",
            Self::OutputSchema { .. } => "Output Schema",
        }
    }

    pub fn inputs(&self) -> Vec<PortSpec> {
        match self {
            Self::Agent => vec![
                PortSpec {
                    name: "model",
                    ty: PortType::Model,
                    required: true,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "prompt",
                    ty: PortType::Prompt,
                    required: true,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "name",
                    ty: PortType::AgentName,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "description",
                    ty: PortType::AgentDescription,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "preamble",
                    ty: PortType::Preamble,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "static_context",
                    ty: PortType::StaticContext,
                    required: false,
                    accepts_multiple: true,
                },
                PortSpec {
                    name: "temperature",
                    ty: PortType::Temperature,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "max_tokens",
                    ty: PortType::MaxTokens,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "additional_params",
                    ty: PortType::AdditionalParams,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "tool_server_handle",
                    ty: PortType::ToolServerHandle,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "dynamic_context",
                    ty: PortType::DynamicContext,
                    required: false,
                    accepts_multiple: true,
                },
                PortSpec {
                    name: "tool_choice",
                    ty: PortType::ToolChoice,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "default_max_turns",
                    ty: PortType::DefaultMaxTurns,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "hook",
                    ty: PortType::Hook,
                    required: false,
                    accepts_multiple: false,
                },
                PortSpec {
                    name: "output_schema",
                    ty: PortType::OutputSchema,
                    required: false,
                    accepts_multiple: false,
                },
            ],
            Self::TextOutput { .. } => vec![PortSpec {
                name: "text",
                ty: PortType::TextResponse,
                required: true,
                accepts_multiple: false,
            }],
            _ => Vec::new(),
        }
    }

    pub fn outputs(&self) -> Vec<PortSpec> {
        match self {
            Self::Agent => vec![PortSpec {
                name: "text",
                ty: PortType::TextResponse,
                required: true,
                accepts_multiple: false,
            }],
            Self::PromptInput { .. } => vec![PortSpec {
                name: "prompt",
                ty: PortType::Prompt,
                required: true,
                accepts_multiple: false,
            }],
            Self::Name { .. } => vec![PortSpec {
                name: "name",
                ty: PortType::AgentName,
                required: false,
                accepts_multiple: false,
            }],
            Self::Description { .. } => vec![PortSpec {
                name: "description",
                ty: PortType::AgentDescription,
                required: false,
                accepts_multiple: false,
            }],
            Self::Model { .. } => vec![PortSpec {
                name: "model",
                ty: PortType::Model,
                required: true,
                accepts_multiple: false,
            }],
            Self::Preamble { .. } => vec![PortSpec {
                name: "preamble",
                ty: PortType::Preamble,
                required: false,
                accepts_multiple: false,
            }],
            Self::StaticContext { .. } => vec![PortSpec {
                name: "document",
                ty: PortType::StaticContext,
                required: false,
                accepts_multiple: false,
            }],
            Self::Temperature { .. } => vec![PortSpec {
                name: "temperature",
                ty: PortType::Temperature,
                required: false,
                accepts_multiple: false,
            }],
            Self::MaxTokens { .. } => vec![PortSpec {
                name: "max_tokens",
                ty: PortType::MaxTokens,
                required: false,
                accepts_multiple: false,
            }],
            Self::AdditionalParams { .. } => vec![PortSpec {
                name: "params",
                ty: PortType::AdditionalParams,
                required: false,
                accepts_multiple: false,
            }],
            Self::ToolServerHandle { .. } => vec![PortSpec {
                name: "tool_server_handle",
                ty: PortType::ToolServerHandle,
                required: false,
                accepts_multiple: false,
            }],
            Self::DynamicContext { .. } => vec![PortSpec {
                name: "dynamic_context",
                ty: PortType::DynamicContext,
                required: false,
                accepts_multiple: false,
            }],
            Self::ToolChoice { .. } => vec![PortSpec {
                name: "tool_choice",
                ty: PortType::ToolChoice,
                required: false,
                accepts_multiple: false,
            }],
            Self::DefaultMaxTurns { .. } => vec![PortSpec {
                name: "default_max_turns",
                ty: PortType::DefaultMaxTurns,
                required: false,
                accepts_multiple: false,
            }],
            Self::Hook { .. } => vec![PortSpec {
                name: "hook",
                ty: PortType::Hook,
                required: false,
                accepts_multiple: false,
            }],
            Self::OutputSchema { .. } => vec![PortSpec {
                name: "schema",
                ty: PortType::OutputSchema,
                required: false,
                accepts_multiple: false,
            }],
            Self::TextOutput { .. } => Vec::new(),
        }
    }

    pub fn summary_lines(&self) -> Vec<String> {
        match self {
            Self::Agent => vec![
                "Ephemeral Rig agent compiled from incoming nodes.".into(),
                "Requires: model + prompt + text output.".into(),
            ],
            Self::PromptInput { text } => preview_lines(text, "prompt"),
            Self::TextOutput { text, status, .. } => {
                let mut lines = vec![format!("status = {}", status)];
                lines.extend(preview_lines(text, "output"));
                lines
            }
            Self::Name { value } => preview_lines(value, "agent name"),
            Self::Description { value } => preview_lines(value, "description"),
            Self::Model {
                provider_label,
                model_name,
            } => vec![
                format!("provider = {}", provider_label),
                format!(
                    "model = {}",
                    model_name.as_deref().unwrap_or("(discovering locally)")
                ),
            ],
            Self::Preamble { value } => preview_lines(value, "system prompt"),
            Self::StaticContext { value } => preview_lines(value, "document"),
            Self::Temperature { value } => vec![format!("value = {:.1}", value)],
            Self::MaxTokens { value } => vec![format!("value = {}", value)],
            Self::AdditionalParams { value } => preview_lines(value, "json"),
            Self::ToolServerHandle { value } => {
                vec!["stored only in this MVP".into(), preview_line(value)]
            }
            Self::DynamicContext { value } => {
                vec!["stored only in this MVP".into(), preview_line(value)]
            }
            Self::ToolChoice { value } => vec![
                format!("value = {}", value.label()),
                "Ollama ignores this today.".into(),
            ],
            Self::DefaultMaxTurns { value } => vec![format!("value = {}", value)],
            Self::Hook { value } => vec!["stored only in this MVP".into(), preview_line(value)],
            Self::OutputSchema { value } => preview_lines(value, "schema"),
        }
    }

    pub fn editable_text(&self) -> Option<&str> {
        match self {
            Self::PromptInput { text }
            | Self::Name { value: text }
            | Self::Description { value: text }
            | Self::Preamble { value: text }
            | Self::StaticContext { value: text }
            | Self::AdditionalParams { value: text }
            | Self::ToolServerHandle { value: text }
            | Self::DynamicContext { value: text }
            | Self::Hook { value: text }
            | Self::OutputSchema { value: text } => Some(text.as_str()),
            Self::TextOutput { text, .. } => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn set_text_value(&mut self, value: String) -> bool {
        match self {
            Self::PromptInput { text }
            | Self::Name { value: text }
            | Self::Description { value: text }
            | Self::Preamble { value: text }
            | Self::StaticContext { value: text }
            | Self::AdditionalParams { value: text }
            | Self::ToolServerHandle { value: text }
            | Self::DynamicContext { value: text }
            | Self::Hook { value: text }
            | Self::OutputSchema { value: text } => {
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

fn preview_lines(value: &str, prefix: &str) -> Vec<String> {
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

fn preview_line(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() > 52 {
        let preview: String = trimmed.chars().take(52).collect();
        format!("{preview}…")
    } else {
        trimmed.to_string()
    }
}

#[derive(Clone, Debug)]
pub struct GraphNode {
    pub id: NodeId,
    pub position: Vec2,
    pub kind: NodeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub from: PortAddress,
    pub to: PortAddress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraggingWire {
    pub from: PortAddress,
    pub ty: PortType,
}

#[derive(Resource, Clone, Debug)]
pub struct GraphEditorState {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub pan: Vec2,
    pub zoom: f32,
    pub selected_node: Option<NodeId>,
    pub dragging_wire: Option<DraggingWire>,
    pub revision: u64,
    next_node_id: NodeId,
}

impl Default for GraphEditorState {
    fn default() -> Self {
        Self::demo()
    }
}

impl GraphEditorState {
    pub fn demo() -> Self {
        let mut graph = Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            pan: Vec2::new(360.0, 120.0),
            zoom: 1.0,
            selected_node: None,
            dragging_wire: None,
            revision: 1,
            next_node_id: 1,
        };

        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::new(760.0, 220.0));
        let output = graph.add_node(
            NodeTemplate::TextOutput.instantiate(),
            Vec2::new(1210.0, 260.0),
        );
        let prompt = graph.add_node(
            NodeTemplate::PromptInput.instantiate(),
            Vec2::new(260.0, 140.0),
        );
        let model = graph.add_node(NodeTemplate::Model.instantiate(), Vec2::new(260.0, 350.0));
        let name = graph.add_node(NodeTemplate::Name.instantiate(), Vec2::new(260.0, 20.0));
        let description = graph.add_node(
            NodeTemplate::Description.instantiate(),
            Vec2::new(260.0, 560.0),
        );
        let preamble = graph.add_node(NodeTemplate::Preamble.instantiate(), Vec2::new(20.0, 140.0));
        let context = graph.add_node(
            NodeTemplate::StaticContext.instantiate(),
            Vec2::new(20.0, 380.0),
        );
        let temperature = graph.add_node(
            NodeTemplate::Temperature.instantiate(),
            Vec2::new(520.0, 20.0),
        );
        let max_tokens = graph.add_node(
            NodeTemplate::MaxTokens.instantiate(),
            Vec2::new(520.0, 560.0),
        );
        let additional_params = graph.add_node(
            NodeTemplate::AdditionalParams.instantiate(),
            Vec2::new(520.0, 700.0),
        );

        graph.connect_ports(name, PortType::AgentName, agent, PortType::AgentName);
        graph.connect_ports(prompt, PortType::Prompt, agent, PortType::Prompt);
        graph.connect_ports(model, PortType::Model, agent, PortType::Model);
        graph.connect_ports(
            description,
            PortType::AgentDescription,
            agent,
            PortType::AgentDescription,
        );
        graph.connect_ports(preamble, PortType::Preamble, agent, PortType::Preamble);
        graph.connect_ports(
            context,
            PortType::StaticContext,
            agent,
            PortType::StaticContext,
        );
        graph.connect_ports(
            temperature,
            PortType::Temperature,
            agent,
            PortType::Temperature,
        );
        graph.connect_ports(max_tokens, PortType::MaxTokens, agent, PortType::MaxTokens);
        graph.connect_ports(
            additional_params,
            PortType::AdditionalParams,
            agent,
            PortType::AdditionalParams,
        );
        graph.connect_ports(
            agent,
            PortType::TextResponse,
            output,
            PortType::TextResponse,
        );

        graph.selected_node = Some(agent);
        graph
    }

    pub fn reset_demo(&mut self) {
        *self = Self::demo();
    }

    pub fn add_node(&mut self, kind: NodeKind, position: Vec2) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.push(GraphNode { id, position, kind });
        self.touch();
        id
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn move_node(&mut self, node_id: NodeId, delta: Vec2) {
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
            node.position += delta;
        }
    }

    pub fn node(&self, node_id: NodeId) -> Option<&GraphNode> {
        self.nodes.iter().find(|node| node.id == node_id)
    }

    pub fn node_kind(&self, node_id: NodeId) -> Option<&NodeKind> {
        self.node(node_id).map(|node| &node.kind)
    }

    pub fn port_spec(&self, address: PortAddress) -> Option<PortSpec> {
        let node = self.node(address.node)?;
        match address.direction {
            PortDirection::Input => node.kind.inputs().get(address.index).copied(),
            PortDirection::Output => node.kind.outputs().get(address.index).copied(),
        }
    }

    pub fn input_port_index(&self, node_id: NodeId, ty: PortType) -> Option<usize> {
        self.node(node_id)?
            .kind
            .inputs()
            .iter()
            .position(|spec| spec.ty == ty)
    }

    pub fn output_port_index(&self, node_id: NodeId, ty: PortType) -> Option<usize> {
        self.node(node_id)?
            .kind
            .outputs()
            .iter()
            .position(|spec| spec.ty == ty)
    }

    pub fn connect_ports(
        &mut self,
        from_node: NodeId,
        from_type: PortType,
        to_node: NodeId,
        to_type: PortType,
    ) -> bool {
        let Some(from_index) = self.output_port_index(from_node, from_type) else {
            return false;
        };
        let Some(to_index) = self.input_port_index(to_node, to_type) else {
            return false;
        };
        self.connect(
            PortAddress {
                node: from_node,
                direction: PortDirection::Output,
                index: from_index,
            },
            PortAddress {
                node: to_node,
                direction: PortDirection::Input,
                index: to_index,
            },
        )
    }

    pub fn connect(&mut self, a: PortAddress, b: PortAddress) -> bool {
        let (from, to) = match (a.direction, b.direction) {
            (PortDirection::Output, PortDirection::Input) => (a, b),
            (PortDirection::Input, PortDirection::Output) => (b, a),
            _ => return false,
        };

        if !self.can_connect(from, to) {
            return false;
        }

        let accepts_multiple = self
            .port_spec(to)
            .map(|spec| spec.accepts_multiple)
            .unwrap_or(false);

        if !accepts_multiple {
            self.edges.retain(|edge| edge.to != to);
        }

        let edge = GraphEdge { from, to };
        if !self.edges.contains(&edge) {
            self.edges.push(edge);
            self.touch();
        }
        true
    }

    pub fn can_connect(&self, from: PortAddress, to: PortAddress) -> bool {
        if from.node == to.node {
            return false;
        }

        if from.direction != PortDirection::Output || to.direction != PortDirection::Input {
            return false;
        }

        let Some(from_spec) = self.port_spec(from) else {
            return false;
        };
        let Some(to_spec) = self.port_spec(to) else {
            return false;
        };

        from_spec.ty == to_spec.ty
    }

    pub fn input_sources(&self, node_id: NodeId, ty: PortType) -> Vec<NodeId> {
        let Some(index) = self.input_port_index(node_id, ty) else {
            return Vec::new();
        };

        self.edges
            .iter()
            .filter(|edge| edge.to.node == node_id && edge.to.index == index)
            .map(|edge| edge.from.node)
            .collect()
    }

    pub fn output_targets(&self, node_id: NodeId, ty: PortType) -> Vec<NodeId> {
        let Some(index) = self.output_port_index(node_id, ty) else {
            return Vec::new();
        };

        self.edges
            .iter()
            .filter(|edge| edge.from.node == node_id && edge.from.index == index)
            .map(|edge| edge.to.node)
            .collect()
    }

    pub fn selected_text_value(&self) -> Option<&str> {
        let selected = self.selected_node?;
        self.node_kind(selected)?.editable_text()
    }

    pub fn set_node_text_value(&mut self, node_id: NodeId, value: String) -> bool {
        let changed = self
            .nodes
            .iter_mut()
            .find(|node| node.id == node_id)
            .map(|node| node.kind.set_text_value(value))
            .unwrap_or(false);

        if changed {
            self.touch();
        }

        changed
    }

    pub fn cycle_selected_setting(&mut self, delta: i32, available_models: &[String]) -> bool {
        let Some(selected) = self.selected_node else {
            return false;
        };

        let changed = self
            .nodes
            .iter_mut()
            .find(|node| node.id == selected)
            .map(|node| match &mut node.kind {
                NodeKind::Model { model_name, .. } => {
                    if available_models.is_empty() {
                        return false;
                    }
                    let current = model_name
                        .as_ref()
                        .and_then(|model| available_models.iter().position(|item| item == model))
                        .unwrap_or(0) as i32;
                    let len = available_models.len() as i32;
                    let next = (current + delta).rem_euclid(len) as usize;
                    *model_name = Some(available_models[next].clone());
                    true
                }
                NodeKind::Temperature { value } => {
                    *value = (*value + (delta as f64 * 0.1)).clamp(0.0, 2.0);
                    true
                }
                NodeKind::MaxTokens { value } => {
                    let next = (*value as i64 + (delta as i64 * 64)).clamp(32, 8192);
                    *value = next as u64;
                    true
                }
                NodeKind::ToolChoice { value } => {
                    *value = value.shifted(delta);
                    true
                }
                NodeKind::DefaultMaxTurns { value } => {
                    let next = (*value as i32 + delta).clamp(1, 16);
                    *value = next as usize;
                    true
                }
                _ => false,
            })
            .unwrap_or(false);

        if changed {
            self.touch();
        }

        changed
    }

    pub fn apply_ollama_models(&mut self, models: &[String]) {
        if models.is_empty() {
            return;
        }

        let mut changed = false;
        for node in &mut self.nodes {
            if let NodeKind::Model { model_name, .. } = &mut node.kind {
                let needs_default = model_name
                    .as_ref()
                    .map(|value| !models.contains(value))
                    .unwrap_or(true);
                if needs_default {
                    *model_name = Some(models[0].clone());
                    changed = true;
                }
            }
        }

        if changed {
            self.touch();
        }
    }

    pub fn set_output_result(&mut self, node_id: NodeId, text: String, status: String) {
        if let Some(NodeKind::TextOutput {
            text: current_text,
            status: current_status,
            ..
        }) = self
            .nodes
            .iter_mut()
            .find(|node| node.id == node_id)
            .map(|node| &mut node.kind)
        {
            *current_text = text;
            *current_status = status;
            self.touch();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_port_types_connect() {
        let mut graph = GraphEditorState::demo();
        let prompt = graph.add_node(NodeTemplate::PromptInput.instantiate(), Vec2::ZERO);
        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::ZERO);
        assert!(graph.connect_ports(prompt, PortType::Prompt, agent, PortType::Prompt));
    }

    #[test]
    fn multi_input_keeps_multiple_context_edges() {
        let mut graph = GraphEditorState::demo();
        let a = graph.add_node(NodeTemplate::StaticContext.instantiate(), Vec2::ZERO);
        let b = graph.add_node(NodeTemplate::StaticContext.instantiate(), Vec2::ZERO);
        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::ZERO);

        assert!(graph.connect_ports(a, PortType::StaticContext, agent, PortType::StaticContext));
        assert!(graph.connect_ports(b, PortType::StaticContext, agent, PortType::StaticContext));
        assert_eq!(graph.input_sources(agent, PortType::StaticContext).len(), 2);
    }
}
