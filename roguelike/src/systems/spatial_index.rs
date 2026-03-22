use bevy::prelude::*;

use crate::components::Position;
use crate::resources::SpatialIndex;

/// Rebuilds the spatial index every tick so that other systems can perform
/// O(1) entity-at-position lookups without scanning all entities.
///
/// Runs unconditionally at the start of `Update`.
pub fn spatial_index_system(mut index: ResMut<SpatialIndex>, query: Query<(Entity, &Position)>) {
    index.map.clear();
    for (entity, pos) in &query {
        index.map.entry(pos.as_grid_vec()).or_default().push(entity);
    }
}
