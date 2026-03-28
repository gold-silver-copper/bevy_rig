use bevy::prelude::*;

use crate::catalog::{NodeId, PortAddress, PortType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraggingWire {
    pub from: PortAddress,
    pub ty: PortType,
}

#[derive(Resource, Clone, Debug)]
pub struct EditorSession {
    pub pan: Vec2,
    pub zoom: f32,
    pub selected_node: Option<NodeId>,
    pub dragging_wire: Option<DraggingWire>,
    pub revision: u64,
}

impl Default for EditorSession {
    fn default() -> Self {
        Self {
            pan: Vec2::new(360.0, 120.0),
            zoom: 1.0,
            selected_node: None,
            dragging_wire: None,
            revision: 1,
        }
    }
}

impl EditorSession {
    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn reset_view(&mut self) {
        *self = Self::default();
    }

    pub fn select_node(&mut self, node_id: Option<NodeId>) {
        if self.selected_node != node_id {
            self.selected_node = node_id;
            self.touch();
        }
    }
}
