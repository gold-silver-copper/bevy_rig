use std::collections::BTreeMap;

use bevy::prelude::*;

use crate::catalog::{
    NodeId, NodeSeed, NodeTemplate, NodeType, NodeValue, PortAddress, PortDirection, PortSpec,
    PortType, node_inputs, node_outputs, preview_line, preview_lines,
};

pub type EdgeId = u64;

#[derive(Clone, Debug)]
pub struct GraphNode {
    pub id: NodeId,
    pub position: Vec2,
    pub size: Vec2,
    pub node_type: NodeType,
    pub title: String,
    pub value: NodeValue,
}

impl GraphNode {
    pub fn inputs(&self) -> &'static [PortSpec] {
        node_inputs(self.node_type)
    }

    pub fn outputs(&self) -> &'static [PortSpec] {
        node_outputs(self.node_type)
    }

    pub fn editable_value_text(&self) -> Option<String> {
        self.value.inline_value_string()
    }

    pub fn set_inline_value(&mut self, value: &str) -> bool {
        self.value.set_inline_value(value)
    }

    pub fn summary_lines(&self) -> Vec<String> {
        match &self.value {
            NodeValue::None => vec![
                "Ephemeral Rig agent compiled from incoming nodes.".into(),
                "Requires: model + prompt + text output.".into(),
            ],
            NodeValue::Text(text) => preview_lines(text, "text"),
            NodeValue::TextOutput { text, status } => {
                let mut lines = vec![format!("status = {status}")];
                lines.extend(preview_lines(text, "output"));
                lines
            }
            NodeValue::Model {
                provider_label,
                model_name,
            } => vec![
                format!("provider = {provider_label}"),
                format!(
                    "model = {}",
                    model_name.as_deref().unwrap_or("(discovering locally)")
                ),
            ],
            NodeValue::Temperature(value) => vec![format!("value = {:.1}", value)],
            NodeValue::U64(value) => vec![format!("value = {}", value)],
            NodeValue::AdditionalParams(value) => preview_lines(value, "json"),
            NodeValue::ToolServerHandle(value) => {
                vec!["stored only in this MVP".into(), preview_line(value)]
            }
            NodeValue::DynamicContext(value) => {
                vec!["stored only in this MVP".into(), preview_line(value)]
            }
            NodeValue::ToolChoice(value) => vec![
                format!("value = {}", value.label()),
                "Ollama ignores this today.".into(),
            ],
            NodeValue::Hook(value) => vec!["stored only in this MVP".into(), preview_line(value)],
            NodeValue::OutputSchema(value) => preview_lines(value, "schema"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub id: EdgeId,
    pub from: PortAddress,
    pub to: PortAddress,
}

#[derive(Resource, Clone, Debug)]
pub struct GraphDocument {
    pub nodes: BTreeMap<NodeId, GraphNode>,
    pub edges: BTreeMap<EdgeId, GraphEdge>,
    pub revision: u64,
    next_node_id: NodeId,
    next_edge_id: EdgeId,
}

impl Default for GraphDocument {
    fn default() -> Self {
        Self::demo()
    }
}

impl GraphDocument {
    pub fn demo() -> Self {
        let mut graph = Self {
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            revision: 1,
            next_node_id: 1,
            next_edge_id: 1,
        };

        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::new(760.0, 220.0));
        let output = graph.add_node(
            NodeTemplate::TextOutput.instantiate(),
            Vec2::new(1210.0, 260.0),
        );
        let prompt = graph.add_node(NodeTemplate::String.instantiate(), Vec2::new(260.0, 140.0));
        let model = graph.add_node(NodeTemplate::Model.instantiate(), Vec2::new(260.0, 350.0));
        let name = graph.add_node(NodeTemplate::String.instantiate(), Vec2::new(260.0, 20.0));
        let description =
            graph.add_node(NodeTemplate::String.instantiate(), Vec2::new(260.0, 560.0));
        let preamble = graph.add_node(NodeTemplate::String.instantiate(), Vec2::new(20.0, 140.0));
        let context = graph.add_node(NodeTemplate::String.instantiate(), Vec2::new(20.0, 380.0));
        let temperature = graph.add_node(
            NodeTemplate::Temperature.instantiate(),
            Vec2::new(520.0, 20.0),
        );
        let max_tokens = graph.add_node(NodeTemplate::U64.instantiate(), Vec2::new(520.0, 560.0));
        let additional_params = graph.add_node(
            NodeTemplate::AdditionalParams.instantiate(),
            Vec2::new(520.0, 700.0),
        );

        graph.configure_text_node(name, "Name", "Hall Greeter");
        graph.configure_text_node(
            description,
            "Description",
            "Greets guests and recommends a fitting brew.",
        );
        graph.configure_text_node(
            preamble,
            "Preamble",
            "You are a merry dwarf host in a loud mountain alehall.",
        );
        graph.configure_text_node(
            context,
            "Static Context",
            "The alehall serves frothy ale, berry mead, root cider, and stubborn cave wine.",
        );
        graph.configure_text_node(
            prompt,
            "Prompt",
            "Write a warm dwarven greeting for the alehall and ask what brew the guest wants.",
        );
        graph.configure_u64_node(max_tokens, "Max Tokens", 192);

        graph.connect_ports(name, PortType::TextValue, agent, PortType::AgentName);
        graph.connect_ports(prompt, PortType::TextValue, agent, PortType::Prompt);
        graph.connect_ports(model, PortType::Model, agent, PortType::Model);
        graph.connect_ports(
            description,
            PortType::TextValue,
            agent,
            PortType::AgentDescription,
        );
        graph.connect_ports(preamble, PortType::TextValue, agent, PortType::Preamble);
        graph.connect_ports(context, PortType::TextValue, agent, PortType::StaticContext);
        graph.connect_ports(
            temperature,
            PortType::Temperature,
            agent,
            PortType::Temperature,
        );
        graph.connect_ports(max_tokens, PortType::U64Value, agent, PortType::MaxTokens);
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

        graph
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.nodes.values()
    }

    pub fn iter_edges(&self) -> impl Iterator<Item = &GraphEdge> {
        self.edges.values()
    }

    pub fn first_node_id_by_type(&self, node_type: NodeType) -> Option<NodeId> {
        self.nodes
            .values()
            .find(|node| node.node_type == node_type)
            .map(|node| node.id)
    }

    pub fn add_node(&mut self, seed: NodeSeed, position: Vec2) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            id,
            GraphNode {
                id,
                position,
                size: default_node_size(seed.node_type),
                node_type: seed.node_type,
                title: seed.title,
                value: seed.value,
            },
        );
        self.touch();
        id
    }

    pub fn duplicate_node(&mut self, node_id: NodeId, offset: Vec2) -> Option<NodeId> {
        let node = self.node(node_id)?.clone();
        let new_id = self.add_node(
            NodeSeed {
                node_type: node.node_type,
                title: node.title,
                value: node.value,
            },
            node.position + offset,
        );
        if let Some(duplicate) = self.node_mut(new_id) {
            duplicate.size = node.size;
        }
        Some(new_id)
    }

    pub fn remove_node(&mut self, node_id: NodeId) -> bool {
        if self.nodes.remove(&node_id).is_none() {
            return false;
        }

        self.edges
            .retain(|_, edge| edge.from.node != node_id && edge.to.node != node_id);
        self.touch();
        true
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn move_node(&mut self, node_id: NodeId, delta: Vec2) {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.position += delta;
        }
    }

    pub fn resize_node(&mut self, node_id: NodeId, corner: ResizeCorner, delta: Vec2) {
        let Some(node) = self.nodes.get_mut(&node_id) else {
            return;
        };

        let min_size = default_node_size(node.node_type);
        let mut left = node.position.x;
        let mut top = node.position.y;
        let mut right = node.position.x + node.size.x;
        let mut bottom = node.position.y + node.size.y;

        match corner {
            ResizeCorner::NorthWest => {
                left += delta.x;
                top += delta.y;
            }
            ResizeCorner::NorthEast => {
                right += delta.x;
                top += delta.y;
            }
            ResizeCorner::SouthWest => {
                left += delta.x;
                bottom += delta.y;
            }
            ResizeCorner::SouthEast => {
                right += delta.x;
                bottom += delta.y;
            }
        }

        if right - left < min_size.x {
            match corner {
                ResizeCorner::NorthWest | ResizeCorner::SouthWest => left = right - min_size.x,
                ResizeCorner::NorthEast | ResizeCorner::SouthEast => right = left + min_size.x,
            }
        }

        if bottom - top < min_size.y {
            match corner {
                ResizeCorner::NorthWest | ResizeCorner::NorthEast => top = bottom - min_size.y,
                ResizeCorner::SouthWest | ResizeCorner::SouthEast => bottom = top + min_size.y,
            }
        }

        node.position = Vec2::new(left, top);
        node.size = Vec2::new(right - left, bottom - top);
    }

    pub fn node(&self, node_id: NodeId) -> Option<&GraphNode> {
        self.nodes.get(&node_id)
    }

    pub fn node_mut(&mut self, node_id: NodeId) -> Option<&mut GraphNode> {
        self.nodes.get_mut(&node_id)
    }

    pub fn port_spec(&self, address: PortAddress) -> Option<PortSpec> {
        let node = self.node(address.node)?;
        match address.direction {
            PortDirection::Input => node.inputs().get(address.index).copied(),
            PortDirection::Output => node.outputs().get(address.index).copied(),
        }
    }

    pub fn input_port_index(&self, node_id: NodeId, ty: PortType) -> Option<usize> {
        self.node(node_id)?
            .inputs()
            .iter()
            .position(|spec| spec.ty == ty)
    }

    pub fn output_port_index(&self, node_id: NodeId, ty: PortType) -> Option<usize> {
        self.node(node_id)?
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
            self.edges.retain(|_, edge| edge.to != to);
        }

        let duplicate = self
            .edges
            .values()
            .any(|edge| edge.from == from && edge.to == to);
        if duplicate {
            return true;
        }

        let edge_id = self.next_edge_id;
        self.next_edge_id += 1;
        self.edges.insert(
            edge_id,
            GraphEdge {
                id: edge_id,
                from,
                to,
            },
        );
        self.touch();
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

        from_spec.value_type == to_spec.value_type
    }

    pub fn input_sources(&self, node_id: NodeId, ty: PortType) -> Vec<NodeId> {
        let Some(index) = self.input_port_index(node_id, ty) else {
            return Vec::new();
        };

        self.edges
            .values()
            .filter(|edge| edge.to.node == node_id && edge.to.index == index)
            .map(|edge| edge.from.node)
            .collect()
    }

    pub fn output_targets(&self, node_id: NodeId, ty: PortType) -> Vec<NodeId> {
        let Some(index) = self.output_port_index(node_id, ty) else {
            return Vec::new();
        };

        self.edges
            .values()
            .filter(|edge| edge.from.node == node_id && edge.from.index == index)
            .map(|edge| edge.to.node)
            .collect()
    }

    #[allow(dead_code)]
    pub fn set_node_title(&mut self, node_id: NodeId, value: String) -> bool {
        self.set_node_title_with_touch(node_id, value, true)
    }

    pub fn set_node_title_live(&mut self, node_id: NodeId, value: String) -> bool {
        self.set_node_title_with_touch(node_id, value, false)
    }

    fn set_node_title_with_touch(&mut self, node_id: NodeId, value: String, touch: bool) -> bool {
        let changed = self
            .node_mut(node_id)
            .map(|node| {
                if node.title == value {
                    false
                } else {
                    node.title = value;
                    true
                }
            })
            .unwrap_or(false);

        if changed && touch {
            self.touch();
        }

        changed
    }

    #[allow(dead_code)]
    pub fn set_node_inline_value(&mut self, node_id: NodeId, value: &str) -> bool {
        self.set_node_inline_value_with_touch(node_id, value, true)
    }

    pub fn set_node_inline_value_live(&mut self, node_id: NodeId, value: &str) -> bool {
        self.set_node_inline_value_with_touch(node_id, value, false)
    }

    fn set_node_inline_value_with_touch(
        &mut self,
        node_id: NodeId,
        value: &str,
        touch: bool,
    ) -> bool {
        let changed = self
            .node_mut(node_id)
            .map(|node| {
                let changed = node.set_inline_value(value);
                if changed {
                    grow_node_to_fit_contents(node);
                }
                changed
            })
            .unwrap_or(false);

        if changed && touch {
            self.touch();
        }

        changed
    }

    pub fn configure_text_node(&mut self, node_id: NodeId, title: &str, value: &str) -> bool {
        let Some(node) = self.node_mut(node_id) else {
            return false;
        };
        if !matches!(node.value, NodeValue::Text(_)) {
            return false;
        }

        node.title = title.to_string();
        node.value = NodeValue::Text(value.to_string());
        grow_node_to_fit_contents(node);
        self.touch();
        true
    }

    pub fn configure_u64_node(&mut self, node_id: NodeId, title: &str, value: u64) -> bool {
        let Some(node) = self.node_mut(node_id) else {
            return false;
        };
        if !matches!(node.value, NodeValue::U64(_)) {
            return false;
        }

        node.title = title.to_string();
        node.value = NodeValue::U64(value);
        self.touch();
        true
    }

    pub fn cycle_setting(
        &mut self,
        node_id: NodeId,
        delta: i32,
        available_models: &[String],
    ) -> bool {
        let changed = self
            .node_mut(node_id)
            .map(|node| match &mut node.value {
                NodeValue::Model { model_name, .. } => {
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
                NodeValue::Temperature(value) => {
                    *value = (*value + (delta as f64 * 0.1)).clamp(0.0, 2.0);
                    true
                }
                NodeValue::U64(value) => {
                    let next = (*value as i64 + delta as i64).clamp(0, 8192);
                    *value = next as u64;
                    true
                }
                NodeValue::ToolChoice(value) => {
                    *value = value.shifted(delta);
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
        for node in self.nodes.values_mut() {
            if let NodeValue::Model { model_name, .. } = &mut node.value {
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
        if let Some(GraphNode {
            value:
                NodeValue::TextOutput {
                    text: current_text,
                    status: current_status,
                },
            ..
        }) = self.node_mut(node_id)
        {
            *current_text = text;
            *current_status = status;
            self.touch();
        }
    }

    pub fn clear_output(&mut self, node_id: NodeId) -> bool {
        if let Some(GraphNode {
            value: NodeValue::TextOutput { text, status },
            ..
        }) = self.node_mut(node_id)
        {
            *text = "Run the selected agent to populate this sink.".into();
            *status = "idle".into();
            self.touch();
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeCorner {
    NorthWest,
    NorthEast,
    SouthWest,
    SouthEast,
}

pub fn default_node_size(node_type: NodeType) -> Vec2 {
    let (width, body_height) = default_node_body_dimensions(node_type);
    let height = node_chrome_height(node_type) + body_height;
    Vec2::new(width, height)
}

fn grow_node_to_fit_contents(node: &mut GraphNode) {
    let min_size = default_node_size(node.node_type);
    let Some(body_height) = dynamic_body_height(node) else {
        node.size = node.size.max(min_size);
        return;
    };

    let target = Vec2::new(
        min_size.x,
        (node_chrome_height(node.node_type) + body_height).max(min_size.y),
    );
    node.size = node.size.max(target);
}

fn dynamic_body_height(node: &GraphNode) -> Option<f32> {
    let text = match &node.value {
        NodeValue::Text(text)
        | NodeValue::AdditionalParams(text)
        | NodeValue::ToolServerHandle(text)
        | NodeValue::DynamicContext(text)
        | NodeValue::Hook(text)
        | NodeValue::OutputSchema(text) => text.as_str(),
        _ => return None,
    };

    let (_, min_body_height) = default_node_body_dimensions(node.node_type);
    let content_width = (node.size.x.max(default_node_size(node.node_type).x) - 40.0).max(140.0);
    let chars_per_line = (content_width / 8.2).floor().max(12.0) as usize;
    let wrapped_lines = text
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(chars_per_line))
        .sum::<usize>()
        .max(1);
    let content_height =
        20.0 + wrapped_lines as f32 * 18.0 + (wrapped_lines.saturating_sub(1) as f32 * 4.0);
    Some(content_height.max(min_body_height))
}

fn default_node_body_dimensions(node_type: NodeType) -> (f32, f32) {
    match node_type {
        NodeType::Agent => (390.0, 0.0),
        NodeType::Text => (360.0, 92.0),
        NodeType::AdditionalParams | NodeType::OutputSchema => (340.0, 118.0),
        NodeType::TextOutput => (340.0, 120.0),
        NodeType::Model => (360.0, 72.0),
        NodeType::Temperature | NodeType::U64 | NodeType::ToolChoice => (300.0, 62.0),
        NodeType::ToolServerHandle | NodeType::DynamicContext | NodeType::Hook => (320.0, 92.0),
    }
}

fn node_chrome_height(node_type: NodeType) -> f32 {
    let row_count = match node_type {
        NodeType::Agent => 21.0,
        _ => node_inputs(node_type)
            .len()
            .max(node_outputs(node_type).len())
            .max(1) as f32,
    };
    34.0 + 12.0 + (row_count * (28.0 + 2.0)) + 8.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_value_types_connect() {
        let mut graph = GraphDocument::demo();
        let prompt = graph.add_node(NodeTemplate::String.instantiate(), Vec2::ZERO);
        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::ZERO);
        assert!(graph.connect_ports(prompt, PortType::TextValue, agent, PortType::Prompt));
    }

    #[test]
    fn multi_input_keeps_multiple_context_edges() {
        let mut graph = GraphDocument::demo();
        let a = graph.add_node(NodeTemplate::String.instantiate(), Vec2::ZERO);
        let b = graph.add_node(NodeTemplate::String.instantiate(), Vec2::ZERO);
        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::ZERO);

        assert!(graph.connect_ports(a, PortType::TextValue, agent, PortType::StaticContext));
        assert!(graph.connect_ports(b, PortType::TextValue, agent, PortType::StaticContext));
        assert_eq!(graph.input_sources(agent, PortType::StaticContext).len(), 2);
    }
}
