//! Soft node-separation physics.
//!
//! Each graph node is mirrored by an invisible [`avian2d`] rigid body whose
//! collider matches the node's on-canvas rectangle. With gravity disabled and
//! deliberately *soft*, slow contact resolution, overlapping nodes drift gently
//! apart and then settle — heavy linear damping bleeds off any leftover motion
//! so nothing keeps sliding forever.
//!
//! The bodies live in world space (the same coordinate space as
//! [`GraphNode::position`], which is the node's top-left corner). The graph
//! document stays the source of truth: external edits (dragging, resizing,
//! pasting) are detected and pushed into the simulation, and the simulated
//! positions are written back into the document every physics tick *without*
//! bumping its revision, so the existing per-frame view systems move the nodes
//! and re-route the wires while the heavier rebuild stays idle.

use std::collections::HashMap;

use avian2d::dynamics::solver::SolverConfig;
use avian2d::prelude::*;
use bevy::prelude::*;

use crate::graph::{GraphDocument, NodeId};

/// Extra padding baked into each collider (world px). Two settled nodes keep
/// roughly this much clear space between their edges.
const NODE_GAP: f32 = 18.0;
/// Linear velocity damping. High, so a nudged node loses its momentum quickly
/// and never drifts off on its own.
const NODE_LINEAR_DAMPING: f32 = 8.0;
/// Surface friction coefficient — kept high per the "high friction" goal.
const NODE_FRICTION: f32 = 0.9;
/// Maximum speed (world px/s) the solver uses to push overlapping nodes apart.
/// Small on purpose: the separation looks like a gentle nudge instead of a pop.
const MAX_OVERLAP_SOLVE_SPEED: f32 = 200.0;
/// Contact softness. Lower than avian's defaults (`10.0` / `1.5`) so contacts
/// behave like soft springs rather than rigid walls.
const CONTACT_DAMPING_RATIO: f32 = 6.0;
const CONTACT_FREQUENCY_FACTOR: f32 = 1.0;
/// A document position change larger than this (world px) is read as an external
/// edit (drag/resize/paste) and teleported into the simulation.
const EXTERNAL_MOVE_EPSILON: f32 = 0.25;
/// Don't write a simulated position back unless it moved at least this much,
/// so a settled graph doesn't churn change-detection every tick.
const WRITEBACK_EPSILON: f32 = 0.05;

pub struct NodePhysicsPlugin;

impl Plugin for NodePhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            // A node canvas has no "down" — bodies only ever move to resolve overlap.
            .insert_resource(Gravity(Vec2::ZERO))
            .insert_resource(SolverConfig {
                max_overlap_solve_speed: MAX_OVERLAP_SOLVE_SPEED,
                contact_damping_ratio: CONTACT_DAMPING_RATIO,
                contact_frequency_factor: CONTACT_FREQUENCY_FACTOR,
                ..default()
            })
            .init_resource::<NodeBodyRegistry>()
            .add_systems(
                FixedPostUpdate,
                (
                    sync_document_to_bodies.before(PhysicsSystems::Prepare),
                    sync_bodies_to_document.after(PhysicsSystems::Writeback),
                ),
            );
    }
}

/// Marks a physics body and ties it back to its graph node.
#[derive(Component)]
struct NodeBody {
    id: NodeId,
}

struct BodyState {
    entity: Entity,
    /// Node size the current collider was built for; rebuilt when it changes.
    size: Vec2,
    /// Top-left the simulation last produced for this node. Comparing it against
    /// the document position reveals edits made elsewhere (drag/resize/paste).
    last_written: Vec2,
}

#[derive(Resource, Default)]
struct NodeBodyRegistry {
    bodies: HashMap<NodeId, BodyState>,
}

/// Spawn/despawn bodies to match the document, and feed external edits in.
fn sync_document_to_bodies(
    mut commands: Commands,
    document: Res<GraphDocument>,
    mut registry: ResMut<NodeBodyRegistry>,
    mut bodies: Query<(&mut Position, &mut LinearVelocity, &mut Collider), With<NodeBody>>,
) {
    for (id, node) in &document.nodes {
        let center = node.position + node.size * 0.5;
        let collider = node.size + Vec2::splat(NODE_GAP);

        match registry.bodies.get_mut(id) {
            None => {
                let entity = commands
                    .spawn((
                        NodeBody { id: *id },
                        RigidBody::Dynamic,
                        Collider::rectangle(collider.x, collider.y),
                        // Nodes stay axis-aligned; only translation matters.
                        LockedAxes::ROTATION_LOCKED,
                        LinearDamping(NODE_LINEAR_DAMPING),
                        Friction::new(NODE_FRICTION),
                        Position(center),
                        Transform::from_translation(center.extend(0.0)),
                    ))
                    .id();
                registry.bodies.insert(
                    *id,
                    BodyState {
                        entity,
                        size: node.size,
                        last_written: node.position,
                    },
                );
            }
            Some(state) => {
                let Ok((mut position, mut velocity, mut collider_shape)) =
                    bodies.get_mut(state.entity)
                else {
                    continue;
                };

                let moved_externally =
                    node.position.distance(state.last_written) > EXTERNAL_MOVE_EPSILON;
                let resized = node.size.distance(state.size) > f32::EPSILON;

                if moved_externally || resized {
                    // Snap the body to where the edit put it and kill any drift,
                    // so a dragged/resized node shoves its neighbours rather than
                    // fighting the solver.
                    position.0 = center;
                    velocity.0 = Vec2::ZERO;
                    state.last_written = node.position;
                }
                if resized {
                    *collider_shape = Collider::rectangle(collider.x, collider.y);
                    state.size = node.size;
                }
            }
        }
    }

    // Drop bodies whose nodes were deleted.
    registry.bodies.retain(|id, state| {
        let alive = document.nodes.contains_key(id);
        if !alive {
            commands.entity(state.entity).despawn();
        }
        alive
    });
}

/// Write settled positions back into the document (without bumping its revision).
fn sync_bodies_to_document(
    mut document: ResMut<GraphDocument>,
    mut registry: ResMut<NodeBodyRegistry>,
    bodies: Query<(&NodeBody, &Position)>,
) {
    for (body, position) in &bodies {
        let Some(node) = document.node(body.id) else {
            continue;
        };
        let top_left = position.0 - node.size * 0.5;
        let current = node.position;

        if let Some(state) = registry.bodies.get_mut(&body.id) {
            state.last_written = top_left;
        }

        // Only touch the document (and trip change-detection) on real motion.
        if top_left.distance(current) > WRITEBACK_EPSILON
            && let Some(node) = document.node_mut(body.id)
        {
            node.position = top_left;
        }
    }
}
