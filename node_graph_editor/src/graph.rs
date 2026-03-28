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
    Model,
    Clip,
    Vae,
    Conditioning,
    Latent,
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortSpec {
    pub name: &'static str,
    pub ty: PortType,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    LoadCheckpoint {
        checkpoint: String,
    },
    ClipTextEncode {
        title: &'static str,
        text: String,
    },
    EmptyLatentImage {
        width: u32,
        height: u32,
        batch_size: u32,
    },
    KSampler {
        seed: u64,
        steps: u32,
        cfg: f32,
    },
    VaeDecode,
    SaveImage {
        filename_prefix: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeTemplate {
    LoadCheckpoint,
    ClipTextEncodePositive,
    ClipTextEncodeNegative,
    EmptyLatentImage,
    KSampler,
    VaeDecode,
    SaveImage,
}

impl NodeTemplate {
    pub const ALL: [NodeTemplate; 7] = [
        NodeTemplate::LoadCheckpoint,
        NodeTemplate::ClipTextEncodePositive,
        NodeTemplate::ClipTextEncodeNegative,
        NodeTemplate::EmptyLatentImage,
        NodeTemplate::KSampler,
        NodeTemplate::VaeDecode,
        NodeTemplate::SaveImage,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::LoadCheckpoint => "Load Checkpoint",
            Self::ClipTextEncodePositive => "CLIP Text Encode (Prompt)",
            Self::ClipTextEncodeNegative => "CLIP Text Encode (Negative)",
            Self::EmptyLatentImage => "Empty Latent Image",
            Self::KSampler => "KSampler",
            Self::VaeDecode => "VAE Decode",
            Self::SaveImage => "Save Image",
        }
    }

    pub fn instantiate(self) -> NodeKind {
        match self {
            Self::LoadCheckpoint => NodeKind::LoadCheckpoint {
                checkpoint: "sdxl_base_1.0.safetensors".into(),
            },
            Self::ClipTextEncodePositive => NodeKind::ClipTextEncode {
                title: "CLIP Text Encode (Prompt)",
                text: "beautiful subterranean dwarven hall, brass lanterns, carved stone archways, frothy ale".into(),
            },
            Self::ClipTextEncodeNegative => NodeKind::ClipTextEncode {
                title: "CLIP Text Encode (Negative)",
                text: "blurry, washed out, watermark".into(),
            },
            Self::EmptyLatentImage => NodeKind::EmptyLatentImage {
                width: 1024,
                height: 1024,
                batch_size: 1,
            },
            Self::KSampler => NodeKind::KSampler {
                seed: 1_566_802_087_002_860,
                steps: 20,
                cfg: 8.0,
            },
            Self::VaeDecode => NodeKind::VaeDecode,
            Self::SaveImage => NodeKind::SaveImage {
                filename_prefix: "BevyGraph".into(),
            },
        }
    }
}

impl NodeKind {
    pub const fn title(&self) -> &'static str {
        match self {
            Self::LoadCheckpoint { .. } => "Load Checkpoint",
            Self::ClipTextEncode { title, .. } => title,
            Self::EmptyLatentImage { .. } => "Empty Latent Image",
            Self::KSampler { .. } => "KSampler",
            Self::VaeDecode => "VAE Decode",
            Self::SaveImage { .. } => "Save Image",
        }
    }

    pub fn inputs(&self) -> Vec<PortSpec> {
        match self {
            Self::LoadCheckpoint { .. } => Vec::new(),
            Self::ClipTextEncode { .. } => vec![PortSpec {
                name: "clip",
                ty: PortType::Clip,
            }],
            Self::EmptyLatentImage { .. } => Vec::new(),
            Self::KSampler { .. } => vec![
                PortSpec {
                    name: "model",
                    ty: PortType::Model,
                },
                PortSpec {
                    name: "positive",
                    ty: PortType::Conditioning,
                },
                PortSpec {
                    name: "negative",
                    ty: PortType::Conditioning,
                },
                PortSpec {
                    name: "latent_image",
                    ty: PortType::Latent,
                },
            ],
            Self::VaeDecode => vec![
                PortSpec {
                    name: "samples",
                    ty: PortType::Latent,
                },
                PortSpec {
                    name: "vae",
                    ty: PortType::Vae,
                },
            ],
            Self::SaveImage { .. } => vec![PortSpec {
                name: "images",
                ty: PortType::Image,
            }],
        }
    }

    pub fn outputs(&self) -> Vec<PortSpec> {
        match self {
            Self::LoadCheckpoint { .. } => vec![
                PortSpec {
                    name: "model",
                    ty: PortType::Model,
                },
                PortSpec {
                    name: "clip",
                    ty: PortType::Clip,
                },
                PortSpec {
                    name: "vae",
                    ty: PortType::Vae,
                },
            ],
            Self::ClipTextEncode { .. } => vec![PortSpec {
                name: "conditioning",
                ty: PortType::Conditioning,
            }],
            Self::EmptyLatentImage { .. } => vec![PortSpec {
                name: "latent",
                ty: PortType::Latent,
            }],
            Self::KSampler { .. } => vec![PortSpec {
                name: "latent",
                ty: PortType::Latent,
            }],
            Self::VaeDecode => vec![PortSpec {
                name: "image",
                ty: PortType::Image,
            }],
            Self::SaveImage { .. } => Vec::new(),
        }
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
    pub selected_node: Option<NodeId>,
    pub dragging_wire: Option<DraggingWire>,
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
            pan: Vec2::new(140.0, 90.0),
            selected_node: None,
            dragging_wire: None,
            next_node_id: 1,
        };

        let load = graph.add_node(
            NodeTemplate::LoadCheckpoint.instantiate(),
            Vec2::new(0.0, 220.0),
        );
        let positive = graph.add_node(
            NodeTemplate::ClipTextEncodePositive.instantiate(),
            Vec2::new(360.0, 40.0),
        );
        let negative = graph.add_node(
            NodeTemplate::ClipTextEncodeNegative.instantiate(),
            Vec2::new(360.0, 310.0),
        );
        let latent = graph.add_node(
            NodeTemplate::EmptyLatentImage.instantiate(),
            Vec2::new(420.0, 560.0),
        );
        let sampler = graph.add_node(
            NodeTemplate::KSampler.instantiate(),
            Vec2::new(780.0, 180.0),
        );
        let decode = graph.add_node(
            NodeTemplate::VaeDecode.instantiate(),
            Vec2::new(1190.0, 180.0),
        );
        let save = graph.add_node(
            NodeTemplate::SaveImage.instantiate(),
            Vec2::new(1520.0, 180.0),
        );

        graph.connect(
            PortAddress {
                node: load,
                direction: PortDirection::Output,
                index: 0,
            },
            PortAddress {
                node: sampler,
                direction: PortDirection::Input,
                index: 0,
            },
        );
        graph.connect(
            PortAddress {
                node: load,
                direction: PortDirection::Output,
                index: 1,
            },
            PortAddress {
                node: positive,
                direction: PortDirection::Input,
                index: 0,
            },
        );
        graph.connect(
            PortAddress {
                node: load,
                direction: PortDirection::Output,
                index: 1,
            },
            PortAddress {
                node: negative,
                direction: PortDirection::Input,
                index: 0,
            },
        );
        graph.connect(
            PortAddress {
                node: positive,
                direction: PortDirection::Output,
                index: 0,
            },
            PortAddress {
                node: sampler,
                direction: PortDirection::Input,
                index: 1,
            },
        );
        graph.connect(
            PortAddress {
                node: negative,
                direction: PortDirection::Output,
                index: 0,
            },
            PortAddress {
                node: sampler,
                direction: PortDirection::Input,
                index: 2,
            },
        );
        graph.connect(
            PortAddress {
                node: latent,
                direction: PortDirection::Output,
                index: 0,
            },
            PortAddress {
                node: sampler,
                direction: PortDirection::Input,
                index: 3,
            },
        );
        graph.connect(
            PortAddress {
                node: sampler,
                direction: PortDirection::Output,
                index: 0,
            },
            PortAddress {
                node: decode,
                direction: PortDirection::Input,
                index: 0,
            },
        );
        graph.connect(
            PortAddress {
                node: load,
                direction: PortDirection::Output,
                index: 2,
            },
            PortAddress {
                node: decode,
                direction: PortDirection::Input,
                index: 1,
            },
        );
        graph.connect(
            PortAddress {
                node: decode,
                direction: PortDirection::Output,
                index: 0,
            },
            PortAddress {
                node: save,
                direction: PortDirection::Input,
                index: 0,
            },
        );

        graph
    }

    pub fn reset_demo(&mut self) {
        *self = Self::demo();
    }

    pub fn add_node(&mut self, kind: NodeKind, position: Vec2) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.push(GraphNode { id, position, kind });
        id
    }

    pub fn node(&self, node_id: NodeId) -> Option<&GraphNode> {
        self.nodes.iter().find(|node| node.id == node_id)
    }

    pub fn node_mut(&mut self, node_id: NodeId) -> Option<&mut GraphNode> {
        self.nodes.iter_mut().find(|node| node.id == node_id)
    }

    pub fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|node| node.id).collect()
    }

    pub fn port_spec(&self, address: PortAddress) -> Option<PortSpec> {
        let node = self.node(address.node)?;
        match address.direction {
            PortDirection::Input => node.kind.inputs().get(address.index).copied(),
            PortDirection::Output => node.kind.outputs().get(address.index).copied(),
        }
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

        self.edges.retain(|edge| edge.to != to);
        let edge = GraphEdge { from, to };
        if !self.edges.contains(&edge) {
            self.edges.push(edge);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_port_types_connect() {
        let mut graph = GraphEditorState::demo();
        let a = graph.add_node(NodeTemplate::EmptyLatentImage.instantiate(), Vec2::ZERO);
        let b = graph.add_node(NodeTemplate::KSampler.instantiate(), Vec2::ZERO);
        assert!(graph.connect(
            PortAddress {
                node: a,
                direction: PortDirection::Output,
                index: 0
            },
            PortAddress {
                node: b,
                direction: PortDirection::Input,
                index: 3
            }
        ));
    }

    #[test]
    fn mismatched_port_types_reject() {
        let mut graph = GraphEditorState::demo();
        let a = graph.add_node(NodeTemplate::EmptyLatentImage.instantiate(), Vec2::ZERO);
        let b = graph.add_node(NodeTemplate::SaveImage.instantiate(), Vec2::ZERO);
        assert!(!graph.connect(
            PortAddress {
                node: a,
                direction: PortDirection::Output,
                index: 0
            },
            PortAddress {
                node: b,
                direction: PortDirection::Input,
                index: 0
            }
        ));
    }
}
