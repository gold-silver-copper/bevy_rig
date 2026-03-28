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

    pub fn editable_text(&self) -> Option<&str> {
        self.value.editable_text()
    }

    pub fn set_text_value(&mut self, value: String) -> bool {
        self.value.set_text_value(value)
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
            NodeValue::MaxTokens(value) => vec![format!("value = {}", value)],
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
            NodeValue::DefaultMaxTurns(value) => vec![format!("value = {}", value)],
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
        let prompt = graph.add_node(
            NodeTemplate::PromptText.instantiate(),
            Vec2::new(260.0, 140.0),
        );
        let model = graph.add_node(NodeTemplate::Model.instantiate(), Vec2::new(260.0, 350.0));
        let name = graph.add_node(NodeTemplate::NameText.instantiate(), Vec2::new(260.0, 20.0));
        let description = graph.add_node(
            NodeTemplate::DescriptionText.instantiate(),
            Vec2::new(260.0, 560.0),
        );
        let preamble = graph.add_node(
            NodeTemplate::PreambleText.instantiate(),
            Vec2::new(20.0, 140.0),
        );
        let context = graph.add_node(
            NodeTemplate::StaticContextText.instantiate(),
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

        graph
    }

    pub fn reset_demo(&mut self) {
        *self = Self::demo();
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
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

    pub fn set_node_text_value(&mut self, node_id: NodeId, value: String) -> bool {
        let changed = self
            .node_mut(node_id)
            .map(|node| node.set_text_value(value))
            .unwrap_or(false);

        if changed {
            self.touch();
        }

        changed
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
                NodeValue::MaxTokens(value) => {
                    let next = (*value as i64 + (delta as i64 * 64)).clamp(32, 8192);
                    *value = next as u64;
                    true
                }
                NodeValue::ToolChoice(value) => {
                    *value = value.shifted(delta);
                    true
                }
                NodeValue::DefaultMaxTurns(value) => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_value_types_connect() {
        let mut graph = GraphDocument::demo();
        let prompt = graph.add_node(NodeTemplate::PromptText.instantiate(), Vec2::ZERO);
        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::ZERO);
        assert!(graph.connect_ports(prompt, PortType::TextValue, agent, PortType::Prompt));
    }

    #[test]
    fn multi_input_keeps_multiple_context_edges() {
        let mut graph = GraphDocument::demo();
        let a = graph.add_node(NodeTemplate::StaticContextText.instantiate(), Vec2::ZERO);
        let b = graph.add_node(NodeTemplate::StaticContextText.instantiate(), Vec2::ZERO);
        let agent = graph.add_node(NodeTemplate::Agent.instantiate(), Vec2::ZERO);

        assert!(graph.connect_ports(a, PortType::TextValue, agent, PortType::StaticContext));
        assert!(graph.connect_ports(b, PortType::TextValue, agent, PortType::StaticContext));
        assert_eq!(graph.input_sources(agent, PortType::StaticContext).len(), 2);
    }
}
