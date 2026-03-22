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

impl Tile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Floor => "floorboards",
            Self::Wall => "timber wall",
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
            Self::Floor => "Scuffed tavern floorboards stained by years of boots and spilled ale.",
            Self::Wall => "A thick timber wall lined with plaster and old lantern soot.",
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
            Self::BarCounter => "bar counter",
            Self::Table => "table",
            Self::Chair => "chair",
            Self::Stool => "stool",
            Self::Barrel => "barrel",
            Self::Crate => "crate",
            Self::Bottle => "bottle",
            Self::Mug => "mug",
            Self::Candle => "candle",
            Self::Shelf => "shelf",
            Self::Piano => "piano",
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
            Self::BarCounter => "A polished bar counter worn smooth by elbows and spilled ale.",
            Self::Table => "A sturdy tavern table ready for cards, gossip, or another round.",
            Self::Chair => "A creaking wooden chair.",
            Self::Stool => "A low stool parked near the bar.",
            Self::Barrel => "A cask that smells faintly of beer and oak.",
            Self::Crate => "A supply crate full of kitchen odds and ends.",
            Self::Bottle => "A bottle of house spirits.",
            Self::Mug => "A mug waiting for a refill.",
            Self::Candle => "A candle throwing warm amber light.",
            Self::Shelf => "A shelf lined with spare cups and bottles.",
            Self::Piano => "An old upright piano with a few sticky keys.",
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
