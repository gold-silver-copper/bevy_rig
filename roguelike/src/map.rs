use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

use bevy::prelude::Resource;

use crate::components::Position;

const A_STAR_BASE_COST: i32 = 10;
const MAX_A_STAR_NODES: usize = 640;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Floor,
    Wall,
}

impl Tile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Floor => "stone flagstones",
            Self::Wall => "carved granite wall",
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Self::Floor => '.',
            Self::Wall => '#',
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Floor => {
                "Broad stone flagstones worn smooth by boots, tankards, and generations of spilled brew."
            }
            Self::Wall => {
                "A carved granite wall blackened by torch smoke and etched with old clan marks."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropKind {
    BarCounter,
    Table,
    Chair,
    Stool,
    Barrel,
    Crate,
    Bottle,
    Mug,
    Candle,
    Shelf,
    Piano,
}

impl PropKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::BarCounter => "ale counter",
            Self::Table => "drinking table",
            Self::Chair => "stone chair",
            Self::Stool => "keg stool",
            Self::Barrel => "ale cask",
            Self::Crate => "brew crate",
            Self::Bottle => "jug",
            Self::Mug => "tankard",
            Self::Candle => "candle",
            Self::Shelf => "keg rack",
            Self::Piano => "anvil organ",
        }
    }

    pub fn glyph(self) -> char {
        match self {
            Self::BarCounter => '=',
            Self::Table => 'T',
            Self::Chair => 'h',
            Self::Stool => 'u',
            Self::Barrel => '0',
            Self::Crate => 'B',
            Self::Bottle => '!',
            Self::Mug => 'u',
            Self::Candle => '\'',
            Self::Shelf => '#',
            Self::Piano => 'P',
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::BarCounter => {
                "A heavy stone-topped ale counter slick with foam, honey mead, and berry wine."
            }
            Self::Table => {
                "A stout dwarven drinking table scarred by mugs, knives, and arm-wrestling contests."
            }
            Self::Chair => "A squat stone chair built for a broad dwarven back.",
            Self::Stool => "A low keg stool shoved up against the ale counter.",
            Self::Barrel => {
                "A cask of whatever the brewers could coax from cave wheat, berries, roots, or honey."
            }
            Self::Crate => {
                "A brew crate full of crocks, herbs, spare cups, and whatever fruit has not fermented yet."
            }
            Self::Bottle => {
                "A stoppered jug of harsh mountain liquor, tart mushroom cider, or plum wine."
            }
            Self::Mug => "A thick tankard beaded with foam and smelling faintly of malt.",
            Self::Candle => "A guttering tallow candle throwing warm amber light.",
            Self::Shelf => {
                "A rack crammed with spare tankards, crocks of mead, and stoneware jugs."
            }
            Self::Piano => {
                "A wheezing anvil organ battered out for marching songs and drunken hall choruses."
            }
        }
    }

    pub fn blocks_movement(self) -> bool {
        !matches!(self, Self::Bottle | Self::Mug | Self::Candle)
    }

    pub fn blocks_sight(self) -> bool {
        matches!(self, Self::BarCounter | Self::Shelf)
    }
}

#[derive(Resource, Clone, Debug)]
pub struct TileMap {
    pub width: i32,
    pub height: i32,
    tiles: Vec<Tile>,
    props: HashMap<(i32, i32), PropKind>,
}

impl TileMap {
    pub fn demo() -> Self {
        let mut map = Self::filled(96, 48, Tile::Wall);
        map.carve_room(3, 3, 90, 42);

        map.paint_vertical_wall(60, 4, 44);
        map.open_vertical_door(60, 12, 2);
        map.open_vertical_door(60, 24, 2);
        map.open_vertical_door(60, 36, 2);

        map.paint_vertical_wall(75, 4, 44);
        map.open_vertical_door(75, 18, 2);
        map.open_vertical_door(75, 34, 2);

        map.paint_horizontal_wall(14, 61, 91);
        map.open_horizontal_door(14, 68, 2);
        map.open_horizontal_door(14, 84, 2);

        map.paint_horizontal_wall(30, 76, 91);
        map.open_horizontal_door(30, 84, 2);

        map.set(46, 44, Tile::Floor);
        map.set(47, 44, Tile::Floor);
        map.set(48, 44, Tile::Floor);

        for x in 12..=28 {
            map.set_prop(x, 10, PropKind::BarCounter);
        }
        for x in [13, 16, 19, 22, 25, 28] {
            map.set_prop(x, 11, PropKind::Stool);
        }
        for x in 13..=27 {
            if x % 2 == 1 {
                map.set_prop(x, 7, PropKind::Shelf);
            }
        }
        for x in [14, 18, 22, 26] {
            map.set_prop(x, 8, PropKind::Bottle);
            map.set_prop(x + 1, 8, PropKind::Mug);
        }

        map.place_table_set(18, 20);
        map.place_table_set(26, 28);
        map.place_table_set(38, 18);
        map.place_table_set(46, 26);
        map.place_table_set(52, 14);
        map.set_prop(50, 8, PropKind::Piano);
        map.set_prop(52, 8, PropKind::Candle);

        for (x, y) in [(64, 8), (66, 8), (70, 8), (72, 8)] {
            map.set_prop(x, y, PropKind::Barrel);
        }
        for (x, y) in [(64, 11), (67, 11), (71, 11)] {
            map.set_prop(x, y, PropKind::Crate);
        }
        map.set_prop(69, 6, PropKind::Shelf);
        map.set_prop(70, 6, PropKind::Bottle);
        map.set_prop(71, 6, PropKind::Bottle);

        map.place_table_set(82, 9);
        map.place_table_set(86, 12);
        map.place_table_set(82, 22);
        map.place_table_set(87, 25);
        map.place_table_set(82, 36);
        map.place_table_set(87, 39);

        for (x, y) in [(65, 21), (69, 21), (72, 21), (65, 26), (70, 27)] {
            map.set_prop(x, y, PropKind::Barrel);
        }
        for (x, y) in [(66, 33), (70, 35), (72, 37)] {
            map.set_prop(x, y, PropKind::Crate);
        }
        for (x, y) in [(78, 6), (88, 6), (78, 32), (88, 32)] {
            map.set_prop(x, y, PropKind::Candle);
        }

        map
    }

    fn filled(width: i32, height: i32, tile: Tile) -> Self {
        Self {
            width,
            height,
            tiles: vec![tile; (width * height) as usize],
            props: HashMap::new(),
        }
    }

    fn carve_room(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for yy in y..(y + h) {
            for xx in x..(x + w) {
                let is_edge = xx == x || yy == y || xx == x + w - 1 || yy == y + h - 1;
                self.set(xx, yy, if is_edge { Tile::Wall } else { Tile::Floor });
            }
        }
    }

    fn paint_vertical_wall(&mut self, x: i32, y0: i32, y1: i32) {
        for y in y0..=y1 {
            self.set(x, y, Tile::Wall);
        }
    }

    fn paint_horizontal_wall(&mut self, y: i32, x0: i32, x1: i32) {
        for x in x0..=x1 {
            self.set(x, y, Tile::Wall);
        }
    }

    fn open_vertical_door(&mut self, x: i32, y: i32, height: i32) {
        for yy in y..(y + height) {
            self.set(x, yy, Tile::Floor);
        }
    }

    fn open_horizontal_door(&mut self, y: i32, x: i32, width: i32) {
        for xx in x..(x + width) {
            self.set(xx, y, Tile::Floor);
        }
    }

    fn place_table_set(&mut self, x: i32, y: i32) {
        self.set_prop(x, y, PropKind::Table);
        self.set_prop(x - 1, y, PropKind::Chair);
        self.set_prop(x + 1, y, PropKind::Chair);
        self.set_prop(x, y - 1, PropKind::Mug);
        self.set_prop(x, y + 1, PropKind::Bottle);
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

    pub fn set_prop(&mut self, x: i32, y: i32, prop: PropKind) {
        if self.in_bounds(x, y) {
            self.props.insert((x, y), prop);
        }
    }

    pub fn prop(&self, x: i32, y: i32) -> Option<PropKind> {
        self.props.get(&(x, y)).copied()
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    pub fn is_walkable(&self, pos: Position) -> bool {
        self.in_bounds(pos.x, pos.y)
            && self.tile(pos.x, pos.y) == Tile::Floor
            && !self
                .prop(pos.x, pos.y)
                .is_some_and(|prop| prop.blocks_movement())
    }

    pub fn blocks_sight(&self, pos: Position) -> bool {
        !self.in_bounds(pos.x, pos.y)
            || self.tile(pos.x, pos.y) == Tile::Wall
            || self
                .prop(pos.x, pos.y)
                .is_some_and(|prop| prop.blocks_sight())
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

    pub fn player_visible_tiles(
        &self,
        origin: Position,
        cursor: Position,
        min_radius: i32,
        max_range: i32,
    ) -> HashSet<Position> {
        let delta = Position::new(cursor.x - origin.x, cursor.y - origin.y);
        let (range, cos_threshold) = compute_player_fov_params(delta, min_radius, max_range);
        let mut visible = self.visible_tiles(origin, range);

        if delta != Position::new(0, 0) {
            let cdx = delta.x as f64;
            let cdy = delta.y as f64;
            let cursor_len = (cdx * cdx + cdy * cdy).sqrt();

            visible.retain(|&tile| {
                let diff = Position::new(tile.x - origin.x, tile.y - origin.y);
                if diff == Position::new(0, 0) {
                    return true;
                }
                // Preserve the immediate "keyhole" around the player when aiming.
                if diff.x.abs() <= 1 && diff.y.abs() <= 1 {
                    return true;
                }

                let dx = diff.x as f64;
                let dy = diff.y as f64;
                let len = (dx * dx + dy * dy).sqrt();
                let dot = (dx * cdx + dy * cdy) / (len * cursor_len);
                dot >= cos_threshold
            });
        }

        visible.retain(|&tile| has_clear_los(self, origin, tile));
        visible
    }

    pub fn has_line_of_sight(&self, origin: Position, target: Position, radius: i32) -> bool {
        origin.chebyshev_distance(target) <= radius && has_clear_los(self, origin, target)
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
        let mut explored = 0usize;
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

            explored += 1;
            if explored >= MAX_A_STAR_NODES {
                break;
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

                let tentative_g = current_cost + A_STAR_BASE_COST;
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

    pub fn first_path_step(
        &self,
        start: Position,
        goal: Position,
        blocked: &HashSet<(i32, i32)>,
    ) -> Option<Position> {
        self.find_path(start, goal, blocked)
            .and_then(|path| path.first().copied())
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

fn compute_player_fov_params(delta: Position, min_radius: i32, max_range: i32) -> (i32, f64) {
    if delta == Position::new(0, 0) {
        return (min_radius.max(0), -1.0);
    }

    let dist = (((delta.x * delta.x + delta.y * delta.y) as f64).sqrt()).max(1.0);
    let growth = ((max_range - min_radius).max(0) as f64) / 20.0;
    let range = (min_radius as f64 + dist * growth).min(max_range as f64);
    let cone_t = (dist / 20.0).min(1.0);
    let cos_threshold = -1.0 + cone_t * 1.985;

    (range.round() as i32, cos_threshold)
}

fn has_clear_los(map: &TileMap, origin: Position, target: Position) -> bool {
    if target == origin {
        return true;
    }

    for tile in origin.bresenham_line(target).into_iter().skip(1) {
        if tile != target && map.blocks_sight(tile) {
            return false;
        }
    }

    true
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

    #[test]
    fn player_fov_hides_tiles_behind_cursor_direction() {
        let map = TileMap::filled(9, 9, Tile::Floor);
        let origin = Position::new(4, 4);
        let cursor = Position::new(8, 4);
        let behind = Position::new(1, 4);

        let visible = map.player_visible_tiles(origin, cursor, 3, 8);

        assert!(visible.contains(&origin));
        assert!(!visible.contains(&behind));
    }

    #[test]
    fn first_path_step_matches_walkable_route() {
        let mut map = TileMap::filled(7, 7, Tile::Floor);
        for y in 1..6 {
            if y != 3 {
                map.set(3, y, Tile::Wall);
            }
        }

        let first = map
            .first_path_step(Position::new(1, 3), Position::new(5, 3), &HashSet::new())
            .expect("path should yield a first step");

        assert_eq!(first, Position::new(2, 3));
    }
}
