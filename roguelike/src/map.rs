use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

use bevy::prelude::Resource;

use crate::components::Position;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Floor,
    Wall,
}

#[derive(Resource, Clone, Debug)]
pub struct TileMap {
    pub width: i32,
    pub height: i32,
    tiles: Vec<Tile>,
}

impl TileMap {
    pub fn demo() -> Self {
        let mut map = Self::filled(54, 22, Tile::Floor);
        map.paint_border();
        map.paint_room(3, 2, 15, 7, 9, 8);
        map.paint_room(20, 2, 15, 7, 27, 8);
        map.paint_room(38, 2, 13, 7, 44, 8);
        map.paint_room(8, 13, 18, 6, 17, 13);
        map.paint_room(31, 13, 18, 6, 40, 13);
        map
    }

    fn filled(width: i32, height: i32, tile: Tile) -> Self {
        Self {
            width,
            height,
            tiles: vec![tile; (width * height) as usize],
        }
    }

    fn paint_border(&mut self) {
        for x in 0..self.width {
            self.set(x, 0, Tile::Wall);
            self.set(x, self.height - 1, Tile::Wall);
        }
        for y in 0..self.height {
            self.set(0, y, Tile::Wall);
            self.set(self.width - 1, y, Tile::Wall);
        }
    }

    fn paint_room(&mut self, x: i32, y: i32, w: i32, h: i32, door_x: i32, door_y: i32) {
        for yy in y..(y + h) {
            for xx in x..(x + w) {
                let is_edge = xx == x || yy == y || xx == x + w - 1 || yy == y + h - 1;
                if is_edge {
                    self.set(xx, yy, Tile::Wall);
                }
            }
        }
        self.set(door_x, door_y, Tile::Floor);
    }

    pub fn tile(&self, x: i32, y: i32) -> Tile {
        if !self.in_bounds(x, y) {
            return Tile::Wall;
        }
        self.tiles[(y * self.width + x) as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, tile: Tile) {
        if self.in_bounds(x, y) {
            self.tiles[(y * self.width + x) as usize] = tile;
        }
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    pub fn is_walkable(&self, pos: Position) -> bool {
        self.in_bounds(pos.x, pos.y) && self.tile(pos.x, pos.y) == Tile::Floor
    }

    pub fn blocks_sight(&self, pos: Position) -> bool {
        !self.in_bounds(pos.x, pos.y) || self.tile(pos.x, pos.y) == Tile::Wall
    }

    pub fn visible_tiles(&self, origin: Position, radius: i32) -> HashSet<Position> {
        let mut visible = HashSet::new();
        if radius < 0 || !self.in_bounds(origin.x, origin.y) {
            return visible;
        }

        visible.insert(origin);
        for octant in 0..8 {
            shadowcast_octant(
                self,
                &mut visible,
                origin,
                radius,
                octant,
                1,
                Slope { y: 1, x: 1 },
                Slope { y: 0, x: 1 },
            );
        }

        visible
    }

    pub fn has_line_of_sight(&self, origin: Position, target: Position, radius: i32) -> bool {
        self.visible_tiles(origin, radius).contains(&target)
    }

    pub fn find_path(
        &self,
        start: Position,
        goal: Position,
        blocked: &HashSet<(i32, i32)>,
    ) -> Option<Vec<Position>> {
        if start == goal {
            return Some(Vec::new());
        }

        if !self.is_walkable(goal) || blocked.contains(&(goal.x, goal.y)) {
            return None;
        }

        let mut frontier =
            BinaryHeap::from([PathNode::new(start, 0, manhattan_distance(start, goal))]);
        let mut came_from = HashMap::new();
        let mut g_score = HashMap::from([((start.x, start.y), 0_i32)]);
        const DIRECTIONS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

        while let Some(node) = frontier.pop() {
            let current = node.position;
            let Some(current_cost) = g_score.get(&(current.x, current.y)).copied() else {
                continue;
            };
            if node.g_cost > current_cost {
                continue;
            }

            if current == goal {
                return reconstruct_path(start, goal, &came_from);
            }

            for (dx, dy) in DIRECTIONS {
                let next = current.offset(dx, dy);
                let key = (next.x, next.y);
                if !self.is_walkable(next) {
                    continue;
                }
                if next != goal && blocked.contains(&key) {
                    continue;
                }

                let tentative_g = current_cost + 1;
                let known_g = g_score.get(&key).copied().unwrap_or(i32::MAX);
                if tentative_g < known_g {
                    came_from.insert(key, current);
                    g_score.insert(key, tentative_g);
                    frontier.push(PathNode::new(
                        next,
                        tentative_g,
                        manhattan_distance(next, goal),
                    ));
                }
            }
        }

        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PathNode {
    position: Position,
    g_cost: i32,
    h_cost: i32,
}

impl PathNode {
    fn new(position: Position, g_cost: i32, h_cost: i32) -> Self {
        Self {
            position,
            g_cost,
            h_cost,
        }
    }

    fn f_cost(self) -> i32 {
        self.g_cost + self.h_cost
    }
}

impl Ord for PathNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_cost()
            .cmp(&self.f_cost())
            .then_with(|| other.h_cost.cmp(&self.h_cost))
            .then_with(|| other.position.y.cmp(&self.position.y))
            .then_with(|| other.position.x.cmp(&self.position.x))
    }
}

impl PartialOrd for PathNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn manhattan_distance(a: Position, b: Position) -> i32 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

fn reconstruct_path(
    start: Position,
    goal: Position,
    came_from: &HashMap<(i32, i32), Position>,
) -> Option<Vec<Position>> {
    let mut path = vec![goal];
    let mut cursor = goal;

    while cursor != start {
        cursor = *came_from.get(&(cursor.x, cursor.y))?;
        if cursor != start {
            path.push(cursor);
        }
    }

    path.reverse();
    Some(path)
}

#[derive(Clone, Copy)]
struct Slope {
    y: i32,
    x: i32,
}

impl Slope {
    fn ge(self, other: Self) -> bool {
        self.y * other.x >= other.y * self.x
    }
}

fn transform(origin: Position, octant: u8, row: i32, col: i32) -> Position {
    match octant {
        0 => origin.offset(col, row),
        1 => origin.offset(row, col),
        2 => origin.offset(row, -col),
        3 => origin.offset(col, -row),
        4 => origin.offset(-col, -row),
        5 => origin.offset(-row, -col),
        6 => origin.offset(-row, col),
        7 => origin.offset(-col, row),
        _ => unreachable!(),
    }
}

fn shadowcast_octant(
    map: &TileMap,
    visible: &mut HashSet<Position>,
    origin: Position,
    range: i32,
    octant: u8,
    row: i32,
    mut start: Slope,
    end: Slope,
) {
    if row > range || !start.ge(end) {
        return;
    }

    let range_sq = range * range;
    let mut prev_opaque = false;
    let mut saved_start = start;
    let min_col = round_down(row, end);
    let max_col = round_up(row, start);

    for col in (min_col..=max_col).rev() {
        let dist_sq = row * row + col * col;
        if dist_sq > range_sq {
            continue;
        }

        let world = transform(origin, octant, row, col);
        if !map.in_bounds(world.x, world.y) {
            continue;
        }

        visible.insert(world);

        let cur_opaque = map.blocks_sight(world);
        if cur_opaque {
            if !prev_opaque {
                saved_start = start;
            }
            start = Slope {
                y: 2 * col - 1,
                x: 2 * row,
            };
        } else if prev_opaque {
            shadowcast_octant(
                map,
                visible,
                origin,
                range,
                octant,
                row + 1,
                saved_start,
                Slope {
                    y: 2 * col + 1,
                    x: 2 * row,
                },
            );
        }

        prev_opaque = cur_opaque;
    }

    if !prev_opaque {
        shadowcast_octant(map, visible, origin, range, octant, row + 1, start, end);
    }
}

fn round_up(row: i32, slope: Slope) -> i32 {
    (row * slope.y + slope.x - 1).div_euclid(slope.x)
}

fn round_down(row: i32, slope: Slope) -> i32 {
    ((row * 2 * slope.y + slope.x) / (2 * slope.x)).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_of_sight_reaches_open_floor() {
        let map = TileMap::filled(8, 8, Tile::Floor);
        let origin = Position::new(1, 1);
        let target = Position::new(5, 1);

        assert!(map.has_line_of_sight(origin, target, 6));
    }

    #[test]
    fn wall_blocks_tiles_behind_it() {
        let mut map = TileMap::filled(8, 8, Tile::Floor);
        for y in 1..7 {
            map.set(3, y, Tile::Wall);
        }

        let origin = Position::new(1, 4);
        let wall = Position::new(3, 4);
        let beyond_wall = Position::new(5, 4);

        let visible = map.visible_tiles(origin, 8);

        assert!(visible.contains(&wall));
        assert!(!visible.contains(&beyond_wall));
        assert!(!map.has_line_of_sight(origin, beyond_wall, 8));
    }

    #[test]
    fn find_path_uses_walkable_route_around_obstacles() {
        let mut map = TileMap::filled(7, 7, Tile::Floor);
        for y in 1..6 {
            if y != 3 {
                map.set(3, y, Tile::Wall);
            }
        }

        let path = map
            .find_path(Position::new(1, 3), Position::new(5, 3), &HashSet::new())
            .expect("path should exist");

        assert_eq!(path.last().copied(), Some(Position::new(5, 3)));
        assert!(path.iter().all(|pos| map.is_walkable(*pos)));
    }
}
