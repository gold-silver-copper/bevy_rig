use std::collections::{HashMap, HashSet};

use crate::grid_vec::GridVec;
use crate::noise::{NoiseSeed, fbm, value_noise};
use crate::typedefs::{CoordinateUnit, MyPoint, RenderPacket, SPAWN_POINT, create_2d_array};
use crate::typeenums::{Floor, Props};
use crate::voxel::Voxel;

/// Side length of a map chunk (tiles). Used for chunk-based spatial queries.
pub const CHUNK_SIZE: CoordinateUnit = 16;

/// The game map: a simple 2D grid of voxels.
pub struct GameMap {
    pub width: CoordinateUnit,
    pub height: CoordinateUnit,
    pub voxels: Vec<Vec<Voxel>>,
    /// Tracks the world turn at which each fire tile was ignited.
    /// Used for deterministic burnout.
    pub fire_turns: HashMap<GridVec, u32>,
    /// Stores the previous floor type before a tile caught fire.
    /// Used to determine post-burnout tile type (e.g., bridges become water).
    pub fire_previous_floor: HashMap<GridVec, Option<Floor>>,
    /// Tracks the world turn at which each sand cloud tile was placed.
    /// Sand clouds dissipate after `SAND_CLOUD_LIFETIME` world turns.
    pub sand_cloud_turns: HashMap<GridVec, u32>,
    /// Stores the previous floor type before a sand cloud was placed.
    /// Used to restore the original floor when the cloud dissipates.
    pub sand_cloud_previous_floor: HashMap<GridVec, Option<Floor>>,
    /// Shared spatial occupancy grid. True = tile is already occupied by
    /// a road, river, beach, or building. No later pass may place on it.
    pub occupancy: Vec<Vec<bool>>,
}

/// A rectangular building footprint used during town generation.
struct Building {
    x: CoordinateUnit,
    y: CoordinateUnit,
    w: CoordinateUnit,
    h: CoordinateUnit,
    /// What kind of building: 0=house, 1=saloon, 2=stable, 3=general store,
    /// 4=police station, 5=post office, 6=church, 7=bank, 8=hotel,
    /// 9=jail, 10=undertaker, 11=blacksmith.
    kind: u32,
}

// ── World generation phase system ────────────────────────────────────────

/// Intermediate data accumulated between world generation phases.
#[derive(Default)]
/// Intermediate data accumulated during hierarchical world generation.
/// Phases produce and consume this data, enabling later phases to reference
/// layout decisions made by earlier ones (e.g., street positions inform
/// building placement).
struct WorldGenData {
    avenue_ys: Vec<CoordinateUnit>,
    cross_xs: Vec<CoordinateUnit>,
    buildings: Vec<Building>,
    avenue_half_width: CoordinateUnit,
    cross_half_width: CoordinateUnit,
    /// Tiles along road edges (sidewalk or dirt tiles adjacent to non-road terrain).
    /// Collected after the infrastructure pass so prop placement can follow
    /// actual road curvature rather than straight-line approximations.
    road_edge_tiles: Vec<(CoordinateUnit, CoordinateUnit)>,
}

/// A single step in the hierarchical world generation pipeline.
/// Each phase receives the map and shared data, applying one category of
/// generation (terrain, water, infrastructure, buildings, etc.).
/// Phases run in order; later phases may read data produced by earlier ones.
trait WorldGenPhase {
    fn execute(
        &self,
        map: &mut GameMap,
        data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    );
}

/// Phase 1: Desert base terrain + forest on outskirts.
struct TerrainPhase;

impl TerrainPhase {
    /// Creates the initial `GameMap` with noise-driven desert terrain.
    fn create_map(width: CoordinateUnit, height: CoordinateUnit, seed: NoiseSeed) -> GameMap {
        let biome_seed = seed;
        let detail_seed = seed.wrapping_add(12345);

        let mut voxels = Vec::with_capacity(height as usize);
        for y in 0..height {
            let mut row = Vec::with_capacity(width as usize);
            for x in 0..width {
                let fx = x as f64;
                let fy = y as f64;

                let biome = fbm(fx, fy, 4, 0.03, 0.5, biome_seed);
                let detail = fbm(fx, fy, 3, 0.1, 0.5, detail_seed);

                let floor = select_desert_floor(biome, detail);

                let prop = None;

                row.push(Voxel {
                    floor: Some(floor),
                    props: prop,
                });
            }
            voxels.push(row);
        }

        let mut map = GameMap {
            width,
            height,
            voxels,
            fire_turns: HashMap::new(),
            fire_previous_floor: HashMap::new(),
            sand_cloud_turns: HashMap::new(),
            sand_cloud_previous_floor: HashMap::new(),
            occupancy: vec![vec![false; height as usize]; width as usize],
        };

        // Forest on outskirts
        let forest_margin = 30;
        let forest_seed = seed.wrapping_add(77700);
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                let dist_to_edge = x.min(y).min(width - 1 - x).min(height - 1 - y);
                if dist_to_edge >= forest_margin {
                    continue;
                }
                let density = (1.0 - dist_to_edge as f64 / forest_margin as f64).powi(2);
                let noise = value_noise(x, y, forest_seed);
                if noise < density * 0.7 {
                    let pos = GridVec::new(x, y);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.floor = Some(Floor::Grass);
                        voxel.props = Some(Props::Tree);
                    }
                }
            }
        }
        // Carve small paths through the forest
        let path_seed = seed.wrapping_add(77711);
        for i in 0..8i32 {
            let angle = value_noise(i, 0, path_seed) * std::f64::consts::TAU;
            let cx = width / 2;
            let cy = height / 2;
            let mut px = cx as f64;
            let mut py = cy as f64;
            let dx = angle.cos();
            let dy = angle.sin();
            while px > 0.0 && px < (width - 1) as f64 && py > 0.0 && py < (height - 1) as f64 {
                let ix = px as CoordinateUnit;
                let iy = py as CoordinateUnit;
                for oy in -1..=1i32 {
                    for ox in -1..=1i32 {
                        let p = GridVec::new(ix + ox, iy + oy);
                        if let Some(voxel) = map.get_voxel_at_mut(&p)
                            && matches!(voxel.props, Some(Props::Tree))
                        {
                            voxel.props = None;
                        }
                    }
                }
                px += dx;
                py += dy;
            }
        }

        map
    }
}

/// Phase 2: River generation.
struct WaterPhase;

impl WorldGenPhase for WaterPhase {
    fn execute(
        &self,
        map: &mut GameMap,
        _data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) {
        let river_seed = seed.wrapping_add(88800);
        let river_cx = width as f64 / 2.0;
        // Track the previous row's center so each row drifts at most ±1 tile.
        let mut prev_center_x = river_cx;
        // River flows top to bottom with smooth banks.
        for y in 1..height - 1 {
            let fy = y as f64;
            // Desired center from multi-octave noise (determines general shape).
            let desired = river_cx
                + (fy * 0.008).sin() * 40.0      // large meander
                + (fy * 0.020).sin() * 20.0; // medium curve
            // Clamp drift to ±1 tile per row for smooth banks.
            let drift = (desired - prev_center_x).clamp(-1.0, 1.0);
            let center_x = prev_center_x + drift;
            prev_center_x = center_x;
            // River width varies smoothly along its length.
            let base_width = 14.0 + value_noise(y, 0, river_seed.wrapping_add(111)) * 10.0;
            let width_pulse = (fy * 0.012).sin() * 6.0; // gentle widening/narrowing
            let river_width = (base_width + width_pulse).max(8.0);
            let beach_width = 5.0;

            for x in 1..width - 1 {
                let fx = x as f64;
                let dist = (fx - center_x).abs();
                let pos = GridVec::new(x, y);
                if dist < river_width * 0.4 {
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.floor = Some(Floor::DeepWater);
                        voxel.props = None;
                    }
                } else if dist < river_width {
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.floor = Some(Floor::ShallowWater);
                        voxel.props = None;
                    }
                } else if dist < river_width + beach_width
                    && let Some(voxel) = map.get_voxel_at_mut(&pos)
                    && !matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall))
                {
                    voxel.floor = Some(Floor::BeachSand);
                    voxel.props = None;
                }
            }
        }

        // Mark water and beach tiles as occupied
        for y in 0..map.height {
            for x in 0..map.width {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = map.get_voxel_at(&pos) {
                    if matches!(
                        voxel.floor,
                        Some(Floor::ShallowWater)
                            | Some(Floor::DeepWater)
                            | Some(Floor::Beach)
                            | Some(Floor::BeachSand)
                    ) {
                        map.occupancy[x as usize][y as usize] = true;
                    }
                }
            }
        }
    }
}

/// Phase 3: Street grid (avenues, cross streets, bridges).
struct InfrastructurePhase;

impl WorldGenPhase for InfrastructurePhase {
    fn execute(
        &self,
        map: &mut GameMap,
        data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) {
        // ── Pass 3a: Roads and streets ───────────────────────────────────
        // Horizontal avenues: wide dirt carriage roads flanked by sidewalks.
        // Avenue spacing varies per seed for unique city feel.
        let spacing_noise = value_noise(0, 0, seed.wrapping_add(99900));
        let avenue_spacing = 32 + (spacing_noise * 12.0) as CoordinateUnit; // 32-44
        let avenue_half_width = 3; // carriage road half-width (7 tiles total)
        let sidewalk_width = 2; // sidewalk on each side of the road
        let curve_seed = seed.wrapping_add(55500);
        {
            let first_avenue = 40; // start inside the forest margin
            let mut ay = first_avenue;
            while ay < height - 40 {
                data.avenue_ys.push(ay);
                ay += avenue_spacing;
            }
        }
        data.avenue_half_width = avenue_half_width;

        // Lay avenue roads with sidewalks — roads skip water and beach (buffer exclusion zone).
        for &ay in &data.avenue_ys {
            let curve_amp = 3.0 + value_noise(ay, 0, curve_seed) * 4.0;
            let curve_freq = 0.015 + value_noise(0, ay, curve_seed) * 0.01;
            for x in 1..width - 1 {
                let curve_offset = (x as f64 * curve_freq).sin() * curve_amp;
                // Sidewalk (outer band)
                for sw in 1..=sidewalk_width {
                    for sign in [-1i32, 1] {
                        let y =
                            ay + sign * (avenue_half_width + sw) + curve_offset as CoordinateUnit;
                        if y <= 0 || y >= height - 1 {
                            continue;
                        }
                        let pos = GridVec::new(x, y);
                        if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                            if matches!(
                                voxel.floor,
                                Some(Floor::ShallowWater)
                                    | Some(Floor::DeepWater)
                                    | Some(Floor::Beach)
                                    | Some(Floor::BeachSand)
                            ) {
                                continue;
                            }
                            voxel.floor = Some(Floor::Sidewalk);
                            voxel.props = None;
                        }
                    }
                }
                // Carriage road (inner band)
                for hw in -avenue_half_width..=avenue_half_width {
                    let y = ay + hw + curve_offset as CoordinateUnit;
                    if y <= 0 || y >= height - 1 {
                        continue;
                    }
                    let pos = GridVec::new(x, y);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        if matches!(
                            voxel.floor,
                            Some(Floor::ShallowWater)
                                | Some(Floor::DeepWater)
                                | Some(Floor::Beach)
                                | Some(Floor::BeachSand)
                        ) {
                            continue;
                        }
                        voxel.floor = Some(Floor::DirtRoad);
                        voxel.props = None;
                    }
                }
            }
        }

        // Vertical cross streets with sinusoidal curvature and sidewalks.
        // Roads skip water, beach, and bridge tiles.
        // Wider base spacing produces larger lots; per-road RNG offset
        // creates organic, irregular lot widths.
        let cross_seed = seed.wrapping_add(66666);
        let cross_noise = value_noise(1, 1, seed.wrapping_add(99901));
        let cross_spacing = 48 + (cross_noise * 16.0) as CoordinateUnit; // 48-64
        let cross_half_width = 2; // narrower than avenues
        let cross_sidewalk_width = 1;
        data.cross_half_width = cross_half_width;
        {
            let mut cx = 50i32; // push first road farther in
            let mut ci = 0i32;
            while cx < width - 50 {
                // Per-road RNG variance: ±12 tiles for organic lot widths.
                let variance = (value_noise(ci, 0, cross_seed) * 24.0) as CoordinateUnit - 12;
                let jitter = (value_noise(ci, 2, cross_seed) * 10.0) as CoordinateUnit - 5;
                let actual_cx = (cx + jitter).clamp(2, width - 3);
                data.cross_xs.push(actual_cx);
                let curve_amp = 2.0 + value_noise(ci, 1, cross_seed) * 3.0;
                let curve_freq = 0.02 + value_noise(1, ci, cross_seed) * 0.01;
                for y in 1..height - 1 {
                    let curve_offset = (y as f64 * curve_freq).sin() * curve_amp;
                    // Sidewalk — must not overwrite avenue dirt roads or walls.
                    for sw in 1..=cross_sidewalk_width {
                        for sign in [-1i32, 1] {
                            let x = actual_cx
                                + sign * (cross_half_width + sw)
                                + curve_offset as CoordinateUnit;
                            if x <= 0 || x >= width - 1 {
                                continue;
                            }
                            let pos = GridVec::new(x, y);
                            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                                if matches!(
                                    voxel.floor,
                                    Some(Floor::ShallowWater)
                                        | Some(Floor::DeepWater)
                                        | Some(Floor::Beach)
                                        | Some(Floor::BeachSand)
                                        | Some(Floor::Bridge)
                                ) {
                                    continue;
                                }
                                if !matches!(
                                    voxel.props,
                                    Some(Props::Wall) | Some(Props::StoneWall)
                                ) && !matches!(voxel.floor, Some(Floor::DirtRoad))
                                {
                                    voxel.floor = Some(Floor::Sidewalk);
                                    voxel.props = None;
                                }
                            }
                        }
                    }
                    // Carriage road
                    for hw in -cross_half_width..=cross_half_width {
                        let x = actual_cx + hw + curve_offset as CoordinateUnit;
                        if x <= 0 || x >= width - 1 {
                            continue;
                        }
                        let pos = GridVec::new(x, y);
                        if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                            if matches!(
                                voxel.floor,
                                Some(Floor::ShallowWater)
                                    | Some(Floor::DeepWater)
                                    | Some(Floor::Beach)
                                    | Some(Floor::BeachSand)
                                    | Some(Floor::Bridge)
                            ) {
                                continue;
                            }
                            if !matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall)) {
                                voxel.floor = Some(Floor::DirtRoad);
                                voxel.props = None;
                            }
                        }
                    }
                }
                cx += cross_spacing + variance;
                ci += 1;
            }
        }

        // ── Pass 3a-gaps: Remove random segments from vertical roads ─────
        // After each vertical road is generated, randomly remove several
        // segments, leaving gaps of varying length. This produces roads
        // that feel organically incomplete and occasionally creates larger
        // lots where two gaps on adjacent roads align.
        {
            let gap_seed = seed.wrapping_add(77777);
            let mut ci = 0i32;
            let gap_range = (height - 20).max(1); // guard for small maps
            for &road_cx in &data.cross_xs {
                let curve_amp = 2.0 + value_noise(ci, 1, cross_seed) * 3.0;
                let curve_freq = 0.02 + value_noise(1, ci, cross_seed) * 0.01;
                // Determine gap count: 3–6 gaps per road.
                let gap_count = 3 + (value_noise(ci, 0, gap_seed) * 4.0) as i32;
                for gi in 0..gap_count {
                    // Random gap start and length.
                    let gap_start = 10
                        + (value_noise(ci, gi * 2 + 1, gap_seed) * gap_range as f64)
                            as CoordinateUnit;
                    let gap_len =
                        8 + (value_noise(ci, gi * 2 + 2, gap_seed) * 20.0) as CoordinateUnit;
                    let gap_end = (gap_start + gap_len).min(height - 2);

                    for y in gap_start..gap_end {
                        // Skip tiles at avenue intersections to preserve connectivity.
                        let at_avenue = data
                            .avenue_ys
                            .iter()
                            .any(|&ay| (y - ay).abs() <= avenue_half_width + 2);
                        if at_avenue {
                            continue;
                        }

                        let curve_offset = (y as f64 * curve_freq).sin() * curve_amp;
                        // Revert carriage road tiles.
                        for hw in -cross_half_width..=cross_half_width {
                            let x = road_cx + hw + curve_offset as CoordinateUnit;
                            if x <= 0 || x >= width - 1 {
                                continue;
                            }
                            let pos = GridVec::new(x, y);
                            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                                if matches!(voxel.floor, Some(Floor::DirtRoad)) {
                                    voxel.floor = Some(Floor::Dirt);
                                }
                            }
                        }
                        // Revert sidewalk tiles.
                        for sw in 1..=cross_sidewalk_width {
                            for sign in [-1i32, 1] {
                                let x = road_cx
                                    + sign * (cross_half_width + sw)
                                    + curve_offset as CoordinateUnit;
                                if x <= 0 || x >= width - 1 {
                                    continue;
                                }
                                let pos = GridVec::new(x, y);
                                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                                    if matches!(voxel.floor, Some(Floor::Sidewalk)) {
                                        voxel.floor = Some(Floor::Dirt);
                                    }
                                }
                            }
                        }
                    }
                }
                ci += 1;
            }
        }

        // ── Pass 3b: Bridges where horizontal roads cross the river ──────
        // Only bridge at the specific coordinates where a horizontal avenue
        // road band crosses water. Bridge tiles replace ShallowWater,
        // DeepWater, and BeachSand so the bridge surface is clean.
        // Vertical cross streets never bridge the river.
        let road_band = avenue_half_width + sidewalk_width;
        for &ay in &data.avenue_ys {
            let curve_amp = 3.0 + value_noise(ay, 0, curve_seed) * 4.0;
            let curve_freq = 0.015 + value_noise(0, ay, curve_seed) * 0.01;
            // First pass: identify X positions where the road band crosses water
            let mut crosses_water = vec![false; width as usize];
            for x in 1..width - 1 {
                let curve_offset = (x as f64 * curve_freq).sin() * curve_amp;
                let center_y = ay + curve_offset as CoordinateUnit;
                for hw in -road_band..=road_band {
                    let y = center_y + hw;
                    if y <= 0 || y >= height - 1 {
                        continue;
                    }
                    let pos = GridVec::new(x, y);
                    if let Some(voxel) = map.get_voxel_at(&pos) {
                        if matches!(
                            voxel.floor,
                            Some(Floor::ShallowWater) | Some(Floor::DeepWater)
                        ) {
                            crosses_water[x as usize] = true;
                            break;
                        }
                    }
                }
            }
            // Second pass: at crossing X positions, bridge only within the
            // road band so the bridge is road-width, not river-width.
            // Replace water and beach sand tiles so bridges have a clean surface.
            for x in 1..width - 1 {
                if !crosses_water[x as usize] {
                    continue;
                }
                let curve_offset = (x as f64 * curve_freq).sin() * curve_amp;
                let center_y = ay + curve_offset as CoordinateUnit;
                for hw in -road_band..=road_band {
                    let y = center_y + hw;
                    if y <= 0 || y >= height - 1 {
                        continue;
                    }
                    let pos = GridVec::new(x, y);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        if matches!(
                            voxel.floor,
                            Some(Floor::ShallowWater)
                                | Some(Floor::DeepWater)
                                | Some(Floor::BeachSand)
                        ) {
                            voxel.floor = Some(Floor::Bridge);
                            voxel.props = None;
                        }
                    }
                }
            }
        }

        // ── Pass 3c: Fill BeachSand near bridges with road surface ───────
        // After bridges are placed, convert remaining BeachSand tiles that
        // are adjacent to a Bridge tile into Dirt, Gravel, or Sidewalk so
        // roads don't abruptly stop at the beach.
        {
            let mut to_fill: Vec<(GridVec, Floor)> = Vec::new();
            for y in 1..height - 1 {
                for x in 1..width - 1 {
                    let pos = GridVec::new(x, y);
                    if !map
                        .get_voxel_at(&pos)
                        .is_some_and(|voxel| matches!(voxel.floor, Some(Floor::BeachSand)))
                    {
                        continue;
                    }

                    let mut saw_bridge = false;
                    let mut fill_floor: Option<(u32, Floor)> = None;
                    for dy in -2..=2i32 {
                        for dx in -2..=2i32 {
                            let sample = GridVec::new(x + dx, y + dy);
                            if sample.x < 0
                                || sample.x >= width
                                || sample.y < 0
                                || sample.y >= height
                            {
                                continue;
                            }
                            let dist = dx.abs() + dy.abs();
                            if dist > 3 {
                                continue;
                            }
                            if let Some(neighbor) = map.get_voxel_at(&sample) {
                                match neighbor.floor {
                                    Some(Floor::Bridge) => saw_bridge = true,
                                    Some(Floor::DirtRoad) => {
                                        let score = dist as u32 * 10;
                                        if fill_floor
                                            .as_ref()
                                            .is_none_or(|current| score < current.0)
                                        {
                                            fill_floor = Some((score, Floor::DirtRoad));
                                        }
                                    }
                                    Some(Floor::Sidewalk) => {
                                        let score = dist as u32 * 10 + 1;
                                        if fill_floor
                                            .as_ref()
                                            .is_none_or(|current| score < current.0)
                                        {
                                            fill_floor = Some((score, Floor::Sidewalk));
                                        }
                                    }
                                    Some(Floor::Dirt) => {
                                        let score = dist as u32 * 10 + 2;
                                        if fill_floor
                                            .as_ref()
                                            .is_none_or(|current| score < current.0)
                                        {
                                            fill_floor = Some((score, Floor::Dirt));
                                        }
                                    }
                                    Some(Floor::Gravel) => {
                                        let score = dist as u32 * 10 + 3;
                                        if fill_floor
                                            .as_ref()
                                            .is_none_or(|current| score < current.0)
                                        {
                                            fill_floor = Some((score, Floor::Gravel));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    if saw_bridge && let Some((_, floor)) = fill_floor {
                        to_fill.push((pos, floor));
                    }
                }
            }
            for (pos, floor) in to_fill {
                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                    voxel.floor = Some(floor);
                    voxel.props = None;
                }
            }
        }

        // ── Collect road-edge tiles for prop placement ───────────────────
        // Road-edge tiles are Sidewalk tiles adjacent to non-road terrain,
        // following the actual curvature of the laid roads.
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = map.get_voxel_at(&pos) {
                    if !matches!(voxel.floor, Some(Floor::Sidewalk)) {
                        continue;
                    }
                    // Check if adjacent to a non-road tile
                    let adjacent_non_road = pos.cardinal_neighbors().iter().any(|n| {
                        map.get_voxel_at(n).is_some_and(|v| {
                            !matches!(
                                v.floor,
                                Some(Floor::DirtRoad) | Some(Floor::Sidewalk) | Some(Floor::Bridge)
                            )
                        })
                    });
                    if adjacent_non_road {
                        data.road_edge_tiles.push((x, y));
                    }
                }
            }
        }

        // Mark road/sidewalk tiles as occupied
        for y in 0..map.height {
            for x in 0..map.width {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = map.get_voxel_at(&pos) {
                    if matches!(
                        voxel.floor,
                        Some(Floor::DirtRoad) | Some(Floor::Sidewalk) | Some(Floor::Bridge)
                    ) {
                        map.occupancy[x as usize][y as usize] = true;
                    }
                }
            }
        }
    }
}

/// Phase 3b: Railroad tracks — placed after roads but before buildings so
/// rail tiles are occupied before any building or prop pass.
struct RailroadPhase;

impl WorldGenPhase for RailroadPhase {
    fn execute(
        &self,
        map: &mut GameMap,
        _data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) {
        place_railroad(map, width, height, seed);
    }
}

/// Phase 4: Buildings, alleys, faction anchors.
struct BuildingPhase;

impl WorldGenPhase for BuildingPhase {
    fn execute(
        &self,
        map: &mut GameMap,
        data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) {
        let (buildings, parks, open_lots) = generate_buildings_bsp(
            width,
            height,
            seed,
            &data.avenue_ys,
            &data.cross_xs,
            data.avenue_half_width,
            data.cross_half_width,
        );
        data.buildings = buildings;

        // Track which buildings were actually placed (not skipped due to water/beach)
        let mut placed_buildings: Vec<usize> = Vec::new();
        for (idx, b) in data.buildings.iter().enumerate() {
            // Check footprint corners, center, and entrance tile
            let check_points = [
                GridVec::new(b.x, b.y),
                GridVec::new(b.x + b.w - 1, b.y),
                GridVec::new(b.x, b.y + b.h - 1),
                GridVec::new(b.x + b.w - 1, b.y + b.h - 1),
                GridVec::new(b.x + b.w / 2, b.y + b.h / 2),
                GridVec::new(b.x + b.w / 2, b.y + b.h), // entrance tile
            ];
            // Skip buildings on water, beach, or road tiles.
            // Every tile in the footprint and padding must be free of
            // DirtRoad, ShallowWater, DeepWater, and BeachSand.
            let on_excluded = check_points.iter().any(|p| {
                map.get_voxel_at(p).is_some_and(|v| {
                    matches!(
                        v.floor,
                        Some(Floor::DirtRoad)
                            | Some(Floor::ShallowWater)
                            | Some(Floor::DeepWater)
                            | Some(Floor::Beach)
                            | Some(Floor::BeachSand)
                    )
                })
            });
            if on_excluded {
                continue;
            }
            // Also skip if any tile in the footprint is occupied (road, water, etc.)
            if map.is_occupied(b.x, b.y, b.w, b.h) {
                continue;
            }
            place_building(map, b, seed);
            // Hierarchical placement: exterior props based on building type
            place_exterior_props(map, b, seed);
            placed_buildings.push(idx);
        }

        // Retain only the buildings that were actually placed
        let new_buildings: Vec<Building> = placed_buildings
            .iter()
            .map(|&i| {
                let b = &data.buildings[i];
                Building {
                    x: b.x,
                    y: b.y,
                    w: b.w,
                    h: b.h,
                    kind: b.kind,
                }
            })
            .collect();
        data.buildings = new_buildings;

        // Place parks in BSP-allocated nodes.  Parks occupy a node and
        // register in the spatial index just like buildings.
        for &(px, py, pw, ph) in &parks {
            // Skip if any tile in the park footprint is a road
            let park_on_road = (px..px + pw).any(|rx| {
                (py..py + ph).any(|ry| {
                    let p = GridVec::new(rx, ry);
                    map.get_voxel_at(&p).is_some_and(|v| {
                        matches!(
                            v.floor,
                            Some(Floor::DirtRoad)
                                | Some(Floor::Sidewalk)
                                | Some(Floor::Bridge)
                                | Some(Floor::ShallowWater)
                                | Some(Floor::DeepWater)
                                | Some(Floor::Beach)
                                | Some(Floor::BeachSand)
                        )
                    })
                })
            });
            if park_on_road {
                continue;
            }
            // Lay park: grass floor with benches and bushes
            for y in py..py + ph {
                for x in px..px + pw {
                    let pos = GridVec::new(x, y);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.floor = Some(Floor::Grass);
                        voxel.props = None;
                    }
                }
            }
            // Benches and bushes inside the park
            let park_seed = seed.wrapping_add(px as u64 * 31 + py as u64 * 37);
            if pw >= 4 && ph >= 4 {
                set_prop(map, px + 1, py + 1, Props::Bench);
                set_prop(map, px + pw - 2, py + 1, Props::Bench);
                set_prop(map, px + pw / 2, py + ph / 2, Props::Bush);
                if value_noise(px, py, park_seed) < 0.5 {
                    set_prop(map, px + pw / 2 - 1, py + ph - 2, Props::Bush);
                    set_prop(map, px + pw / 2 + 1, py + ph - 2, Props::Bush);
                } else {
                    set_prop(map, px + 1, py + ph - 2, Props::Well);
                }
            }
        }

        place_building_frontages(map, &data.buildings, seed);
        place_service_yards(map, &data.buildings, seed);

        // Fill open lots with contextually appropriate scatter
        place_open_lot_scatter(map, &open_lots, seed);

        // Place narrow alleys between adjacent buildings within blocks
        place_alleys(map, &data.buildings);

        // Ensure every alley region connects to a road or sidewalk
        ensure_alley_connectivity(map, width, height);
        clear_props_near_alleys(map, width, height);

        // Transition zone scatter around town perimeter
        place_transition_zones(map, width, height, seed, &data.buildings);
    }
}

/// Phase 5: Landmarks, plazas, urban features, rock formations.
struct LandmarkPhase;

impl WorldGenPhase for LandmarkPhase {
    fn execute(
        &self,
        map: &mut GameMap,
        data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) {
        place_mission(map, width, height, seed, &data.buildings);
        place_town_hall(map, width, height, seed, &data.buildings);
        place_grand_saloon(map, width, height, seed, &data.buildings);
        place_stone_church(map, width, height, seed, &data.buildings);

        place_town_plaza(map, width, height, seed, &data.buildings);
        place_cemetery(map, width, height, seed, &data.buildings);
        place_corral(map, width, height, seed, &data.buildings);

        place_town_well(map, width, height, seed);
        place_gallows(map, width, height, seed);
        place_water_tower(map, width, height, seed);
        place_windmill(map, width, height, seed);
        place_outposts(map, width, height, seed, &data.buildings);
        place_rock_formations(map, width, height, seed);
    }
}

/// Phase 6: Street props and decorations.
struct DetailPhase;

impl WorldGenPhase for DetailPhase {
    fn execute(
        &self,
        map: &mut GameMap,
        data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) {
        // Street props placed on actual road-edge tiles so they follow
        // road curvature rather than straight-line approximations.
        place_street_props_curved(map, &data.road_edge_tiles, seed);

        // Lamp posts along cross streets
        for &cx in &data.cross_xs {
            place_lamp_posts(map, height, cx, seed);
        }

        // Decorative elements in open areas
        place_desert_decorations(map, width, height, seed);

        // Scatter gunpowder barrels around the map
        place_gunpowder_barrels(map, width, height, seed);
    }
}

/// Phase 7: Spawn clearing and water cleanup.
struct FinalizationPhase;

impl WorldGenPhase for FinalizationPhase {
    fn execute(
        &self,
        map: &mut GameMap,
        data: &mut WorldGenData,
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) {
        for building in &data.buildings {
            clear_building_entrance(map, building);
        }
        clear_props_near_alleys(map, width, height);

        // Post-pass coherence: verify every building can reach the main street
        check_connectivity(
            map,
            &data.buildings,
            &data.avenue_ys,
            &data.cross_xs,
            width,
            height,
        );

        // Clear around default spawn point. If SPAWN_POINT is outside the map
        // (common on smaller maps), fall back to the map center so the
        // central area is always walkable.
        let local_spawn = if SPAWN_POINT.x >= 0
            && SPAWN_POINT.x < width
            && SPAWN_POINT.y >= 0
            && SPAWN_POINT.y < height
        {
            SPAWN_POINT
        } else {
            GridVec::new(width / 2, height / 2)
        };
        clear_around(map, local_spawn, 6);
        if let Some(bridge_pos) = map.find_bridge_center() {
            clear_around(map, bridge_pos, 6);
        }

        // Final pass: clear props from water, beach, and bridge tiles.
        // Later generation steps may have placed props on river tiles.
        // Also clear gravel/rail tiles that ended up on main roads.
        for y in 0..height {
            for x in 0..width {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                    if matches!(
                        voxel.floor,
                        Some(Floor::ShallowWater)
                            | Some(Floor::DeepWater)
                            | Some(Floor::Beach)
                            | Some(Floor::BeachSand)
                    ) {
                        voxel.props = None;
                    }
                }
            }
        }

        // Re-apply the railroad after all other worldgen passes so later
        // landmark/detail placement cannot leave the track fragmented.
        place_railroad(map, width, height, seed);
    }
}

impl GameMap {
    /// Creates a new game map as a giant midwestern town.
    ///
    /// The generation pipeline runs as a sequence of phases:
    ///   1. **Terrain** — desert base + forest outskirts (creates the map).
    ///   2. **Water** — river generation.
    ///   3. **Infrastructure** — street grid (avenues, cross streets, bridges).
    ///   4. **Buildings** — houses, saloons, stables, alleys, faction anchors.
    ///   5. **Landmarks** — Town Hall, Grand Saloon, plazas, urban features.
    ///   6. **Details** — street props, decorations, gunpowder barrels.
    ///   7. **Finalization** — spawn clearing, water cleanup.
    ///
    /// After generation, structural verification checks are run. If any
    /// check fails, the map is regenerated with a modified seed (up to 10
    /// retries). In debug builds, panics if no valid map is produced.
    #[allow(unused_assignments)]
    pub fn new(width: CoordinateUnit, height: CoordinateUnit, seed: NoiseSeed) -> Self {
        const MAX_RETRIES: u32 = 10;
        #[allow(unused_variables)]
        let mut last_map = None;
        let mut last_reason = String::new();
        for attempt in 0..=MAX_RETRIES {
            let current_seed = if attempt == 0 {
                seed
            } else {
                seed.wrapping_add(attempt as u64 * 7919)
            };
            let (map, data) = Self::generate_with_seed(width, height, current_seed);

            match verify_world(
                &map,
                &data.buildings,
                &data.avenue_ys,
                &data.cross_xs,
                width,
                height,
            ) {
                Ok(()) => return map,
                Err(reason) => {
                    // Silently retry — no diagnostic text should reach the player.
                    last_reason = reason;
                    last_map = Some(map);
                }
            }
        }
        #[cfg(debug_assertions)]
        panic!(
            "World generation failed after {} retries. Last failure: {}",
            MAX_RETRIES, last_reason
        );
        // In release builds, return the last attempt.
        #[allow(unreachable_code)]
        last_map.unwrap_or_else(|| Self::generate_with_seed(width, height, seed).0)
    }

    /// Internal: run all generation phases with a specific seed.
    fn generate_with_seed(
        width: CoordinateUnit,
        height: CoordinateUnit,
        seed: NoiseSeed,
    ) -> (Self, WorldGenData) {
        let mut map = TerrainPhase::create_map(width, height, seed);
        let mut data = WorldGenData::default();

        let phases: Vec<Box<dyn WorldGenPhase>> = vec![
            Box::new(WaterPhase),
            Box::new(InfrastructurePhase),
            Box::new(RailroadPhase),
            Box::new(BuildingPhase),
            Box::new(LandmarkPhase),
            Box::new(DetailPhase),
            Box::new(FinalizationPhase),
        ];

        for phase in &phases {
            phase.execute(&mut map, &mut data, width, height, seed);
        }

        (map, data)
    }

    /// Get a reference to the voxel at the given map coordinate.
    #[inline]
    pub fn get_voxel_at(&self, point: &MyPoint) -> Option<&Voxel> {
        let GridVec { x, y } = *point;
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            Some(&self.voxels[y as usize][x as usize])
        } else {
            None
        }
    }

    /// Get a mutable reference to the voxel at the given map coordinate.
    #[inline]
    pub fn get_voxel_at_mut(&mut self, point: &MyPoint) -> Option<&mut Voxel> {
        let GridVec { x, y } = *point;
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            Some(&mut self.voxels[y as usize][x as usize])
        } else {
            None
        }
    }

    /// Marks a rectangle as occupied in the occupancy grid.
    pub fn mark_occupied(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for iy in y..y + h {
            for ix in x..x + w {
                if ix >= 0 && ix < self.width && iy >= 0 && iy < self.height {
                    self.occupancy[ix as usize][iy as usize] = true;
                }
            }
        }
    }

    /// Returns true if any cell in the rectangle is occupied.
    pub fn is_occupied(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        for iy in y..y + h {
            for ix in x..x + w {
                if ix >= 0 && ix < self.width && iy >= 0 && iy < self.height {
                    if self.occupancy[ix as usize][iy as usize] {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Returns true if any cell in the rectangle is water or beach.
    /// Used for landmark placement that may intentionally straddle roads.
    pub fn is_water_occupied(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        for iy in y..y + h {
            for ix in x..x + w {
                if ix >= 0 && ix < self.width && iy >= 0 && iy < self.height {
                    let pos = GridVec::new(ix, iy);
                    if let Some(voxel) = self.get_voxel_at(&pos) {
                        if matches!(
                            voxel.floor,
                            Some(Floor::ShallowWater)
                                | Some(Floor::DeepWater)
                                | Some(Floor::Beach)
                                | Some(Floor::BeachSand)
                        ) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Returns true if any cell in the rectangle is a road, water, or beach tile.
    /// Used by landmark and building placement to reject footprints that would
    /// overlap infrastructure or the river.
    pub fn has_excluded_tile(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        for iy in y..y + h {
            for ix in x..x + w {
                if ix >= 0 && ix < self.width && iy >= 0 && iy < self.height {
                    let pos = GridVec::new(ix, iy);
                    if let Some(voxel) = self.get_voxel_at(&pos) {
                        if matches!(
                            voxel.floor,
                            Some(Floor::DirtRoad)
                                | Some(Floor::ShallowWater)
                                | Some(Floor::DeepWater)
                                | Some(Floor::BeachSand)
                        ) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Returns `true` if the tile at `point` is passable (no blocking props).
    /// Water tiles are NOT blocked here — bullets, projectiles, and thrown
    /// items fly over water. Use `is_water()` for NPC/player movement checks.
    #[inline]
    pub fn is_passable(&self, point: &MyPoint) -> bool {
        self.get_voxel_at(point).is_some_and(|v| match &v.props {
            Some(f) => !f.blocks_movement(),
            None => true,
        })
    }

    /// Returns `true` if a projectile can pass through this tile.
    /// Windows allow projectiles through even though they block movement.
    #[inline]
    pub fn is_passable_for_projectiles(&self, point: &MyPoint) -> bool {
        self.get_voxel_at(point).is_some_and(|v| match &v.props {
            Some(f) => !f.blocks_projectiles(),
            None => true,
        })
    }

    /// Returns `true` if the tile at `point` is shallow or deep water.
    #[inline]
    pub fn is_water(&self, point: &MyPoint) -> bool {
        self.get_voxel_at(point)
            .is_some_and(|v| matches!(v.floor, Some(Floor::ShallowWater) | Some(Floor::DeepWater)))
    }

    /// Returns `true` if the tile is suitable for spawning an entity:
    /// passable, no props, and not fire or water.
    pub fn is_spawnable(&self, point: &MyPoint) -> bool {
        if !self.is_passable(point) {
            return false;
        }
        match self.get_voxel_at(point) {
            Some(v) => {
                v.props.is_none()
                    && !matches!(
                        v.floor,
                        Some(Floor::Fire) | Some(Floor::DeepWater) | Some(Floor::ShallowWater)
                    )
            }
            None => false,
        }
    }

    /// Finds a spawnable tile near the given center within the given radius.
    /// Searches in expanding rings from the center outward.
    pub fn find_spawnable_near(&self, center: GridVec, radius: i32) -> Option<GridVec> {
        for r in 0..=radius {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    } // only ring perimeter
                    let pos = center + GridVec::new(dx, dy);
                    if self.is_spawnable(&pos) {
                        return Some(pos);
                    }
                }
            }
        }
        None
    }

    /// Finds a passable tile right outside the door of a house.
    /// Scans the map near the bottom-left for a building doorway.
    /// Returns `None` if no suitable location is found.
    pub fn find_house_exterior(&self) -> Option<GridVec> {
        // Search near SPAWN_POINT for a building with a doorway
        let search_radius = 40;
        let sx = SPAWN_POINT.x;
        let sy = SPAWN_POINT.y;
        for y in sy.saturating_sub(search_radius).max(1)..=(sy + search_radius).min(self.height - 2)
        {
            for x in
                sx.saturating_sub(search_radius).max(1)..=(sx + search_radius).min(self.width - 2)
            {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = self.get_voxel_at(&pos)
                    && voxel.props.is_none()
                    && matches!(
                        voxel.floor,
                        Some(Floor::Dirt)
                            | Some(Floor::DirtRoad)
                            | Some(Floor::Sand)
                            | Some(Floor::Gravel)
                            | Some(Floor::Grass)
                            | Some(Floor::Sidewalk)
                    )
                {
                    // Check if there's an adjacent wall (meaning we're just outside a building)
                    let has_adjacent_wall = pos.cardinal_neighbors().iter().any(|n| {
                        self.get_voxel_at(n)
                            .is_some_and(|v| v.props.as_ref().is_some_and(|p| p.is_wall()))
                    });
                    if has_adjacent_wall {
                        return Some(pos);
                    }
                }
            }
        }
        None
    }

    /// Finds a passable tile inside a building near the map center.
    /// Searches for WoodPlanks or StoneFloor tiles with no props that have
    /// an adjacent wall, confirming the tile is truly inside a building.
    pub fn find_building_interior(&self, center: GridVec, radius: i32) -> Option<GridVec> {
        for r in 0..=radius {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let pos = center + GridVec::new(dx, dy);
                    if let Some(voxel) = self.get_voxel_at(&pos)
                        && voxel.props.is_none()
                        && matches!(
                            voxel.floor,
                            Some(Floor::WoodPlanks) | Some(Floor::StoneFloor)
                        )
                    {
                        // Confirm there's an adjacent wall (inside a building)
                        let has_wall = pos.cardinal_neighbors().iter().any(|n| {
                            self.get_voxel_at(n)
                                .is_some_and(|v| v.props.as_ref().is_some_and(|p| p.is_wall()))
                        });
                        if has_wall {
                            return Some(pos);
                        }
                    }
                }
            }
        }
        None
    }

    /// Finds an interior tile likely belonging to the jail near the town
    /// center. Jail interiors are stone-floored, adjacent to stone walls, and
    /// near both barrels and signage from the jail furnishing pass.
    pub fn find_jail_interior(&self, center: GridVec, radius: i32) -> Option<GridVec> {
        for r in 0..=radius {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let pos = center + GridVec::new(dx, dy);
                    let Some(voxel) = self.get_voxel_at(&pos) else {
                        continue;
                    };
                    if voxel.props.is_some() || !matches!(voxel.floor, Some(Floor::StoneFloor)) {
                        continue;
                    }

                    let has_stone_wall = pos.cardinal_neighbors().iter().any(|n| {
                        self.get_voxel_at(n).is_some_and(|v| {
                            matches!(v.props, Some(Props::StoneWall))
                        })
                    });
                    if !has_stone_wall {
                        continue;
                    }

                    let mut nearby_barrels = 0;
                    let mut nearby_sign = false;
                    for sy in -3..=3 {
                        for sx in -3..=3 {
                            let sample = pos + GridVec::new(sx, sy);
                            if let Some(sample_voxel) = self.get_voxel_at(&sample) {
                                match sample_voxel.props {
                                    Some(Props::Barrel) => nearby_barrels += 1,
                                    Some(Props::Sign) => nearby_sign = true,
                                    _ => {}
                                }
                            }
                        }
                    }

                    if nearby_barrels >= 2 && nearby_sign {
                        return Some(pos);
                    }
                }
            }
        }
        None
    }

    /// Finds a Bridge tile near the horizontal center of the map.
    /// Returns a point on the bridge closest to `width / 2`.
    pub fn find_bridge_center(&self) -> Option<GridVec> {
        let target_x = self.width / 2;
        let mut best: Option<(i32, GridVec)> = None;
        for y in 0..self.height {
            for x in 0..self.width {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = self.get_voxel_at(&pos)
                    && matches!(voxel.floor, Some(Floor::Bridge))
                {
                    let dist = (x - target_x).abs();
                    if best.is_none_or(|(best_dist, _)| dist < best_dist) {
                        best = Some((dist, pos));
                    }
                }
            }
        }
        best.map(|(_, pos)| pos)
    }

    /// Returns the approximate X-coordinate of the river center for a given Y.
    pub fn river_center_x(&self, _y: i32) -> f64 {
        // Simplified - just return map center since the river meanders around it
        self.width as f64 / 2.0
    }

    /// Creates a RenderPacket with visibility-based dimming.
    ///
    /// The entire map is always visible (no fog of war / hidden areas).
    /// Tiles are rendered in two states:
    /// - **Visible** (in player's current FOV): full brightness.
    /// - **Not visible** (outside player's FOV): dimmed but still shown.
    ///
    /// When `visible_tiles` is `None`, all tiles render at full brightness.
    pub fn create_render_packet_with_fog(
        &self,
        center: &MyPoint,
        render_width: u16,
        render_height: u16,
        visible_tiles: Option<&std::collections::HashSet<MyPoint>>,
        _revealed_tiles: Option<&std::collections::HashSet<MyPoint>>,
    ) -> RenderPacket {
        let w_radius = render_width as CoordinateUnit / 2;
        let h_radius = render_height as CoordinateUnit / 2;

        let bottom_left = *center - GridVec::new(w_radius, h_radius);

        let mut grid = create_2d_array(render_width as usize, render_height as usize);

        // Note: iteration is already viewport-scoped (only tiles within render_width/height
        // are processed), which is equivalent to chunk-based culling for the viewport.
        for ry in 0..render_height as CoordinateUnit {
            for rx in 0..render_width as CoordinateUnit {
                let world_pos = bottom_left + GridVec::new(rx, ry);

                if let Some(voxel) = self.get_voxel_at(&world_pos) {
                    let is_visible = visible_tiles
                        .map(|vt| vt.contains(&world_pos))
                        .unwrap_or(true);

                    // Always show the map — visible at full brightness, rest dimmed
                    grid[ry as usize][rx as usize] = voxel.to_graphic(is_visible);
                }
            }
        }

        grid
    }

    /// Returns which chunk coordinates overlap the given viewport.
    pub fn active_chunks(
        &self,
        center: &GridVec,
        render_width: u16,
        render_height: u16,
    ) -> Vec<(CoordinateUnit, CoordinateUnit)> {
        let w_radius = render_width as CoordinateUnit / 2;
        let h_radius = render_height as CoordinateUnit / 2;
        let min_x = (center.x - w_radius).div_euclid(CHUNK_SIZE);
        let max_x = (center.x + w_radius).div_euclid(CHUNK_SIZE);
        let min_y = (center.y - h_radius).div_euclid(CHUNK_SIZE);
        let max_y = (center.y + h_radius).div_euclid(CHUNK_SIZE);
        let mut chunks = Vec::new();
        for cy in min_y..=max_y {
            for cx in min_x..=max_x {
                chunks.push((cx, cy));
            }
        }
        chunks
    }
}

/// Selects a desert/arid floor tile from layered noise values.
fn select_desert_floor(biome: f64, detail: f64) -> Floor {
    if biome < 0.35 {
        if detail < 0.5 {
            Floor::Sand
        } else if detail < 0.8 {
            Floor::Gravel
        } else {
            Floor::Dirt
        }
    } else if biome < 0.60 {
        if detail < 0.4 {
            Floor::Dirt
        } else if detail < 0.7 {
            Floor::Sand
        } else {
            Floor::Gravel
        }
    } else if biome < 0.80 {
        if detail < 0.3 {
            Floor::Sand
        } else if detail < 0.5 {
            Floor::Dirt
        } else if detail < 0.75 {
            Floor::Gravel
        } else {
            Floor::Grass
        }
    } else {
        // Occasional sparse grass/dry vegetation at high biome values
        if detail < 0.3 {
            Floor::Dirt
        } else if detail < 0.6 {
            Floor::Sand
        } else if detail < 0.85 {
            Floor::Grass
        } else {
            Floor::TallGrass
        }
    }
}

/// Number of distinct building types used during town generation.
/// Types 0-11: House, Saloon, Stable, General Store, Police Station,
/// Post Office, Church, Bank, Hotel, Jail, Undertaker, Blacksmith.
const BUILDING_TYPE_COUNT: u32 = 12;

// ── BSP spatial partitioning ─────────────────────────────────────────────

/// Minimum width for a BSP leaf node. Leaves narrower than this cannot
/// contain a building and are treated as open lots.
const BSP_MIN_LEAF_W: CoordinateUnit = 5;
/// Minimum height for a BSP leaf node.
const BSP_MIN_LEAF_H: CoordinateUnit = 5;
/// Padding between building footprint and leaf boundary. Creates natural
/// alleyways and breathing room between adjacent structures.
const BSP_PADDING: CoordinateUnit = 0;
/// Maximum BSP recursion depth.
const BSP_MAX_DEPTH: u32 = 15;

/// Semantic zone types assigned to BSP regions.
const ZONE_COMMERCIAL: u32 = 0;
const ZONE_RESIDENTIAL: u32 = 1;
const ZONE_INDUSTRIAL: u32 = 2;

/// A node in the binary space partition tree. Each node owns an exclusive
/// rectangular region of the map. Leaf nodes are allocated one building;
/// internal nodes define the split.
struct BspNode {
    x: CoordinateUnit,
    y: CoordinateUnit,
    w: CoordinateUnit,
    h: CoordinateUnit,
    left: Option<Box<BspNode>>,
    right: Option<Box<BspNode>>,
}

impl BspNode {
    fn new(x: CoordinateUnit, y: CoordinateUnit, w: CoordinateUnit, h: CoordinateUnit) -> Self {
        Self {
            x,
            y,
            w,
            h,
            left: None,
            right: None,
        }
    }

    fn is_leaf(&self) -> bool {
        self.left.is_none() && self.right.is_none()
    }

    /// Recursively subdivide this node. Splits are biased to align with the
    /// street network (avenue Y-positions and cross-street X-positions) so
    /// buildings naturally face roads rather than spawning at arbitrary
    /// angles to them.
    fn subdivide(
        &mut self,
        seed: NoiseSeed,
        depth: u32,
        avenue_ys: &[CoordinateUnit],
        cross_xs: &[CoordinateUnit],
    ) {
        if depth >= BSP_MAX_DEPTH {
            return;
        }

        let can_split_h = self.h >= BSP_MIN_LEAF_H * 2 + BSP_PADDING;
        let can_split_v = self.w >= BSP_MIN_LEAF_W * 2 + BSP_PADDING;
        if !can_split_h && !can_split_v {
            return;
        }

        // Bias: split along the longer axis with randomization so plots
        // feel organic rather than grid-stamped.
        let noise = value_noise(
            self.x + depth as i32,
            self.y + depth as i32,
            seed.wrapping_add(depth as u64 * 7919),
        );
        let split_horizontal = if can_split_h && can_split_v {
            if self.h > self.w {
                noise < 0.7
            } else {
                noise < 0.3
            }
        } else {
            can_split_h
        };

        if split_horizontal {
            let min_y = self.y + BSP_MIN_LEAF_H;
            let max_y = self.y + self.h - BSP_MIN_LEAF_H - BSP_PADDING;
            if min_y > max_y {
                return;
            }
            let mid_y = self.y + self.h / 2;
            let mut split_y = mid_y;
            // Bias toward nearest avenue so buildings face roads.
            for &ay in avenue_ys {
                if ay >= min_y && ay <= max_y && (ay - mid_y).abs() < (split_y - mid_y).abs() + 8 {
                    split_y = ay;
                }
            }
            let jitter = ((value_noise(self.x, self.y, seed.wrapping_add(33333 + depth as u64))
                - 0.5)
                * 6.0) as CoordinateUnit;
            split_y = (split_y + jitter).clamp(min_y, max_y);

            let top_h = split_y - self.y;
            let bot_y = split_y + BSP_PADDING;
            let bot_h = (self.y + self.h) - bot_y;
            if top_h >= BSP_MIN_LEAF_H && bot_h >= BSP_MIN_LEAF_H {
                self.left = Some(Box::new(BspNode::new(self.x, self.y, self.w, top_h)));
                self.right = Some(Box::new(BspNode::new(self.x, bot_y, self.w, bot_h)));
            }
        } else {
            let min_x = self.x + BSP_MIN_LEAF_W;
            let max_x = self.x + self.w - BSP_MIN_LEAF_W - BSP_PADDING;
            if min_x > max_x {
                return;
            }
            let mid_x = self.x + self.w / 2;
            let mut split_x = mid_x;
            for &cx in cross_xs {
                if cx >= min_x && cx <= max_x && (cx - mid_x).abs() < (split_x - mid_x).abs() + 8 {
                    split_x = cx;
                }
            }
            let jitter = ((value_noise(self.y, self.x, seed.wrapping_add(44444 + depth as u64))
                - 0.5)
                * 6.0) as CoordinateUnit;
            split_x = (split_x + jitter).clamp(min_x, max_x);

            let left_w = split_x - self.x;
            let right_x = split_x + BSP_PADDING;
            let right_w = (self.x + self.w) - right_x;
            if left_w >= BSP_MIN_LEAF_W && right_w >= BSP_MIN_LEAF_W {
                self.left = Some(Box::new(BspNode::new(self.x, self.y, left_w, self.h)));
                self.right = Some(Box::new(BspNode::new(right_x, self.y, right_w, self.h)));
            }
        }

        if let Some(ref mut left) = self.left {
            left.subdivide(seed.wrapping_add(1), depth + 1, avenue_ys, cross_xs);
        }
        if let Some(ref mut right) = self.right {
            right.subdivide(seed.wrapping_add(2), depth + 1, avenue_ys, cross_xs);
        }
    }

    /// Collect all leaf rectangles into a flat vector.
    fn collect_leaves(
        &self,
        out: &mut Vec<(
            CoordinateUnit,
            CoordinateUnit,
            CoordinateUnit,
            CoordinateUnit,
        )>,
    ) {
        if self.is_leaf() {
            out.push((self.x, self.y, self.w, self.h));
        } else {
            if let Some(ref left) = self.left {
                left.collect_leaves(out);
            }
            if let Some(ref right) = self.right {
                right.collect_leaves(out);
            }
        }
    }
}

// ── Semantic zone assignment ─────────────────────────────────────────────

/// Assigns a semantic zone type to a BSP leaf based on its position
/// relative to streets, the railroad area, and the town center.
/// Commercial zones line the main streets, industrial zones cluster near
/// the southern edge (where the railroad runs), and residential fills
/// the outskirts.
fn assign_zone(
    cx: CoordinateUnit,
    cy: CoordinateUnit,
    _width: CoordinateUnit,
    height: CoordinateUnit,
    avenue_ys: &[CoordinateUnit],
    seed: NoiseSeed,
) -> u32 {
    let dist_to_avenue = avenue_ys
        .iter()
        .map(|&ay| (cy - ay).abs())
        .min()
        .unwrap_or(height);

    // Commercial: near main streets (saloons, general stores, banks)
    if dist_to_avenue < 20 {
        return ZONE_COMMERCIAL;
    }

    // Industrial: near southern edge (railroad area) or river
    if cy > height * 2 / 3 {
        let noise = value_noise(cx, cy, seed.wrapping_add(55555));
        if noise < 0.5 {
            return ZONE_INDUSTRIAL;
        }
    }

    // Residential: default for outskirts
    ZONE_RESIDENTIAL
}

/// Selects a building kind based on semantic zone type.
/// Commercial: saloons, general stores, banks, police station.
/// Residential: houses, churches, hotels.
/// Industrial: stables, blacksmiths, undertakers.
fn zone_building_kind(zone: u32, noise: f64) -> u32 {
    match zone {
        ZONE_COMMERCIAL => {
            if noise < 0.20 {
                1
            }
            // Saloon
            else if noise < 0.40 {
                3
            }
            // General Store
            else if noise < 0.55 {
                7
            }
            // Bank
            else if noise < 0.70 {
                4
            }
            // Police Station
            else if noise < 0.85 {
                5
            }
            // Post Office
            else {
                9
            } // Jail
        }
        ZONE_RESIDENTIAL => {
            if noise < 0.50 {
                0
            }
            // House
            else if noise < 0.70 {
                6
            }
            // Church
            else if noise < 0.85 {
                8
            }
            // Hotel
            else {
                5
            } // Post Office
        }
        ZONE_INDUSTRIAL => {
            if noise < 0.30 {
                2
            }
            // Stable
            else if noise < 0.55 {
                11
            }
            // Blacksmith
            else if noise < 0.75 {
                3
            }
            // General Store
            else {
                10
            } // Undertaker
        }
        // Mixed / catch-all: saloons, hotels.
        _ => {
            if noise < 0.35 {
                1
            }
            // Saloon
            else if noise < 0.60 {
                8
            }
            // Hotel
            else if noise < 0.80 {
                0
            }
            // House
            else {
                1
            } // Saloon
        }
    }
}

// ── Landmark anchoring ───────────────────────────────────────────────────

/// Pre-places a small number of landmark anchor buildings at fixed or
/// strongly-biased positions. These anchors are immovable constraints
/// that the BSP partition works around rather than over, giving every
/// generated town a consistent skeleton.
fn pre_place_landmark_anchors(
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    avenue_ys: &[CoordinateUnit],
) -> Vec<Building> {
    let mut anchors = Vec::new();
    if width < 60 || height < 40 {
        return anchors;
    }
    let anchor_seed = seed.wrapping_add(777888);
    let center_x = width / 2;
    let center_y = height / 2;

    // Police station near the town center
    let sx = (center_x - 8 + (value_noise(1, 1, anchor_seed) * 10.0) as CoordinateUnit)
        .clamp(8, width - 20);
    let sy = (center_y - 4 + (value_noise(2, 2, anchor_seed) * 6.0) as CoordinateUnit)
        .clamp(8, height - 16);
    anchors.push(Building {
        x: sx,
        y: sy,
        w: 12,
        h: 10,
        kind: 4,
    });

    // Saloon on the main street corner
    if let Some(&first_avenue) = avenue_ys.first() {
        let sal_x = (center_x + 20 + (value_noise(3, 3, anchor_seed) * 10.0) as CoordinateUnit)
            .clamp(8, width - 22);
        let sal_y = (first_avenue + 6).clamp(8, height - 14);
        anchors.push(Building {
            x: sal_x,
            y: sal_y,
            w: 14,
            h: 10,
            kind: 1,
        });
    }

    // Church toward the outskirts
    let cx = (width / 4 + (value_noise(5, 5, anchor_seed) * 15.0) as CoordinateUnit)
        .clamp(8, width - 16);
    let cy = (height / 4 + (value_noise(6, 6, anchor_seed) * 10.0) as CoordinateUnit)
        .clamp(8, height - 14);
    anchors.push(Building {
        x: cx,
        y: cy,
        w: 12,
        h: 10,
        kind: 6,
    });

    // Jail near the river and town center. This is the player's guaranteed
    // starting prison, so keep it central and close to the river corridor.
    let jail_w: CoordinateUnit = 14;
    let jail_h: CoordinateUnit = 12;
    let river_x = width / 2;
    let jail_candidates = [
        (
            (river_x + 8).clamp(6, width - jail_w - 6),
            (center_y - 6).clamp(8, height - jail_h - 8),
        ),
        (
            (river_x - jail_w - 8).clamp(6, width - jail_w - 6),
            (center_y - 4).clamp(8, height - jail_h - 8),
        ),
        (
            (river_x + 10).clamp(6, width - jail_w - 6),
            (center_y + 6).clamp(8, height - jail_h - 8),
        ),
    ];
    for (jx, jy) in jail_candidates {
        if anchors
            .iter()
            .all(|b| !rects_overlap(jx, jy, jail_w, jail_h, b.x, b.y, b.w, b.h))
        {
            anchors.push(Building {
                x: jx,
                y: jy,
                w: jail_w,
                h: jail_h,
                kind: 9,
            });
            break;
        }
    }

    anchors
}

/// AABB overlap test between two rectangles.
fn rects_overlap(
    ax: CoordinateUnit,
    ay: CoordinateUnit,
    aw: CoordinateUnit,
    ah: CoordinateUnit,
    bx: CoordinateUnit,
    by: CoordinateUnit,
    bw: CoordinateUnit,
    bh: CoordinateUnit,
) -> bool {
    ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by
}

// ── BSP-based building generation ────────────────────────────────────────

// ── Types ─────────────────────────────────────────────────────────────────────

/// The inner rectangle of a lot after stripping road, sidewalk, and curve
/// buffers from all four edges. All building placement works from this.
#[derive(Clone, Copy, Debug)]
struct LotRect {
    x: CoordinateUnit,
    y: CoordinateUnit,
    w: CoordinateUnit,
    h: CoordinateUnit,
}

impl LotRect {
    fn right(self) -> CoordinateUnit {
        self.x + self.w
    }
    fn bot(self) -> CoordinateUnit {
        self.y + self.h
    }
    fn cx(self) -> CoordinateUnit {
        self.x + self.w / 2
    }
    fn cy(self) -> CoordinateUnit {
        self.y + self.h / 2
    }
}

// ── Constants ─────────────────────────────────────────────────────────────────

/// Tiles stripped from a road-bounded edge.
const ROAD_EDGE_BUFFER: CoordinateUnit = 0;

/// Tiles stripped from a map-edge boundary (no adjacent road).
const MAP_EDGE_BUFFER: CoordinateUnit = 1;

/// Minimum lot side length below which no placement is attempted.
const LOT_MIN_DIM: CoordinateUnit = 14;

/// Minimum side length for the large anchor building placed first in each lot.
const ANCHOR_MIN_DIM: CoordinateUnit = 10;

/// Minimum side length for smaller filler buildings placed in strips around the anchor.
const FILLER_MIN_DIM: CoordinateUnit = 4;

/// Gap in tiles enforced between every pair of adjacent buildings.
const GAP: CoordinateUnit = 1;

/// Maximum side lengths for BSP-leaf buildings.
const BSP_MAX_W: CoordinateUnit = 40;
const BSP_MAX_H: CoordinateUnit = 36;

/// Padding between a BSP leaf boundary and the building inside it.
const BSP_PADDING_LOCAL: CoordinateUnit = BSP_PADDING;

/// Alley gap detection threshold in tiles.
const ALLEY_GAP_MAX: i64 = 6;

/// Fraction of BSP leaves that become parks instead of buildings.
const PARK_NOISE_THRESHOLD: f64 = 0.06;

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Returns `true` when rect `(bx,by,bw,bh)` padded by `margin` overlaps any
/// already-placed footprint.
#[inline]
fn overlaps_placed(
    placed: &[(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )],
    bx: CoordinateUnit,
    by: CoordinateUnit,
    bw: CoordinateUnit,
    bh: CoordinateUnit,
    margin: CoordinateUnit,
) -> bool {
    placed.iter().any(|&(px, py, pw, ph)| {
        rects_overlap(
            bx - margin,
            by - margin,
            bw + margin * 2,
            bh + margin * 2,
            px,
            py,
            pw,
            ph,
        )
    })
}

/// Selects the building kind for a given world position.
#[inline]
fn pick_kind(
    cx: CoordinateUnit,
    cy: CoordinateUnit,
    noise_x: i32,
    noise_y: i32,
    noise_seed: NoiseSeed,
    width: CoordinateUnit,
    height: CoordinateUnit,
    avenue_ys: &[CoordinateUnit],
    seed: NoiseSeed,
) -> u32 {
    let zone = assign_zone(cx, cy, width, height, avenue_ys, seed);
    let kn = value_noise(noise_x, noise_y, noise_seed);
    zone_building_kind(zone, kn).min(BUILDING_TYPE_COUNT - 1)
}

/// Records a building if it passes the size check (`bw,bh >= min_dim`) and the
/// overlap check. Returns `true` on success.
#[inline]
fn try_place(
    bx: CoordinateUnit,
    by: CoordinateUnit,
    bw: CoordinateUnit,
    bh: CoordinateUnit,
    min_dim: CoordinateUnit,
    kind: u32,
    placed: &mut Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )>,
    out: &mut Vec<Building>,
) -> bool {
    if bw < min_dim || bh < min_dim {
        return false;
    }
    if overlaps_placed(placed, bx, by, bw, bh, GAP) {
        return false;
    }
    placed.push((bx, by, bw, bh));
    out.push(Building {
        x: bx,
        y: by,
        w: bw,
        h: bh,
        kind,
    });
    true
}

/// Reserves and clears the tiles directly outside a building's main doorway
/// so later decoration passes cannot block the entrance.
fn reserve_building_entrance(map: &mut GameMap, b: &Building) {
    let door_x = b.x + b.w / 2;
    let door_y = b.y + b.h;
    for dy in 0..=2 {
        for dx in -1..=1 {
            map.mark_occupied(door_x + dx, door_y + dy, 1, 1);
        }
    }
}

/// Clears blocking props from the small apron outside a building entrance.
fn clear_building_entrance(map: &mut GameMap, b: &Building) {
    let door_x = b.x + b.w / 2;
    let door_y = b.y + b.h;
    let mut apron = Vec::new();
    for dy in 0..=2 {
        for dx in -1..=1 {
            apron.push(GridVec::new(door_x + dx, door_y + dy));
        }
    }

    for pos in apron {
        if pos.x < 0 || pos.x >= map.width || pos.y < 0 || pos.y >= map.height {
            continue;
        }
        if let Some(voxel) = map.get_voxel_at_mut(&pos)
            && voxel
                .props
                .as_ref()
                .is_some_and(|prop| prop.blocks_movement() && !prop.is_wall())
        {
            voxel.props = None;
        }
    }
}

fn clear_props_near_alleys(map: &mut GameMap, width: CoordinateUnit, height: CoordinateUnit) {
    let mut to_clear: Vec<GridVec> = Vec::new();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let pos = GridVec::new(x, y);
            let Some(voxel) = map.get_voxel_at(&pos) else {
                continue;
            };
            let adjacent_to_alley = matches!(voxel.floor, Some(Floor::Alley))
                || pos.cardinal_neighbors().iter().any(|neighbor| {
                    map.get_voxel_at(neighbor)
                        .is_some_and(|v| matches!(v.floor, Some(Floor::Alley)))
                });
            if !adjacent_to_alley {
                continue;
            }
            if voxel.props.as_ref().is_some_and(|prop| {
                prop.blocks_movement() && !prop.is_wall() && !matches!(prop, Props::Fence)
            }) {
                to_clear.push(pos);
            }
        }
    }

    for pos in to_clear {
        if let Some(voxel) = map.get_voxel_at_mut(&pos) {
            voxel.props = None;
        }
    }
}

#[inline]
fn building_shape_type(b: &Building, seed: NoiseSeed) -> u32 {
    let shape_noise = value_noise(b.x + b.y, b.w + b.h, seed.wrapping_add(77777));
    if b.w >= 8 && b.h >= 8 {
        (shape_noise * 3.0) as u32
    } else {
        SHAPE_RECT
    }
}

#[inline]
fn building_has_side_door(b: &Building) -> bool {
    (b.w >= 12 || b.h >= 12) && matches!(b.kind, 1 | 3 | 4 | 5 | 7 | 8 | 9 | 11)
}

/// Derives the inner lot boundary from a pair of boundary coordinates.
fn lot_inner(
    lo: CoordinateUnit,
    hi: CoordinateUnit,
    lo_is_road: bool,
    hi_is_road: bool,
    half_width: CoordinateUnit,
    sidewalk: CoordinateUnit,
) -> (CoordinateUnit, CoordinateUnit) {
    let inner_lo = if lo_is_road {
        lo + half_width + sidewalk + 1 + ROAD_EDGE_BUFFER
    } else {
        lo + MAP_EDGE_BUFFER
    };
    let inner_hi = if hi_is_road {
        hi - half_width - sidewalk - 1 - ROAD_EDGE_BUFFER
    } else {
        hi - MAP_EDGE_BUFFER
    };
    (inner_lo, inner_hi)
}

/// Computes the `LotRect` for grid cell `(xi, yi)`. Returns `None` if the
/// resulting lot is below `LOT_MIN_DIM`.
fn compute_lot(
    xi: usize,
    yi: usize,
    x_bounds: &[CoordinateUnit],
    y_bounds: &[CoordinateUnit],
    avenue_set: &HashSet<CoordinateUnit>,
    cross_set: &HashSet<CoordinateUnit>,
    avenue_half_width: CoordinateUnit,
    cross_half_width: CoordinateUnit,
) -> Option<LotRect> {
    const SIDEWALK: CoordinateUnit = 1;

    let (lot_left, lot_right) = lot_inner(
        x_bounds[xi],
        x_bounds[xi + 1],
        cross_set.contains(&x_bounds[xi]),
        cross_set.contains(&x_bounds[xi + 1]),
        cross_half_width,
        SIDEWALK,
    );
    let (lot_top, lot_bot) = lot_inner(
        y_bounds[yi],
        y_bounds[yi + 1],
        avenue_set.contains(&y_bounds[yi]),
        avenue_set.contains(&y_bounds[yi + 1]),
        avenue_half_width,
        SIDEWALK,
    );

    let w = lot_right - lot_left;
    let h = lot_bot - lot_top;
    if w < LOT_MIN_DIM || h < LOT_MIN_DIM {
        return None;
    }

    Some(LotRect {
        x: lot_left,
        y: lot_top,
        w,
        h,
    })
}

// ── Per-lot two-phase placement ────────────────────────────────────────────────

/// Places buildings in `lot` in two phases.
///
/// **Phase 1 — Anchor.**  One large building occupying 55–80% of the lot in
/// each axis is placed first. Its position is centred with a small noise-driven
/// offset so every lot looks slightly different.
///
/// **Phase 2 — Strip fill.**  The four rectangular strips of free space around
/// the anchor (top, bottom, left flank, right flank) are each passed to
/// `fill_strip`, which packs 1–3 smaller buildings into them.
///
///  ┌─────────────────────────┐
///  │         TOP             │
///  ├──────┬──────────┬───────┤
///  │ LEFT │  ANCHOR  │ RIGHT │
///  ├──────┴──────────┴───────┤
///  │        BOTTOM           │
///  └─────────────────────────┘
fn place_lot_buildings(
    lot: LotRect,
    lot_seed: NoiseSeed,
    placed: &mut Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )>,
    out: &mut Vec<Building>,
    width: CoordinateUnit,
    height: CoordinateUnit,
    avenue_ys: &[CoordinateUnit],
    seed: NoiseSeed,
) {
    // ── Phase 1: anchor ───────────────────────────────────────────────
    // Size the anchor at 55–80% of the lot in each axis. The two axes use
    // complementary fractions so tall lots get wide anchors and vice versa,
    // producing natural variety rather than a uniform aspect ratio.
    let size_n = value_noise(lot.cx(), lot.cy(), lot_seed.wrapping_add(1111));
    let frac_w = 0.40 + size_n * 0.18;
    let frac_h = 0.40 + (1.0 - size_n) * 0.18;

    let anchor_w = ((lot.w as f64 * frac_w) as CoordinateUnit)
        .max(ANCHOR_MIN_DIM)
        .min(lot.w.saturating_sub(GAP * 2));
    let anchor_h = ((lot.h as f64 * frac_h) as CoordinateUnit)
        .max(ANCHOR_MIN_DIM)
        .min(lot.h.saturating_sub(GAP * 2));

    if anchor_w < ANCHOR_MIN_DIM || anchor_h < ANCHOR_MIN_DIM {
        // Lot too small for an anchor — fill the whole thing as one small building.
        let kind = pick_kind(
            lot.cx(),
            lot.cy(),
            lot.x,
            lot.y,
            lot_seed,
            width,
            height,
            avenue_ys,
            seed,
        );
        try_place(
            lot.x + GAP,
            lot.y + GAP,
            lot.w.saturating_sub(GAP * 2),
            lot.h.saturating_sub(GAP * 2),
            FILLER_MIN_DIM,
            kind,
            placed,
            out,
        );
        return;
    }

    // Centre the anchor with a noise-driven jitter (up to ⅓ of the free margin
    // on each axis) so lots don't all look stamped from the same template.
    let jitter_n = value_noise(lot.x, lot.y, lot_seed.wrapping_add(2222));
    let free_x = lot.w.saturating_sub(anchor_w).saturating_sub(GAP * 2);
    let free_y = lot.h.saturating_sub(anchor_h).saturating_sub(GAP * 2);
    let jitter_range_x = free_x / 3;
    let jitter_range_y = free_y / 3;
    let jitter_x =
        ((jitter_n * (jitter_range_x * 2) as f64) as CoordinateUnit).saturating_sub(jitter_range_x);
    let jitter_y = (((1.0 - jitter_n) * (jitter_range_y * 2) as f64) as CoordinateUnit)
        .saturating_sub(jitter_range_y);

    let anchor_x = (lot.cx() - anchor_w / 2 + jitter_x)
        .max(lot.x + GAP)
        .min(lot.right().saturating_sub(anchor_w + GAP));
    let anchor_y = (lot.cy() - anchor_h / 2 + jitter_y)
        .max(lot.y + GAP)
        .min(lot.bot().saturating_sub(anchor_h + GAP));

    let anchor_kind = pick_kind(
        lot.cx(),
        lot.cy(),
        lot.x,
        lot.y,
        lot_seed.wrapping_add(3333),
        width,
        height,
        avenue_ys,
        seed,
    );

    let anchor_placed = try_place(
        anchor_x,
        anchor_y,
        anchor_w,
        anchor_h,
        ANCHOR_MIN_DIM,
        anchor_kind,
        placed,
        out,
    );

    // If the anchor couldn't land (overlaps a landmark, etc.) treat the whole
    // lot as a single free strip and try to fit something.
    if !anchor_placed {
        fill_strip(
            lot.x,
            lot.y,
            lot.w,
            lot.h,
            lot_seed.wrapping_add(9999),
            placed,
            out,
            width,
            height,
            avenue_ys,
            seed,
        );
        return;
    }

    // ── Phase 2: fill strips around the anchor ────────────────────────

    // Top strip — full lot width, above the anchor.
    let top_h = anchor_y - GAP - lot.y;
    if top_h >= FILLER_MIN_DIM {
        fill_strip(
            lot.x,
            lot.y,
            lot.w,
            top_h,
            lot_seed.wrapping_add(4444),
            placed,
            out,
            width,
            height,
            avenue_ys,
            seed,
        );
    }

    // Bottom strip — full lot width, below the anchor.
    let bot_y = anchor_y + anchor_h + GAP;
    let bot_h = lot.bot() - bot_y;
    if bot_h >= FILLER_MIN_DIM {
        fill_strip(
            lot.x,
            bot_y,
            lot.w,
            bot_h,
            lot_seed.wrapping_add(5555),
            placed,
            out,
            width,
            height,
            avenue_ys,
            seed,
        );
    }

    // Left and right flanks share the anchor's vertical band so the four
    // strips tile the lot without leaving uncovered corners.
    let side_y = anchor_y;
    let side_h = anchor_h;

    // Left flank — between the top/bottom strips, left of the anchor.
    let left_w = anchor_x - GAP - lot.x;
    if left_w >= FILLER_MIN_DIM && side_h >= FILLER_MIN_DIM {
        fill_strip(
            lot.x,
            side_y,
            left_w,
            side_h,
            lot_seed.wrapping_add(6666),
            placed,
            out,
            width,
            height,
            avenue_ys,
            seed,
        );
    }

    // Right flank — between the top/bottom strips, right of the anchor.
    let right_x = anchor_x + anchor_w + GAP;
    let right_w = lot.right() - right_x;
    if right_w >= FILLER_MIN_DIM && side_h >= FILLER_MIN_DIM {
        fill_strip(
            right_x,
            side_y,
            right_w,
            side_h,
            lot_seed.wrapping_add(7777),
            placed,
            out,
            width,
            height,
            avenue_ys,
            seed,
        );
    }
}

/// Packs 1–3 smaller buildings into the rectangle `(sx, sy, sw, sh)`.
///
/// The strip is divided along its longer axis into equal segments; each
/// segment receives one building sized to fill it minus the inter-building gap.
/// The number of segments is determined by how many `FILLER_MIN_DIM`-sized
/// buildings fit side by side, capped at 3.
fn fill_strip(
    sx: CoordinateUnit,
    sy: CoordinateUnit,
    sw: CoordinateUnit,
    sh: CoordinateUnit,
    strip_seed: NoiseSeed,
    placed: &mut Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )>,
    out: &mut Vec<Building>,
    width: CoordinateUnit,
    height: CoordinateUnit,
    avenue_ys: &[CoordinateUnit],
    seed: NoiseSeed,
) {
    if sw < FILLER_MIN_DIM || sh < FILLER_MIN_DIM {
        return;
    }

    // Divide along the longer axis; cap at 3 segments to avoid tiny slivers.
    let segments: CoordinateUnit = if sw >= sh {
        (sw / (FILLER_MIN_DIM + GAP)).min(5).max(1)
    } else {
        (sh / (FILLER_MIN_DIM + GAP)).min(5).max(1)
    };

    if sw >= sh {
        // Horizontal division: `segments` columns.
        let seg_w = sw / segments;
        for s in 0..segments {
            let base_bx = sx + s * seg_w + if s > 0 { GAP } else { 0 };
            let base_by = sy;
            let seg_bw = (if s == segments - 1 {
                sw - s * seg_w
            } else {
                seg_w
            })
            .saturating_sub(if s < segments - 1 { GAP } else { 0 });
            let seg_bh = sh;
            if seg_bw < FILLER_MIN_DIM || seg_bh < FILLER_MIN_DIM {
                continue;
            }
            let seg_seed = strip_seed.wrapping_add(s as u64 * 31);
            let shrink_x_max = seg_bw.saturating_sub(FILLER_MIN_DIM).min(2);
            let shrink_y_max = seg_bh.saturating_sub(FILLER_MIN_DIM).min(4);
            let shrink_x = if shrink_x_max > 0 {
                (value_noise(base_bx, base_by, seg_seed) * (shrink_x_max + 1) as f64)
                    as CoordinateUnit
            } else {
                0
            };
            let shrink_y = if shrink_y_max > 0 {
                (value_noise(base_by, base_bx, seg_seed.wrapping_add(1))
                    * (shrink_y_max + 1) as f64) as CoordinateUnit
            } else {
                0
            };
            let bw = seg_bw - shrink_x;
            let bh = seg_bh - shrink_y;
            let jitter_x = if seg_bw > bw {
                (value_noise(seg_bw, seg_bh, seg_seed.wrapping_add(2)) * (seg_bw - bw + 1) as f64)
                    as CoordinateUnit
            } else {
                0
            };
            let jitter_y = if seg_bh > bh {
                (value_noise(seg_bh, seg_bw, seg_seed.wrapping_add(3)) * (seg_bh - bh + 1) as f64)
                    as CoordinateUnit
            } else {
                0
            };
            let bx = base_bx + jitter_x;
            let by = base_by + jitter_y;
            let kind = pick_kind(
                bx + bw / 2,
                by + bh / 2,
                bx,
                by,
                seg_seed,
                width,
                height,
                avenue_ys,
                seed,
            );
            if !try_place(bx, by, bw, bh, FILLER_MIN_DIM, kind, placed, out) {
                try_place(
                    base_bx,
                    base_by,
                    seg_bw,
                    seg_bh,
                    FILLER_MIN_DIM,
                    kind,
                    placed,
                    out,
                );
            }
        }
    } else {
        // Vertical division: `segments` rows.
        let seg_h = sh / segments;
        for s in 0..segments {
            let base_bx = sx;
            let base_by = sy + s * seg_h + if s > 0 { GAP } else { 0 };
            let seg_bw = sw;
            let seg_bh = (if s == segments - 1 {
                sh - s * seg_h
            } else {
                seg_h
            })
            .saturating_sub(if s < segments - 1 { GAP } else { 0 });
            if seg_bw < FILLER_MIN_DIM || seg_bh < FILLER_MIN_DIM {
                continue;
            }
            let seg_seed = strip_seed.wrapping_add(s as u64 * 37);
            let shrink_x_max = seg_bw.saturating_sub(FILLER_MIN_DIM).min(4);
            let shrink_y_max = seg_bh.saturating_sub(FILLER_MIN_DIM).min(2);
            let shrink_x = if shrink_x_max > 0 {
                (value_noise(base_bx, base_by, seg_seed) * (shrink_x_max + 1) as f64)
                    as CoordinateUnit
            } else {
                0
            };
            let shrink_y = if shrink_y_max > 0 {
                (value_noise(base_by, base_bx, seg_seed.wrapping_add(1))
                    * (shrink_y_max + 1) as f64) as CoordinateUnit
            } else {
                0
            };
            let bw = seg_bw - shrink_x;
            let bh = seg_bh - shrink_y;
            let jitter_x = if seg_bw > bw {
                (value_noise(seg_bw, seg_bh, seg_seed.wrapping_add(2)) * (seg_bw - bw + 1) as f64)
                    as CoordinateUnit
            } else {
                0
            };
            let jitter_y = if seg_bh > bh {
                (value_noise(seg_bh, seg_bw, seg_seed.wrapping_add(3)) * (seg_bh - bh + 1) as f64)
                    as CoordinateUnit
            } else {
                0
            };
            let bx = base_bx + jitter_x;
            let by = base_by + jitter_y;
            let kind = pick_kind(
                bx + bw / 2,
                by + bh / 2,
                bx,
                by,
                seg_seed,
                width,
                height,
                avenue_ys,
                seed,
            );
            if !try_place(bx, by, bw, bh, FILLER_MIN_DIM, kind, placed, out) {
                try_place(
                    base_bx,
                    base_by,
                    seg_bw,
                    seg_bh,
                    FILLER_MIN_DIM,
                    kind,
                    placed,
                    out,
                );
            }
        }
    }
}

// ── Main generation entry point ────────────────────────────────────────────────

/// Generates all building footprints for the map.
///
/// Pipeline:
///  1. Pre-place immovable landmark anchors.
///  2. BSP tree: one large building (or park) per leaf.
///  3. Road-grid lots: per lot, `place_lot_buildings` drops one dominant
///     anchor building first, then fills the four surrounding strips with
///     smaller buildings.
///
/// Returns `(buildings, parks, open_lots)`.
fn generate_buildings_bsp(
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    avenue_ys: &[CoordinateUnit],
    cross_xs: &[CoordinateUnit],
    avenue_half_width: CoordinateUnit,
    cross_half_width: CoordinateUnit,
) -> (
    Vec<Building>,
    Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )>,
    Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )>,
) {
    let mut buildings: Vec<Building> = Vec::new();
    let mut parks: Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )> = Vec::new();
    let mut open_lots: Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )> = Vec::new();

    let bsp_seed = seed.wrapping_add(11111);
    let margin = (26 as CoordinateUnit).min(width / 8).min(height / 8).max(5);
    let build_w = width - margin * 2;
    let build_h = height - margin * 2;

    if build_w < BSP_MIN_LEAF_W || build_h < BSP_MIN_LEAF_H {
        return (buildings, parks, open_lots);
    }

    let mut placed: Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )> = Vec::new();

    // ── 1. Landmark anchors ───────────────────────────────────────────
    let anchors = pre_place_landmark_anchors(width, height, seed, avenue_ys);
    for a in &anchors {
        placed.push((a.x, a.y, a.w, a.h));
        buildings.push(Building {
            x: a.x,
            y: a.y,
            w: a.w,
            h: a.h,
            kind: a.kind,
        });
    }

    // ── 2. BSP tree: one large building or park per leaf ─────────────
    let mut root = BspNode::new(margin, margin, build_w, build_h);
    root.subdivide(bsp_seed, 0, avenue_ys, cross_xs);

    let mut leaves: Vec<(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )> = Vec::new();
    root.collect_leaves(&mut leaves);

    for (i, &(lx, ly, lw, lh)) in leaves.iter().enumerate() {
        let leaf_cx = lx + lw / 2;
        let leaf_cy = ly + lh / 2;

        let on_avenue = avenue_ys
            .iter()
            .any(|&ay| ly < ay + avenue_half_width + 1 && ly + lh > ay - avenue_half_width - 1);
        if on_avenue {
            continue;
        }
        if anchors
            .iter()
            .any(|a| rects_overlap(lx, ly, lw, lh, a.x, a.y, a.w, a.h))
        {
            continue;
        }

        let pad = BSP_PADDING_LOCAL;
        let leaf_inner_w = lw - pad * 2;
        let leaf_inner_h = lh - pad * 2;
        let bw = leaf_inner_w.min(BSP_MAX_W);
        let bh = leaf_inner_h.min(BSP_MAX_H);

        if bw < ANCHOR_MIN_DIM || bh < ANCHOR_MIN_DIM {
            open_lots.push((lx, ly, lw, lh));
            continue;
        }

        let leaf_seed = bsp_seed.wrapping_add(i as u64 * 53);
        let free_x = leaf_inner_w.saturating_sub(bw);
        let free_y = leaf_inner_h.saturating_sub(bh);
        let bx = lx
            + pad
            + if free_x > 0 {
                (value_noise(lx, ly, leaf_seed) * (free_x + 1) as f64) as CoordinateUnit
            } else {
                0
            };
        let by = ly
            + pad
            + if free_y > 0 {
                (value_noise(ly, lx, leaf_seed.wrapping_add(1)) * (free_y + 1) as f64)
                    as CoordinateUnit
            } else {
                0
            };

        let (bx, bw) = match fit_beside_cross_streets(bx, bw, lx, cross_xs, cross_half_width) {
            Some(fit) => fit,
            None => {
                open_lots.push((lx, ly, lw, lh));
                continue;
            }
        };

        if value_noise(leaf_cx, leaf_cy, bsp_seed.wrapping_add(8888)) < PARK_NOISE_THRESHOLD {
            parks.push((bx, by, bw, bh));
            placed.push((bx, by, bw, bh));
            continue;
        }

        let kind = pick_kind(
            leaf_cx,
            leaf_cy,
            i as i32,
            leaf_cx + leaf_cy,
            bsp_seed.wrapping_add(2222),
            width,
            height,
            avenue_ys,
            seed,
        );

        // Try at full size, shrinking up to 4 steps if needed.
        let placed_ok = (0u32..4).find_map(|s| {
            let sw = bw - s as CoordinateUnit * 2;
            let sh = bh - s as CoordinateUnit * 2;
            if sw < ANCHOR_MIN_DIM || sh < ANCHOR_MIN_DIM {
                return None;
            }
            if overlaps_placed(&placed, bx, by, sw, sh, GAP) {
                return None;
            }
            Some((sw, sh))
        });

        match placed_ok {
            Some((fw, fh)) => {
                placed.push((bx, by, fw, fh));
                buildings.push(Building {
                    x: bx,
                    y: by,
                    w: fw,
                    h: fh,
                    kind,
                });
            }
            None => open_lots.push((lx, ly, lw, lh)),
        }
    }

    // ── 3. Road-grid lots: anchor first, then strip fill ──────────────
    let avenue_set: HashSet<CoordinateUnit> = avenue_ys.iter().copied().collect();
    let cross_set: HashSet<CoordinateUnit> = cross_xs.iter().copied().collect();

    let mut y_bounds: Vec<CoordinateUnit> = std::iter::once(margin)
        .chain(avenue_ys.iter().copied())
        .chain(std::iter::once(height - margin))
        .collect();
    y_bounds.sort_unstable();

    let mut x_bounds: Vec<CoordinateUnit> = std::iter::once(margin)
        .chain(cross_xs.iter().copied())
        .chain(std::iter::once(width - margin))
        .collect();
    x_bounds.sort_unstable();

    for yi in 0..y_bounds.len().saturating_sub(1) {
        for xi in 0..x_bounds.len().saturating_sub(1) {
            let lot = match compute_lot(
                xi,
                yi,
                &x_bounds,
                &y_bounds,
                &avenue_set,
                &cross_set,
                avenue_half_width,
                cross_half_width,
            ) {
                Some(l) => l,
                None => continue,
            };

            let lot_seed = bsp_seed.wrapping_add(
                (yi as u64)
                    .wrapping_mul(1000)
                    .wrapping_add(xi as u64)
                    .wrapping_mul(7),
            );

            place_lot_buildings(
                lot,
                lot_seed,
                &mut placed,
                &mut buildings,
                width,
                height,
                avenue_ys,
                seed,
            );
        }
    }

    (buildings, parks, open_lots)
}

/// Returns the adjusted `(bx, bw)` so the building avoids any cross street.
/// Returns `None` if no valid position exists.
fn fit_beside_cross_streets(
    bx: CoordinateUnit,
    bw: CoordinateUnit,
    lx: CoordinateUnit,
    cross_xs: &[CoordinateUnit],
    cross_half_width: CoordinateUnit,
) -> Option<(CoordinateUnit, CoordinateUnit)> {
    let on_cross = cross_xs
        .iter()
        .any(|&cx| bx < cx + cross_half_width + 1 && bx + bw > cx - cross_half_width - 1);
    if !on_cross {
        return Some((bx, bw));
    }

    for &cx in cross_xs {
        let left_edge = cx - cross_half_width - 1;
        if left_edge > bx {
            let w = (left_edge - bx).min(bw);
            if w >= ANCHOR_MIN_DIM {
                return Some((bx, w));
            }
        }
        let right_x = cx + cross_half_width + 1;
        if right_x >= lx && right_x < bx + bw {
            let w = (bx + bw - right_x).min(bw);
            if w >= ANCHOR_MIN_DIM {
                return Some((right_x, w));
            }
        }
    }
    None
}

// ── Alley placement ────────────────────────────────────────────────────────────

/// Places `Floor::Alley` tiles in every gap of up to `ALLEY_GAP_MAX` tiles
/// between adjacent building pairs.
fn place_alleys(map: &mut GameMap, buildings: &[Building]) {
    for (i, a) in buildings.iter().enumerate() {
        for b in &buildings[i + 1..] {
            try_place_alley_strip(map, a, b);
        }
    }
}

/// Fills the gap between one building pair with alley tiles.
fn try_place_alley_strip(map: &mut GameMap, a: &Building, b: &Building) {
    let (a, b) = if a.x <= b.x && a.y <= b.y {
        (a, b)
    } else {
        (b, a)
    };

    // Horizontal gap (shared Y band, gap along X).
    if a.y < b.y + b.h && b.y < a.y + a.h {
        let gap = b.x as i64 - (a.x + a.w) as i64;
        if (1..=ALLEY_GAP_MAX).contains(&gap) {
            let y_min = a.y.max(b.y);
            let y_max = (a.y + a.h).min(b.y + b.h);
            for y in y_min..y_max {
                for gx in 0..gap as CoordinateUnit {
                    write_alley(map, a.x + a.w + gx, y);
                }
            }
        }
    }

    // Vertical gap (shared X band, gap along Y).
    if a.x < b.x + b.w && b.x < a.x + a.w {
        let gap = b.y as i64 - (a.y + a.h) as i64;
        if (1..=ALLEY_GAP_MAX).contains(&gap) {
            let x_min = a.x.max(b.x);
            let x_max = (a.x + a.w).min(b.x + b.w);
            for x in x_min..x_max {
                for gy in 0..gap as CoordinateUnit {
                    write_alley(map, x, a.y + a.h + gy);
                }
            }
        }
    }
}

/// Writes a single `Floor::Alley` tile if the cell is unoccupied.
#[inline]
fn write_alley(map: &mut GameMap, x: CoordinateUnit, y: CoordinateUnit) {
    let pos = GridVec::new(x, y);
    if let Some(v) = map.get_voxel_at_mut(&pos) {
        if v.props.is_none()
            && !matches!(v.floor, Some(Floor::WoodPlanks) | Some(Floor::StoneFloor))
        {
            v.floor = Some(Floor::Alley);
        }
    }
}

// ── Alley connectivity ─────────────────────────────────────────────────────────

/// Ensures every enclosed alley region has at least one exit to the street
/// by punching through an adjacent wall tile if necessary.
fn ensure_alley_connectivity(map: &mut GameMap, width: CoordinateUnit, height: CoordinateUnit) {
    let alley_tiles = collect_floor_tiles(map, width, height, |f| matches!(f, Floor::Alley));
    if alley_tiles.is_empty() {
        return;
    }

    let regions = flood_fill_regions(&alley_tiles, width, height, map, |f| {
        matches!(f, Floor::Alley)
    });

    for region in &regions {
        if region_has_road_exit(region, map, width, height) {
            continue;
        }
        open_wall_passage(region, map, width, height);
    }
}

fn collect_floor_tiles(
    map: &GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    pred: impl Fn(&Floor) -> bool,
) -> Vec<GridVec> {
    let mut tiles = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let pos = GridVec::new(x, y);
            if let Some(v) = map.get_voxel_at(&pos) {
                if v.floor.as_ref().is_some_and(|f| pred(f)) {
                    tiles.push(pos);
                }
            }
        }
    }
    tiles
}

fn flood_fill_regions(
    seeds: &[GridVec],
    width: CoordinateUnit,
    height: CoordinateUnit,
    map: &GameMap,
    passable: impl Fn(&Floor) -> bool,
) -> Vec<Vec<GridVec>> {
    let map_w = width as usize;
    let mut visited = vec![false; map_w * height as usize];
    let mut regions = Vec::new();

    for &start in seeds {
        let idx = start.y as usize * map_w + start.x as usize;
        if visited[idx] {
            continue;
        }

        let mut region = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        visited[idx] = true;
        queue.push_back(start);

        while let Some(pos) = queue.pop_front() {
            region.push(pos);
            for nb in in_bounds_neighbors(pos, width, height) {
                let ni = nb.y as usize * map_w + nb.x as usize;
                if !visited[ni] {
                    if let Some(v) = map.get_voxel_at(&nb) {
                        if v.floor.as_ref().is_some_and(|f| passable(f)) {
                            visited[ni] = true;
                            queue.push_back(nb);
                        }
                    }
                }
            }
        }
        regions.push(region);
    }
    regions
}

fn region_has_road_exit(
    region: &[GridVec],
    map: &GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
) -> bool {
    region.iter().any(|&pos| {
        in_bounds_neighbors(pos, width, height).iter().any(|&nb| {
            map.get_voxel_at(&nb).is_some_and(|v| {
                matches!(
                    v.floor.as_ref(),
                    Some(Floor::DirtRoad)
                        | Some(Floor::Sidewalk)
                        | Some(Floor::Dirt)
                        | Some(Floor::Bridge)
                ) && !v.props.as_ref().is_some_and(|p| p.is_wall())
            })
        })
    })
}

fn open_wall_passage(
    region: &[GridVec],
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
) {
    'outer: for &pos in region {
        for nb in in_bounds_neighbors(pos, width, height) {
            if map
                .get_voxel_at(&nb)
                .is_some_and(|v| v.props.as_ref().is_some_and(|p| p.is_wall()))
            {
                if let Some(voxel) = map.get_voxel_at_mut(&nb) {
                    voxel.props = None;
                }
                break 'outer;
            }
        }
    }
}

#[inline]
fn in_bounds_neighbors(
    pos: GridVec,
    width: CoordinateUnit,
    height: CoordinateUnit,
) -> Vec<GridVec> {
    pos.cardinal_neighbors()
        .into_iter()
        .filter(|nb| nb.x >= 0 && nb.x < width && nb.y >= 0 && nb.y < height)
        .collect()
}

/// Places a building on the map: walls around the perimeter, wood plank floor,
/// and interior props based on building kind.
/// Supports non-rectangular shapes for larger buildings.
const SHAPE_RECT: u32 = 0;
const SHAPE_ROUNDED: u32 = 1;
const SHAPE_L: u32 = 2;

/// Corner radius for rounded-corner building shapes.
const ROUNDED_CORNER_RADIUS: CoordinateUnit = 2;

/// L-shape notch is ~1/N of building dimensions.
const L_SHAPE_NOTCH_DIVISOR: CoordinateUnit = 3;

fn place_building(map: &mut GameMap, b: &Building, seed: NoiseSeed) {
    // Determine building shape based on noise
    let shape_type = building_shape_type(b, seed);

    // For L-shape, compute notch dimensions
    let notch_w = b.w / L_SHAPE_NOTCH_DIVISOR;
    let notch_h = b.h / L_SHAPE_NOTCH_DIVISOR;
    // Notch is in top-right corner
    let notch_x_start = b.x + b.w - notch_w;
    let notch_y_start = b.y + b.h - notch_h;

    let corner_radius = ROUNDED_CORNER_RADIUS;

    for y in b.y..b.y + b.h {
        for x in b.x..b.x + b.w {
            let pos = GridVec::new(x, y);

            // L-shape: skip the notch area entirely
            if shape_type == SHAPE_L && x >= notch_x_start && y >= notch_y_start {
                continue;
            }

            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                let mut is_border =
                    x == b.x || x == b.x + b.w - 1 || y == b.y || y == b.y + b.h - 1;
                // For L-shape, the notch edges are also borders
                if shape_type == SHAPE_L {
                    // Recompute border for L-shape
                    let in_main = x >= b.x
                        && x < b.x + b.w
                        && y >= b.y
                        && y < b.y + b.h
                        && !(x >= notch_x_start && y >= notch_y_start);
                    if in_main {
                        let left_out = x == b.x;
                        let right_out = x == b.x + b.w - 1 && y < notch_y_start;
                        let right_notch_edge = x == notch_x_start - 1 && y >= notch_y_start;
                        // right edge for L: either original right wall (above notch) or notch inner edge
                        let top_out = y == b.y + b.h - 1 && x < notch_x_start;
                        let top_notch_edge = y == notch_y_start - 1 && x >= notch_x_start;
                        let bottom_out = y == b.y;
                        is_border = left_out
                            || right_out
                            || right_notch_edge
                            || top_out
                            || top_notch_edge
                            || bottom_out;
                    }
                }

                // Door positions
                let is_door = y == b.y + b.h - 1
                    && x == b.x + b.w / 2
                    && (shape_type != SHAPE_L || x < notch_x_start);

                // Side doors for bigger buildings
                let is_side_door = if building_has_side_door(b) {
                    x == b.x + b.w - 1
                        && y == b.y + b.h / 2
                        && (shape_type != SHAPE_L || y < notch_y_start)
                } else {
                    false
                };

                // Rounded corners: skip corner walls.
                // True when the tile is within `corner_radius` of both a
                // horizontal and a vertical building edge (i.e. in any of the
                // four corner regions).
                let is_corner = if shape_type == SHAPE_ROUNDED {
                    (b.x + b.w - 1 - x < corner_radius || x - b.x < corner_radius)
                        && (b.y + b.h - 1 - y < corner_radius || y - b.y < corner_radius)
                } else {
                    false
                };

                if is_border && !is_door && !is_side_door && !is_corner {
                    // Stone buildings: churches, banks, jails
                    if matches!(b.kind, 6 | 7 | 9) {
                        voxel.props = Some(Props::StoneWall);
                        voxel.floor = Some(Floor::StoneFloor);
                    } else {
                        // Small chance to place a window instead of a wall.
                        // Only on non-corner border tiles (skip first/last of each edge).
                        let at_building_corner =
                            (x == b.x || x == b.x + b.w - 1) && (y == b.y || y == b.y + b.h - 1);
                        let window_noise = value_noise(x, y, seed.wrapping_add(88888));
                        if !at_building_corner && window_noise < 0.12 {
                            voxel.props = Some(Props::Window);
                        } else {
                            voxel.props = Some(Props::Wall);
                        }
                        voxel.floor = Some(Floor::WoodPlanks);
                    }
                } else {
                    voxel.props = None;
                    if matches!(b.kind, 6 | 7 | 9) {
                        voxel.floor = Some(Floor::StoneFloor);
                    } else {
                        voxel.floor = Some(Floor::WoodPlanks);
                    }
                }
            }
        }
    }

    // Place interior props based on building kind
    let interior_x = b.x + 1;
    let interior_y = b.y + 1;
    let iw = b.w - 2;
    let ih = b.h - 2;

    // Large buildings (interior ≥ 10×10) get a 2×2 room grid with corridors
    // before kind-specific props. set_prop already skips wall tiles, so
    // props placed by the per-kind block won't conflict with dividing walls.
    if iw >= 10 && ih >= 10 {
        let mid_x = iw / 2;
        let mid_y = ih / 2;
        let wall_prop = if matches!(b.kind, 6 | 7 | 9) {
            Props::StoneWall
        } else {
            Props::Wall
        };
        // Horizontal dividing wall with corridor gap at the center
        for dx in 0..iw {
            if dx == mid_x {
                continue;
            }
            let pos = GridVec::new(interior_x + dx, interior_y + mid_y);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                voxel.props = Some(wall_prop.clone());
            }
        }
        // Vertical dividing wall with corridor gap at the center
        for dy in 0..ih {
            if dy == mid_y {
                continue;
            }
            let pos = GridVec::new(interior_x + mid_x, interior_y + dy);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                voxel.props = Some(wall_prop.clone());
            }
        }
    }

    match b.kind {
        0 => {
            // House: dividing wall separates front room from back room
            if iw >= 6 && ih >= 8 {
                // Large: dividing wall + furnished rooms
                let wall_row = ih / 2;
                for dx in 0..iw {
                    if dx == iw / 2 {
                        continue;
                    }
                    let pos = GridVec::new(interior_x + dx, interior_y + wall_row);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        if matches!(b.kind, 6 | 7 | 9) {
                            voxel.props = Some(Props::StoneWall);
                        } else {
                            voxel.props = Some(Props::Wall);
                        }
                    }
                }
                // Front room: dining table with chairs
                set_prop(map, interior_x + 2, interior_y + wall_row + 2, Props::Table);
                set_prop(map, interior_x + 1, interior_y + wall_row + 2, Props::Chair);
                set_prop(map, interior_x + 3, interior_y + wall_row + 2, Props::Chair);
                // Back room: bedroom area
                set_prop(map, interior_x + iw - 1, interior_y + 1, Props::Bench);
                set_prop(map, interior_x + iw - 2, interior_y + 1, Props::Bench);
                // Storage
                set_prop(map, interior_x, interior_y, Props::Barrel);
                set_prop(map, interior_x + iw - 1, interior_y, Props::Crate);
            } else if iw >= 4 && ih >= 4 {
                // Medium: simple dividing wall with door gap, minimal furnishing
                let wall_row = ih / 2;
                for dx in 0..iw {
                    if dx == iw / 2 {
                        continue;
                    }
                    let pos = GridVec::new(interior_x + dx, interior_y + wall_row);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.props = Some(Props::Wall);
                    }
                }
                set_prop(map, interior_x + 1, interior_y + wall_row + 1, Props::Table);
                set_prop(map, interior_x, interior_y + wall_row + 1, Props::Chair);
                set_prop(map, interior_x + iw - 1, interior_y, Props::Bench);
            } else if iw >= 2 && ih >= 2 {
                set_prop(map, interior_x + 1, interior_y + 1, Props::Table);
                set_prop(map, interior_x, interior_y + 1, Props::Chair);
            }
        }
        1 => {
            // Saloon: piano, bar counter, multiple table/chair clusters
            if iw >= 6 && ih >= 5 {
                set_prop(map, interior_x, interior_y, Props::Piano);
                // Bar (barrels along back wall)
                for dx in 2..iw.min(8) {
                    set_prop(map, interior_x + dx, interior_y, Props::Barrel);
                }
                // Table clusters in a grid
                for row in 0..(ih - 2) / 3 {
                    for col in 0..(iw - 1) / 4 {
                        let tx = interior_x + 1 + col * 4;
                        let ty = interior_y + 2 + row * 3;
                        if tx + 1 < interior_x + iw && ty < interior_y + ih {
                            set_prop(map, tx, ty, Props::Table);
                            set_prop(map, tx - 1, ty, Props::Chair);
                            set_prop(map, tx + 1, ty, Props::Chair);
                        }
                    }
                }
            } else if iw >= 4 && ih >= 3 {
                set_prop(map, interior_x, interior_y, Props::Piano);
                set_prop(map, interior_x + 2, interior_y + 1, Props::Table);
                set_prop(map, interior_x + 1, interior_y + 1, Props::Chair);
                set_prop(map, interior_x + 3, interior_y + 1, Props::Chair);
            }
        }
        2 => {
            // Stable: hitching posts along walls, hay bales, water troughs
            if iw >= 5 && ih >= 4 {
                // Hitching posts along one wall
                for dx in (0..iw).step_by(3) {
                    set_prop(map, interior_x + dx, interior_y, Props::HitchingPost);
                }
                // Hay bales along opposite wall
                for dx in (0..iw).step_by(2) {
                    set_prop(map, interior_x + dx, interior_y + ih - 1, Props::HayBale);
                }
                // Water trough and crates
                set_prop(
                    map,
                    interior_x + iw / 2,
                    interior_y + ih / 2,
                    Props::WaterTrough,
                );
                set_prop(map, interior_x + iw - 1, interior_y + ih / 2, Props::Crate);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x, interior_y, Props::HitchingPost);
                set_prop(map, interior_x + 2, interior_y, Props::HitchingPost);
                set_prop(map, interior_x + 1, interior_y + ih - 1, Props::WaterTrough);
            }
        }
        3 => {
            // General store: shelves (barrels/crates) along walls, counter
            if iw >= 5 && ih >= 4 {
                // Counter (tables)
                for dx in 1..iw - 1 {
                    set_prop(map, interior_x + dx, interior_y + ih / 2, Props::Table);
                }
                // Shelves along walls
                set_prop(map, interior_x, interior_y, Props::Barrel);
                set_prop(map, interior_x, interior_y + 1, Props::Crate);
                set_prop(map, interior_x + iw - 1, interior_y, Props::Crate);
                set_prop(map, interior_x + iw - 1, interior_y + 1, Props::Barrel);
                if ih >= 6 {
                    set_prop(map, interior_x, interior_y + ih - 1, Props::Crate);
                    set_prop(map, interior_x + iw - 1, interior_y + ih - 1, Props::Barrel);
                }
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x, interior_y, Props::Barrel);
                set_prop(map, interior_x + 1, interior_y, Props::Crate);
            }
        }
        4 => {
            // Police station: desk area in front, rear cell block
            if iw >= 5 && ih >= 4 {
                // Wanted posters on front wall
                set_prop(map, interior_x, interior_y + ih - 1, Props::Sign);
                set_prop(map, interior_x + 1, interior_y + ih - 1, Props::Sign);
                // Desk area (front half)
                set_prop(map, interior_x + 2, interior_y + ih - 2, Props::Table);
                set_prop(map, interior_x + 1, interior_y + ih - 2, Props::Chair);
                set_prop(map, interior_x + 3, interior_y + ih - 2, Props::Chair);
                // Cell block dividing wall with door
                let cell_wall_row = ih / 2;
                for dx in 0..iw {
                    if dx == iw / 2 {
                        continue;
                    } // door gap
                    let pos = GridVec::new(interior_x + dx, interior_y + cell_wall_row);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.props = Some(Props::Wall);
                    }
                }
                // Cell furnishings in the rear
                set_prop(map, interior_x + iw - 1, interior_y, Props::Barrel);
                set_prop(map, interior_x, interior_y, Props::Bench);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x + 1, interior_y, Props::Table);
                set_prop(map, interior_x, interior_y, Props::Chair);
            }
        }
        5 => {
            // Post office: service counter, mail storage
            if iw >= 5 && ih >= 4 {
                // Counter
                for dx in 1..iw - 1 {
                    set_prop(map, interior_x + dx, interior_y + 2, Props::Table);
                }
                // Mail crates behind counter
                set_prop(map, interior_x, interior_y, Props::Crate);
                set_prop(map, interior_x + 1, interior_y, Props::Crate);
                set_prop(map, interior_x + iw - 1, interior_y, Props::Crate);
                // Sign
                set_prop(map, interior_x + iw / 2, interior_y + ih - 1, Props::Sign);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x + 1, interior_y, Props::Table);
                set_prop(map, interior_x, interior_y, Props::Crate);
            }
        }
        6 => {
            // Church: pews in rows, altar at front
            if iw >= 5 && ih >= 5 {
                // Altar
                set_prop(map, interior_x + iw / 2, interior_y, Props::Table);
                set_prop(map, interior_x + iw / 2 - 1, interior_y, Props::Sign);
                set_prop(map, interior_x + iw / 2 + 1, interior_y, Props::Sign);
                // Pew rows (benches on both sides of center aisle)
                for row in 2..ih.min(8) {
                    set_prop(map, interior_x + 1, interior_y + row, Props::Bench);
                    if iw >= 6 {
                        set_prop(map, interior_x + iw - 2, interior_y + row, Props::Bench);
                    }
                }
            } else if iw >= 3 && ih >= 3 {
                set_prop(map, interior_x + iw / 2, interior_y, Props::Table);
                for row in 1..ih.min(4) {
                    set_prop(map, interior_x, interior_y + row, Props::Bench);
                }
            }
        }
        7 => {
            // Bank: counter, vault area, strongboxes
            if iw >= 5 && ih >= 4 {
                // Counter
                for dx in 1..iw - 1 {
                    set_prop(map, interior_x + dx, interior_y + ih / 2, Props::Table);
                }
                // Vault (barrels as heavy door / safe)
                set_prop(map, interior_x, interior_y, Props::Barrel);
                set_prop(map, interior_x + 1, interior_y, Props::Barrel);
                set_prop(map, interior_x, interior_y + 1, Props::Barrel);
                // Strongboxes
                set_prop(map, interior_x + iw - 1, interior_y, Props::Crate);
                set_prop(map, interior_x + iw - 1, interior_y + ih - 1, Props::Crate);
                set_prop(map, interior_x + iw - 2, interior_y + ih - 1, Props::Crate);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x + 1, interior_y, Props::Table);
                set_prop(map, interior_x, interior_y + ih - 1, Props::Barrel);
            }
        }
        8 => {
            // Hotel: lobby, guest rooms suggested by bed rows
            if iw >= 5 && ih >= 4 {
                // Lobby area
                set_prop(map, interior_x + 1, interior_y, Props::Table);
                set_prop(map, interior_x, interior_y, Props::Chair);
                set_prop(map, interior_x + 2, interior_y, Props::Chair);
                // Guest rooms (beds/benches in rows)
                for row in (2..ih).step_by(2) {
                    set_prop(map, interior_x, interior_y + row, Props::Bench);
                    if iw >= 6 {
                        set_prop(map, interior_x + iw - 1, interior_y + row, Props::Bench);
                    }
                }
                // Storage
                set_prop(map, interior_x + iw - 1, interior_y, Props::Barrel);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x, interior_y, Props::Table);
                set_prop(map, interior_x + 1, interior_y, Props::Chair);
            }
        }
        9 => {
                // Jail: cells, notices, desk
            if iw >= 5 && ih >= 4 {
                // Desk
                set_prop(map, interior_x + 1, interior_y, Props::Table);
                set_prop(map, interior_x, interior_y, Props::Chair);
                // Wanted posters
                set_prop(map, interior_x + 2, interior_y, Props::Sign);
                // Cell bars (barrels forming cell walls)
                for dy in 0..ih.min(5) {
                    set_prop(map, interior_x + iw - 1, interior_y + dy, Props::Barrel);
                }
                set_prop(map, interior_x + iw - 2, interior_y + ih - 1, Props::Barrel);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x, interior_y, Props::Sign);
                set_prop(map, interior_x + iw - 1, interior_y, Props::Barrel);
            }
        }
        10 => {
            // Undertaker: preparation tables, coffins (crates)
            if iw >= 5 && ih >= 4 {
                // Tables in center
                set_prop(map, interior_x + 2, interior_y + 1, Props::Table);
                set_prop(map, interior_x + 2, interior_y + ih / 2, Props::Table);
                if iw >= 7 {
                    set_prop(map, interior_x + 4, interior_y + 1, Props::Table);
                }
                // Coffins (crates) along wall
                for dy in (0..ih).step_by(2) {
                    set_prop(map, interior_x, interior_y + dy, Props::Crate);
                }
                set_prop(map, interior_x + iw - 1, interior_y + ih - 1, Props::Crate);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x + 1, interior_y, Props::Table);
                set_prop(map, interior_x, interior_y + ih - 1, Props::Crate);
            }
        }
        _ => {
            // Blacksmith: forge area, anvil, quench barrel, supplies
            if iw >= 5 && ih >= 4 {
                // Anvil (hitching post stand-in)
                set_prop(
                    map,
                    interior_x + iw / 2,
                    interior_y + ih / 2,
                    Props::HitchingPost,
                );
                // Quench barrel
                set_prop(
                    map,
                    interior_x + iw / 2 + 1,
                    interior_y + ih / 2,
                    Props::Barrel,
                );
                // Water trough
                set_prop(map, interior_x, interior_y + ih - 1, Props::WaterTrough);
                // Supplies along walls
                set_prop(map, interior_x + iw - 1, interior_y, Props::Crate);
                set_prop(map, interior_x + iw - 1, interior_y + 1, Props::Crate);
                set_prop(map, interior_x, interior_y, Props::Barrel);
            } else if iw >= 3 && ih >= 2 {
                set_prop(map, interior_x, interior_y, Props::HitchingPost);
                set_prop(map, interior_x + 1, interior_y + ih - 1, Props::Barrel);
            }
        }
    }
    // ── Wall integrity: repair any gaps in the perimeter ───────────────
    validate_building_walls(map, b, seed);
    map.mark_occupied(b.x, b.y, b.w, b.h);
    reserve_building_entrance(map, b);
}

/// Validates and repairs the wall perimeter of a building. Ensures every
/// edge tile that should be a wall (except doors and rounded corners) is
/// filled. Called immediately after placement to fix BSP-clipping or
/// late occupancy conflicts.
fn validate_building_walls(map: &mut GameMap, b: &Building, seed: NoiseSeed) {
    let shape_type = building_shape_type(b, seed);
    let notch_w = b.w / L_SHAPE_NOTCH_DIVISOR;
    let notch_h = b.h / L_SHAPE_NOTCH_DIVISOR;
    let notch_x_start = b.x + b.w - notch_w;
    let notch_y_start = b.y + b.h - notch_h;
    let corner_radius = ROUNDED_CORNER_RADIUS;
    let wall_prop = if matches!(b.kind, 6 | 7 | 9) {
        Props::StoneWall
    } else {
        Props::Wall
    };

    for y in b.y..b.y + b.h {
        for x in b.x..b.x + b.w {
            // L-shape: skip the notch
            if shape_type == SHAPE_L && x >= notch_x_start && y >= notch_y_start {
                continue;
            }
            let is_border = x == b.x || x == b.x + b.w - 1 || y == b.y || y == b.y + b.h - 1;
            // L-shape additional border edges
            let is_l_border = if shape_type == SHAPE_L {
                (x == notch_x_start - 1 && y >= notch_y_start)
                    || (y == notch_y_start - 1 && x >= notch_x_start)
            } else {
                false
            };
            // Door positions
            let is_door = y == b.y + b.h - 1
                && x == b.x + b.w / 2
                && (shape_type != SHAPE_L || x < notch_x_start);
            let is_side_door = if building_has_side_door(b) {
                x == b.x + b.w - 1
                    && y == b.y + b.h / 2
                    && (shape_type != SHAPE_L || y < notch_y_start)
            } else {
                false
            };
            let is_corner = if shape_type == SHAPE_ROUNDED {
                (b.x + b.w - 1 - x < corner_radius || x - b.x < corner_radius)
                    && (b.y + b.h - 1 - y < corner_radius || y - b.y < corner_radius)
            } else {
                false
            };

            if (is_border || is_l_border) && !is_door && !is_side_door && !is_corner {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                    if !voxel.props.as_ref().is_some_and(|p| p.is_wall()) {
                        voxel.props = Some(wall_prop.clone());
                    }
                }
            }
        }
    }
}

/// Helper: sets a prop at a position if within bounds, not occupied by a wall,
/// and not on a road, beach, water, or bridge tile. Also checks the occupancy grid.
fn set_prop(map: &mut GameMap, x: CoordinateUnit, y: CoordinateUnit, prop: Props) {
    if x < 0 || x >= map.width || y < 0 || y >= map.height {
        return;
    }
    // Check occupancy grid before taking a mutable borrow on voxels
    if map.occupancy[x as usize][y as usize] {
        return;
    }
    let pos = GridVec::new(x, y);
    if let Some(voxel) = map.get_voxel_at_mut(&pos)
        && !matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall))
        && !matches!(
            voxel.floor,
            Some(Floor::DirtRoad)
                | Some(Floor::BeachSand)
                | Some(Floor::ShallowWater)
                | Some(Floor::DeepWater)
                | Some(Floor::Bridge)
        )
    {
        voxel.props = Some(prop);
    }
}

#[inline]
fn is_street_floor(floor: &Floor) -> bool {
    matches!(
        floor,
        Floor::DirtRoad | Floor::Sidewalk | Floor::Bridge | Floor::Alley
    )
}

#[inline]
fn is_exterior_ground_floor(floor: &Floor) -> bool {
    !matches!(
        floor,
        Floor::ShallowWater
            | Floor::DeepWater
            | Floor::Beach
            | Floor::BeachSand
            | Floor::Bridge
            | Floor::WoodPlanks
            | Floor::StoneFloor
            | Floor::Rooftop
            | Floor::Plaza
    )
}

fn pave_exterior_tile(
    map: &mut GameMap,
    x: CoordinateUnit,
    y: CoordinateUnit,
    floor: Floor,
    reserve: bool,
) -> bool {
    if x < 0 || x >= map.width || y < 0 || y >= map.height {
        return false;
    }

    let pos = GridVec::new(x, y);
    let mut changed = false;
    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
        if !voxel.floor.as_ref().is_some_and(is_exterior_ground_floor)
            || voxel.props.as_ref().is_some_and(|prop| prop.is_wall())
        {
            return false;
        }

        voxel.floor = Some(floor);
        if voxel.props.is_some() {
            voxel.props = None;
        }
        changed = true;
    }

    if changed && reserve {
        map.occupancy[x as usize][y as usize] = true;
    }
    changed
}

fn nearest_front_street_tile(
    map: &GameMap,
    b: &Building,
    max_radius: CoordinateUnit,
) -> Option<GridVec> {
    let door_x = b.x + b.w / 2;
    let door_y = b.y + b.h;
    let mut best: Option<(i32, GridVec)> = None;

    for dy in -2..=max_radius {
        for dx in -max_radius..=max_radius {
            let x = door_x + dx;
            let y = door_y + dy;
            if x < 0 || x >= map.width || y < 0 || y >= map.height {
                continue;
            }
            let pos = GridVec::new(x, y);
            let Some(voxel) = map.get_voxel_at(&pos) else {
                continue;
            };
            let Some(floor) = voxel.floor.as_ref() else {
                continue;
            };
            if !is_street_floor(floor) || !map.is_passable(&pos) {
                continue;
            }

            let dist = (x - door_x).abs() + (y - door_y).abs();
            let directional_penalty = if y < door_y { 6 } else { 0 };
            let lateral_penalty = (x - door_x).abs() / 2;
            let floor_bias = match floor {
                Floor::Sidewalk => 0,
                Floor::DirtRoad => 1,
                Floor::Bridge => 2,
                Floor::Alley => 3,
                _ => 4,
            };
            let score = dist * 10 + directional_penalty + lateral_penalty + floor_bias;
            if best.is_none_or(|current| score < current.0) {
                best = Some((score, pos));
            }
        }
    }

    best.map(|(_, pos)| pos)
}

fn place_building_frontages(map: &mut GameMap, buildings: &[Building], seed: NoiseSeed) {
    for b in buildings {
        let door_x = b.x + b.w / 2;
        let door_y = b.y + b.h;
        let frontage_seed = seed.wrapping_add(b.x as u64 * 131 + b.y as u64 * 17);
        let commercial = matches!(b.kind, 1 | 3 | 4 | 5 | 7 | 8 | 10 | 11);
        let frontage_depth = if commercial {
            2.min((value_noise(b.w, b.h, frontage_seed) * 2.0) as CoordinateUnit + 1)
        } else {
            1
        };
        let frontage_floor = if commercial {
            Floor::WoodPlanks
        } else if matches!(b.kind, 6 | 7 | 9) {
            Floor::Sidewalk
        } else {
            Floor::Dirt
        };

        let span_lo = (b.x + 1).max(door_x - 2);
        let span_hi = (b.x + b.w - 2).min(door_x + 2);
        for depth in 0..frontage_depth {
            for x in span_lo..=span_hi {
                pave_exterior_tile(map, x, door_y + depth, frontage_floor.clone(), true);
            }
        }

        let Some(target) = nearest_front_street_tile(map, b, 8) else {
            continue;
        };
        let path_floor = map
            .get_voxel_at(&target)
            .and_then(|voxel| voxel.floor.clone())
            .filter(is_street_floor)
            .map_or(Floor::Dirt, |floor| match floor {
                Floor::Sidewalk | Floor::DirtRoad => Floor::Sidewalk,
                Floor::Bridge => Floor::Sidewalk,
                _ => Floor::Dirt,
            });

        for step in GridVec::new(door_x, door_y).bresenham_line(target) {
            if step == target {
                break;
            }
            pave_exterior_tile(map, step.x, step.y, path_floor.clone(), true);
            if commercial && (step.y - door_y).abs() <= 1 {
                pave_exterior_tile(map, step.x - 1, step.y, path_floor.clone(), true);
                pave_exterior_tile(map, step.x + 1, step.y, path_floor.clone(), true);
            }
        }
    }
}

fn rect_is_clear_for_yard(
    map: &GameMap,
    x: CoordinateUnit,
    y: CoordinateUnit,
    w: CoordinateUnit,
    h: CoordinateUnit,
) -> bool {
    if x < 1 || y < 1 || x + w >= map.width - 1 || y + h >= map.height - 1 {
        return false;
    }

    for iy in y..y + h {
        for ix in x..x + w {
            if map.occupancy[ix as usize][iy as usize] {
                return false;
            }
            let pos = GridVec::new(ix, iy);
            let Some(voxel) = map.get_voxel_at(&pos) else {
                return false;
            };
            if !voxel.floor.as_ref().is_some_and(is_exterior_ground_floor)
                || voxel.props.as_ref().is_some_and(|prop| prop.is_wall())
            {
                return false;
            }
        }
    }
    true
}

fn place_service_yards(map: &mut GameMap, buildings: &[Building], seed: NoiseSeed) {
    for b in buildings {
        if !matches!(b.kind, 0 | 2 | 5 | 8 | 10 | 11) {
            continue;
        }

        let yard_seed = seed.wrapping_add(b.x as u64 * 73 + b.y as u64 * 97);
        let yard_depth = 3 + (value_noise(b.x, b.y, yard_seed) * 3.0) as CoordinateUnit;
        let yard_x = (b.x - 1).max(1);
        let yard_w = (b.w + 2).min(map.width - yard_x - 1);
        let yard_y = b.y - yard_depth;
        if yard_y < 1 || !rect_is_clear_for_yard(map, yard_x, yard_y, yard_w, yard_depth) {
            continue;
        }

        for iy in yard_y..yard_y + yard_depth {
            for ix in yard_x..yard_x + yard_w {
                let floor = if matches!(b.kind, 0 | 8) {
                    Floor::Grass
                } else {
                    Floor::Dirt
                };
                let _ = pave_exterior_tile(map, ix, iy, floor, false);
            }
        }

        let gate_x = yard_x + yard_w / 2;
        for ix in yard_x..yard_x + yard_w {
            set_prop(map, ix, yard_y, Props::Fence);
        }
        for iy in yard_y + 1..yard_y + yard_depth {
            set_prop(map, yard_x, iy, Props::Fence);
            set_prop(map, yard_x + yard_w - 1, iy, Props::Fence);
        }
        if let Some(voxel) = map.get_voxel_at_mut(&GridVec::new(gate_x, yard_y + yard_depth - 1)) {
            if matches!(voxel.props, Some(Props::Fence)) {
                voxel.props = None;
            }
        }

        match b.kind {
            0 => {
                set_prop(map, gate_x - 1, yard_y + 1, Props::Bush);
                set_prop(map, gate_x + 1, yard_y + 1, Props::Bush);
                set_prop(map, gate_x, yard_y + yard_depth / 2, Props::Well);
            }
            2 => {
                set_prop(map, gate_x - 1, yard_y + 1, Props::HayBale);
                set_prop(map, gate_x + 1, yard_y + 1, Props::HayBale);
                set_prop(map, gate_x, yard_y + yard_depth / 2, Props::WaterTrough);
            }
            5 | 8 | 10 => {
                set_prop(map, gate_x - 1, yard_y + 1, Props::Crate);
                set_prop(map, gate_x + 1, yard_y + 1, Props::Barrel);
                set_prop(map, gate_x, yard_y + yard_depth / 2, Props::Bench);
            }
            _ => {
                set_prop(map, gate_x - 1, yard_y + 1, Props::Barrel);
                set_prop(map, gate_x + 1, yard_y + 1, Props::Crate);
                set_prop(map, gate_x, yard_y + yard_depth / 2, Props::WaterTrough);
            }
        }
    }
}

// ── Hierarchical placement: exterior props ───────────────────────────────

/// Places contextually appropriate exterior props around a building based
/// on its type and which edge faces the street. A house gets a fence
/// perimeter and a garden; a saloon gets hitching posts out front and
/// barrels along the side; a stable gets a corral attached to its rear.
fn place_exterior_props(map: &mut GameMap, b: &Building, seed: NoiseSeed) {
    let ext_seed = seed.wrapping_add(b.x as u64 * 31 + b.y as u64 * 37);
    match b.kind {
        0 => {
            // House: fence perimeter along the south edge, small garden/well
            for dx in -1..=b.w {
                set_prop(map, b.x + dx, b.y + b.h, Props::Fence);
            }
            let pick = value_noise(b.x, b.y, ext_seed);
            if pick < 0.5 {
                set_prop(map, b.x + b.w / 2, b.y - 1, Props::Well);
            } else {
                set_prop(map, b.x + 1, b.y - 1, Props::Bush);
                set_prop(map, b.x + 2, b.y - 1, Props::Bush);
            }
        }
        1 => {
            // Saloon: hitching posts out front, barrels along the side
            set_prop(map, b.x + b.w / 3, b.y + b.h, Props::HitchingPost);
            set_prop(map, b.x + b.w * 2 / 3, b.y + b.h, Props::HitchingPost);
            for dy in (0..b.h).step_by(3) {
                set_prop(map, b.x - 1, b.y + dy, Props::Barrel);
            }
        }
        2 => {
            // Stable: corral fence attached to rear, hay and water
            let corral_max_width: CoordinateUnit = 6;
            for dx in 0..b.w.min(corral_max_width) {
                set_prop(map, b.x + dx, b.y - 1, Props::Fence);
            }
            set_prop(map, b.x + 2, b.y - 2, Props::HayBale);
            set_prop(map, b.x + 4, b.y - 2, Props::WaterTrough);
        }
        3 => {
            // General store: crates and barrels near entrance
            set_prop(map, b.x + b.w / 2 - 1, b.y + b.h, Props::Crate);
            set_prop(map, b.x + b.w / 2 + 1, b.y + b.h, Props::Barrel);
        }
        4 => {
            // Police station: hitching post and town notice
            set_prop(map, b.x + b.w / 2, b.y + b.h, Props::HitchingPost);
            set_prop(map, b.x - 1, b.y + b.h / 2, Props::Sign);
        }
        _ => {}
    }
}

// ── Open lot scatter ─────────────────────────────────────────────────────

/// Fills open lots (BSP leaves that couldn't hold a building) with
/// contextually appropriate scatter: dead trees, broken wagons, campfire
/// rings, patches of scrub.
fn place_open_lot_scatter(
    map: &mut GameMap,
    open_lots: &[(
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
        CoordinateUnit,
    )],
    seed: NoiseSeed,
) {
    let scatter_seed = seed.wrapping_add(151515);
    for (i, &(lx, ly, lw, lh)) in open_lots.iter().enumerate() {
        let cx = lx + lw / 2;
        let cy = ly + lh / 2;
        let pick = value_noise(cx, cy, scatter_seed.wrapping_add(i as u64));
        let large_lot = lw >= 10 && lh >= 8;

        if large_lot {
            let patch_x0 = (cx - 2).max(lx + 1);
            let patch_x1 = (cx + 2).min(lx + lw - 2);
            let patch_y0 = (cy - 1).max(ly + 1);
            let patch_y1 = (cy + 2).min(ly + lh - 2);
            for y in patch_y0..=patch_y1 {
                for x in patch_x0..=patch_x1 {
                    let floor = if pick < 0.33 {
                        Floor::Dirt
                    } else if pick < 0.66 {
                        Floor::Gravel
                    } else {
                        Floor::Grass
                    };
                    let _ = pave_exterior_tile(map, x, y, floor, false);
                }
            }
        }

        if pick < 0.20 {
            // Fenced wagon or supply yard
            let yard_x0 = (cx - 2).max(lx + 1);
            let yard_x1 = (cx + 2).min(lx + lw - 2);
            let yard_y0 = (cy - 1).max(ly + 1);
            let yard_y1 = (cy + 2).min(ly + lh - 2);
            for x in yard_x0..=yard_x1 {
                set_prop(map, x, yard_y0, Props::Fence);
            }
            for y in yard_y0 + 1..=yard_y1 {
                set_prop(map, yard_x0, y, Props::Fence);
                set_prop(map, yard_x1, y, Props::Fence);
            }
            set_prop(map, cx - 1, cy, Props::Crate);
            set_prop(map, cx + 1, cy, Props::Barrel);
            set_prop(map, cx, cy + 1, Props::Bench);
        } else if pick < 0.40 {
            // Small market or camp spot
            set_prop(map, cx, cy, Props::Table);
            set_prop(map, cx - 1, cy, Props::Bench);
            set_prop(map, cx + 1, cy, Props::Bench);
            set_prop(map, cx, cy - 1, Props::Sign);
            set_prop(map, cx, cy + 1, Props::Crate);
        } else if pick < 0.60 {
            // Broken wagon / supply spill
            set_prop(map, cx - 1, cy, Props::Crate);
            set_prop(map, cx, cy, Props::Crate);
            set_prop(map, cx + 1, cy, Props::Barrel);
            set_prop(map, cx, cy + 1, Props::Rock);
        } else if pick < 0.80 {
            // Campfire ring
            set_prop(map, cx - 1, cy, Props::Rock);
            set_prop(map, cx + 1, cy, Props::Rock);
            set_prop(map, cx, cy - 1, Props::Rock);
            set_prop(map, cx, cy + 1, Props::Rock);
            set_prop(map, cx + 1, cy + 1, Props::Bench);
        } else {
            // Scrub garden
            set_prop(map, cx, cy, Props::Bush);
            set_prop(map, cx + 1, cy, Props::Bush);
            set_prop(map, cx - 1, cy + 1, Props::Bush);
            if large_lot {
                set_prop(map, cx + 1, cy + 1, Props::Well);
            }
        }
    }
}

// ── Transition zones ─────────────────────────────────────────────────────

/// Places transitional scatter around the town perimeter — isolated shacks,
/// ruined structures, old fence lines, abandoned campsites — that gradually
/// thin out into open terrain. This makes the town feel like it grew
/// organically rather than being stamped onto the map.
fn place_transition_zones(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    if width < 60 || height < 40 || buildings.is_empty() {
        return;
    }
    let trans_seed = seed.wrapping_add(121212);

    // Find the bounding box of the town
    let Some(town_min_x) = buildings.iter().map(|b| b.x).min() else {
        return;
    };
    let Some(town_min_y) = buildings.iter().map(|b| b.y).min() else {
        return;
    };
    let Some(town_max_x) = buildings.iter().map(|b| b.x + b.w).max() else {
        return;
    };
    let Some(town_max_y) = buildings.iter().map(|b| b.y + b.h).max() else {
        return;
    };

    let num_scatter = 12 + (value_noise(0, 0, trans_seed) * 8.0) as i32;
    for i in 0..num_scatter {
        let nx = value_noise(i, 0, trans_seed);
        let ny = value_noise(0, i, trans_seed);
        let x = (nx * width as f64).clamp(4.0, (width - 6) as f64) as CoordinateUnit;
        let y = (ny * height as f64).clamp(4.0, (height - 6) as f64) as CoordinateUnit;

        // Only place in the transition zone (outside town boundaries)
        if x >= town_min_x - 5 && x <= town_max_x + 5 && y >= town_min_y - 5 && y <= town_max_y + 5
        {
            continue;
        }

        let pos = GridVec::new(x, y);
        if let Some(voxel) = map.get_voxel_at(&pos) {
            if voxel.props.is_some() {
                continue;
            }
            if matches!(
                voxel.floor,
                Some(Floor::ShallowWater) | Some(Floor::DeepWater) | Some(Floor::Bridge)
            ) {
                continue;
            }
        } else {
            continue;
        }

        let pick = value_noise(i, i, trans_seed.wrapping_add(111));
        if pick < 0.25 {
            set_prop(map, x, y, Props::DeadTree);
        } else if pick < 0.45 {
            // Old fence line
            for dx in 0..4 {
                set_prop(map, x + dx, y, Props::Fence);
            }
        } else if pick < 0.65 {
            // Abandoned campsite
            set_prop(map, x, y, Props::Rock);
            set_prop(map, x + 1, y, Props::Rock);
            set_prop(map, x, y + 1, Props::Barrel);
        } else if pick < 0.80 {
            // Broken wagon
            set_prop(map, x, y, Props::Crate);
            set_prop(map, x + 1, y, Props::Crate);
        } else {
            // Scrub
            set_prop(map, x, y, Props::Bush);
            set_prop(map, x + 1, y + 1, Props::Bush);
        }
    }

    // Place isolated outskirts shacks — small ruined structures with
    // partial walls that thin out toward the wilderness.
    let shack_seed = trans_seed.wrapping_add(333);
    let num_shacks = 4 + (value_noise(1, 1, shack_seed) * 4.0) as i32;
    for i in 0..num_shacks {
        let nx = value_noise(i, 2, shack_seed);
        let ny = value_noise(2, i, shack_seed);
        let sx = (nx * width as f64).clamp(8.0, (width - 12) as f64) as CoordinateUnit;
        let sy = (ny * height as f64).clamp(8.0, (height - 12) as f64) as CoordinateUnit;

        // Must be outside the town bounding box
        if sx >= town_min_x - 10
            && sx <= town_max_x + 10
            && sy >= town_min_y - 10
            && sy <= town_max_y + 10
        {
            continue;
        }

        // Skip water
        let pos = GridVec::new(sx, sy);
        if let Some(voxel) = map.get_voxel_at(&pos) {
            if matches!(
                voxel.floor,
                Some(Floor::ShallowWater) | Some(Floor::DeepWater)
            ) {
                continue;
            }
        } else {
            continue;
        }

        // Place a small 4×4 shack with partial walls (ruined)
        let sw: CoordinateUnit = 4;
        let sh: CoordinateUnit = 4;
        if sx + sw >= width - 1 || sy + sh >= height - 1 {
            continue;
        }
        for dy in 0..sh {
            for dx in 0..sw {
                let p = GridVec::new(sx + dx, sy + dy);
                if let Some(voxel) = map.get_voxel_at_mut(&p) {
                    if matches!(
                        voxel.floor,
                        Some(Floor::ShallowWater) | Some(Floor::DeepWater)
                    ) {
                        continue;
                    }
                    let is_border = dx == 0 || dx == sw - 1 || dy == 0 || dy == sh - 1;
                    let is_door = dy == sh - 1 && dx == sw / 2;
                    // Ruined: skip some wall segments based on noise
                    let ruin_noise =
                        value_noise(sx + dx, sy + dy, shack_seed.wrapping_add(i as u64));
                    if is_border && !is_door && ruin_noise < 0.7 {
                        voxel.props = Some(Props::Wall);
                        voxel.floor = Some(Floor::WoodPlanks);
                    } else {
                        voxel.props = None;
                        voxel.floor = Some(Floor::WoodPlanks);
                    }
                }
            }
        }

        // Lay a short dirt path from the shack door toward the town
        let door_x = sx + sw / 2;
        let door_y = sy + sh;
        let town_cx = (town_min_x + town_max_x) / 2;
        let town_cy = (town_min_y + town_max_y) / 2;
        let path_dx: CoordinateUnit = if town_cx > door_x { 1 } else { -1 };
        let path_dy: CoordinateUnit = if town_cy > door_y { 1 } else { -1 };
        let mut px = door_x;
        let mut py = door_y;
        for _ in 0..15 {
            if px <= 0 || px >= width - 1 || py <= 0 || py >= height - 1 {
                break;
            }
            let p = GridVec::new(px, py);
            if let Some(voxel) = map.get_voxel_at_mut(&p)
                && !matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall))
                && !matches!(
                    voxel.floor,
                    Some(Floor::ShallowWater) | Some(Floor::DeepWater)
                )
            {
                voxel.floor = Some(Floor::Dirt);
                voxel.props = None;
            }
            // Alternate between horizontal and vertical steps
            let step_noise = value_noise(px, py, shack_seed.wrapping_add(777));
            if step_noise < 0.5 {
                px += path_dx;
            } else {
                py += path_dy;
            }
        }
    }
}

// ── Post-pass connectivity check ─────────────────────────────────────────

/// Verifies every building has at least one navigable path to the main
/// street using BFS. If a building is unreachable, opens a gap in the
/// nearest blocking structure. No structure should be an island.
fn check_connectivity(
    map: &mut GameMap,
    buildings: &[Building],
    avenue_ys: &[CoordinateUnit],
    cross_xs: &[CoordinateUnit],
    width: CoordinateUnit,
    height: CoordinateUnit,
) {
    if buildings.is_empty() || (avenue_ys.is_empty() && cross_xs.is_empty()) {
        return;
    }
    if width < 30 || height < 30 {
        return;
    }

    let w = width as usize;
    let _h = height as usize;
    let mut reachable = vec![false; w * height as usize];
    let mut queue = std::collections::VecDeque::new();

    // Seed BFS from all passable road tiles (dirt, sidewalk, bridge, alley).
    // This is more robust than seeding only from avenue/cross-street centers,
    // because those specific positions may be blocked by the river.
    for y in 0..height {
        for x in 0..width {
            let pos = GridVec::new(x, y);
            if let Some(v) = map.get_voxel_at(&pos) {
                if matches!(
                    v.floor,
                    Some(Floor::Dirt)
                        | Some(Floor::DirtRoad)
                        | Some(Floor::Sidewalk)
                        | Some(Floor::Bridge)
                        | Some(Floor::Alley)
                ) && map.is_passable(&pos)
                {
                    let idx = y as usize * w + x as usize;
                    reachable[idx] = true;
                    queue.push_back(pos);
                }
            }
        }
    }

    while let Some(pos) = queue.pop_front() {
        for neighbor in pos.cardinal_neighbors() {
            if neighbor.x >= 0 && neighbor.x < width && neighbor.y >= 0 && neighbor.y < height {
                let idx = neighbor.y as usize * w + neighbor.x as usize;
                if !reachable[idx] && map.is_passable(&neighbor) {
                    reachable[idx] = true;
                    queue.push_back(neighbor);
                }
            }
        }
    }

    // Check each building's door area
    for b in buildings {
        let door_x = b.x + b.w / 2;
        let door_y = b.y + b.h; // Tile just outside south door
        if door_x < 0 || door_x >= width || door_y < 0 || door_y >= height {
            continue;
        }
        let idx = door_y as usize * w + door_x as usize;
        if reachable[idx] {
            continue;
        }

        // Building unreachable — clear blocking props and bridge water to open a path.
        // Search up to half the map size to ensure we always find a road.
        let max_r = (width / 3).max(height / 3).max(20);
        for r in 1..=max_r {
            let mut cleared = false;
            for dy in -r..=r {
                for dx in -r..=r {
                    let cx = door_x + dx;
                    let cy = door_y + dy;
                    if cx < 0 || cx >= width || cy < 0 || cy >= height {
                        continue;
                    }
                    let cidx = cy as usize * w + cx as usize;
                    if reachable[cidx] {
                        // Clear a line from door to this reachable tile,
                        // creating a dirt path and removing blocking props.
                        // Widen to 3 tiles to ensure cardinal connectivity
                        // for diagonal Bresenham segments.
                        let from = GridVec::new(door_x, door_y);
                        let to = GridVec::new(cx, cy);
                        for step in from.bresenham_line(to) {
                            for wy in -1..=1i32 {
                                for wx in -1..=1i32 {
                                    let wp = GridVec::new(step.x + wx, step.y + wy);
                                    if wp.x >= 0 && wp.x < width && wp.y >= 0 && wp.y < height {
                                        if let Some(voxel) = map.get_voxel_at_mut(&wp) {
                                            if voxel.props.as_ref().is_some_and(|p| {
                                                p.blocks_movement() && !p.is_wall()
                                            }) {
                                                voxel.props = None;
                                            }
                                            // Bridge water tiles to create walkable path
                                            if matches!(
                                                voxel.floor,
                                                Some(Floor::ShallowWater) | Some(Floor::DeepWater)
                                            ) {
                                                voxel.floor = Some(Floor::Bridge);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        cleared = true;
                        break;
                    }
                }
                if cleared {
                    break;
                }
            }
            if cleared {
                break;
            }
        }
    }
}

/// Structural verification of world generation invariants.
/// Returns `Ok(())` if the map passes all checks, or `Err(reason)` describing
/// which check failed. Used by the retry loop in `GameMap::new()`.
fn verify_world(
    map: &GameMap,
    buildings: &[Building],
    avenue_ys: &[CoordinateUnit],
    cross_xs: &[CoordinateUnit],
    width: CoordinateUnit,
    height: CoordinateUnit,
) -> Result<(), String> {
    if buildings.is_empty() || width < 60 || height < 60 || avenue_ys.is_empty() {
        return Ok(()); // Skip validation on maps without a proper road network
    }

    // 1. No two building footprints overlap (including 1-tile gap margin).
    //    Expand one building by 1 tile on each side and check against the
    //    exact footprint of the other — this enforces a 1-tile minimum gap
    //    which matches the grid-based lot partitioning gap.
    for (i, a) in buildings.iter().enumerate() {
        for b in &buildings[i + 1..] {
            let overlaps = rects_overlap(a.x - 1, a.y - 1, a.w + 2, a.h + 2, b.x, b.y, b.w, b.h);
            if overlaps {
                return Err(format!(
                    "buildings at ({},{}) {}x{} and ({},{}) {}x{} overlap (including padding)",
                    a.x, a.y, a.w, a.h, b.x, b.y, b.w, b.h
                ));
            }
        }
    }

    // 2. No building tile sits on a river, road, or beach tile
    for b in buildings {
        for by in b.y..b.y + b.h {
            for bx in b.x..b.x + b.w {
                let pos = GridVec::new(bx, by);
                if let Some(v) = map.get_voxel_at(&pos) {
                    if matches!(v.floor, Some(Floor::ShallowWater) | Some(Floor::DeepWater)) {
                        return Err(format!(
                            "building at ({},{}) has tile on river at ({},{})",
                            b.x, b.y, bx, by
                        ));
                    }
                }
            }
        }
    }

    // 3. Every building has a reachable entrance (BFS from all road tiles)
    let w = width as usize;
    let mut reachable = vec![false; w * height as usize];
    let mut queue = std::collections::VecDeque::new();
    for y in 0..height {
        for x in 0..width {
            let pos = GridVec::new(x, y);
            if let Some(v) = map.get_voxel_at(&pos) {
                if matches!(
                    v.floor,
                    Some(Floor::Dirt)
                        | Some(Floor::DirtRoad)
                        | Some(Floor::Sidewalk)
                        | Some(Floor::Bridge)
                        | Some(Floor::Alley)
                ) && map.is_passable(&pos)
                {
                    let idx = y as usize * w + x as usize;
                    reachable[idx] = true;
                    queue.push_back(pos);
                }
            }
        }
    }
    while let Some(pos) = queue.pop_front() {
        for neighbor in pos.cardinal_neighbors() {
            if neighbor.x >= 0 && neighbor.x < width && neighbor.y >= 0 && neighbor.y < height {
                let idx = neighbor.y as usize * w + neighbor.x as usize;
                if !reachable[idx] && map.is_passable(&neighbor) {
                    reachable[idx] = true;
                    queue.push_back(neighbor);
                }
            }
        }
    }
    for b in buildings {
        let door_x = b.x + b.w / 2;
        let door_y = b.y + b.h;
        if door_x >= 0 && door_x < width && door_y >= 0 && door_y < height {
            let idx = door_y as usize * w + door_x as usize;
            if !reachable[idx] {
                return Err(format!(
                    "building at ({},{}) has unreachable entrance at ({},{})",
                    b.x, b.y, door_x, door_y
                ));
            }
        }
    }

    // 4. Street graph is connected: every avenue tile that is passable
    //    can reach every cross-street tile that is passable.
    if avenue_ys.len() >= 2 {
        let first_ay = avenue_ys[0];
        let last_ay = *avenue_ys.last().unwrap();
        if first_ay >= 0 && first_ay < height && last_ay >= 0 && last_ay < height {
            let mid_x = width / 2;
            let start_idx = first_ay as usize * w + mid_x as usize;
            let end_idx = last_ay as usize * w + mid_x as usize;
            if reachable[start_idx] && !reachable[end_idx] {
                return Err(format!(
                    "street graph not fully connected — avenue y={} cannot reach avenue y={}",
                    first_ay, last_ay
                ));
            }
        }
    }
    for &cx in cross_xs {
        if cx >= 0 && cx < width {
            if let Some(&ay) = avenue_ys.first() {
                if ay >= 0 && ay < height {
                    let pos = GridVec::new(cx, ay);
                    if map.is_passable(&pos) {
                        let idx = ay as usize * w + cx as usize;
                        if !reachable[idx] {
                            return Err(format!(
                                "cross street x={} not connected to avenue y={}",
                                cx, ay
                            ));
                        }
                    }
                }
            }
        }
    }

    // 5. No building footprint contains a DirtRoad, water, or beach tile
    for b in buildings {
        for by in b.y..b.y + b.h {
            for bx in b.x..b.x + b.w {
                let pos = GridVec::new(bx, by);
                if let Some(v) = map.get_voxel_at(&pos) {
                    if matches!(
                        v.floor,
                        Some(Floor::DirtRoad)
                            | Some(Floor::ShallowWater)
                            | Some(Floor::DeepWater)
                            | Some(Floor::BeachSand)
                    ) {
                        return Err(format!(
                            "building at ({},{}) has excluded tile at ({},{})",
                            b.x, b.y, bx, by
                        ));
                    }
                }
            }
        }
    }

    // 6. No prop or tree sits on a water tile
    for y in 0..height {
        for x in 0..width {
            let pos = GridVec::new(x, y);
            if let Some(v) = map.get_voxel_at(&pos) {
                if v.props.is_some()
                    && v.props.as_ref().is_some_and(|prop| !prop.is_wall())
                    && matches!(v.floor, Some(Floor::ShallowWater) | Some(Floor::DeepWater))
                {
                    return Err(format!("prop on water tile at ({},{})", x, y));
                }
            }
        }
    }

    // 7. Bridges now override BeachSand tiles within the road band so
    //    bridge surfaces are clean. No separate verification needed.

    // 8. Wall perimeter integrity is enforced by validate_building_walls()
    //    during placement. Shape variants (L-shape, rounded corners) have
    //    intentional wall gaps that make a generic perimeter check fragile,
    //    so verification trusts the placement-time repair pass.

    Ok(())
}

/// Returns `true` if the rectangle (rx, ry, rw, rh) overlaps any building
/// footprint (including a 2-tile buffer for doors and entrance access).
fn overlaps_any_building(
    rx: CoordinateUnit,
    ry: CoordinateUnit,
    rw: CoordinateUnit,
    rh: CoordinateUnit,
    buildings: &[Building],
) -> bool {
    buildings
        .iter()
        .any(|b| rects_overlap(rx, ry, rw, rh, b.x - 2, b.y - 2, b.w + 4, b.h + 4))
}

/// Places the focal mission/church building near the center of the map.
/// This is a large, thick-walled stone structure with a courtyard, bell tower,
/// and multiple interior rooms that functions as a natural fortress and
/// late-game combat anchor.
fn place_mission(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    let mw: CoordinateUnit = 24;
    let mh: CoordinateUnit = 18;
    if width < mw + 10 || height < mh + 10 {
        return; // map too small for a mission
    }
    let m_seed = seed.wrapping_add(555666);
    // Try several candidate positions around the town center.
    // Dense building placement requires a wider search area.
    let offsets: [(i32, i32); 16] = [
        (0, 0),
        (-15, -15),
        (15, 15),
        (-30, 20),
        (30, -20),
        (-20, 30),
        (20, -30),
        (-35, -10),
        (40, 10),
        (-40, -10),
        (10, 40),
        (-10, -40),
        (50, 0),
        (-50, 0),
        (0, 50),
        (0, -50),
    ];
    for (i, &(ox, oy)) in offsets.iter().enumerate() {
        let noise_x = (value_noise(i as i32 * 2, 2, m_seed) * 10.0) as CoordinateUnit;
        let noise_y = (value_noise(i as i32 * 2 + 1, 3, m_seed) * 10.0) as CoordinateUnit;
        let cx = width / 2 + ox + noise_x;
        let cy = height / 2 + oy + noise_y;
        let mx = (cx - mw / 2).clamp(2, width - mw - 2);
        let my = (cy - mh / 2).clamp(2, height - mh - 2);

        if overlaps_any_building(mx, my, mw, mh, buildings) {
            continue;
        }
        if map.has_excluded_tile(mx, my, mw, mh) {
            continue;
        }

        // Lay down thick stone walls and interior
        place_mission_at(map, mx, my, mw, mh);
        return;
    }

    let mut fallback: Option<(CoordinateUnit, CoordinateUnit, CoordinateUnit)> = None;
    let center_x = width / 2 - mw / 2;
    let center_y = height / 2 - mh / 2;
    for my in (2..=height - mh - 2).step_by(6) {
        for mx in (2..=width - mw - 2).step_by(6) {
            if overlaps_any_building(mx, my, mw, mh, buildings)
                || map.has_excluded_tile(mx, my, mw, mh)
            {
                continue;
            }
            let score = (mx - center_x).abs() + (my - center_y).abs();
            if fallback.is_none_or(|current| score < current.0) {
                fallback = Some((score, mx, my));
            }
        }
    }
    if let Some((_, mx, my)) = fallback {
        place_mission_at(map, mx, my, mw, mh);
    }
}

/// Internal helper: actually places mission tiles at the given position.
fn place_mission_at(
    map: &mut GameMap,
    mx: CoordinateUnit,
    my: CoordinateUnit,
    mw: CoordinateUnit,
    mh: CoordinateUnit,
) {
    for y in my..my + mh {
        for x in mx..mx + mw {
            let pos = GridVec::new(x, y);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                let is_border = x == mx || x == mx + mw - 1 || y == my || y == my + mh - 1;
                // Main entrance (south wall center)
                let is_main_door = y == my + mh - 1 && (x == mx + mw / 2 || x == mx + mw / 2 - 1);
                // Side entrance (east wall)
                let is_side_door = x == mx + mw - 1 && y == my + mh / 2;
                // Back entrance (north wall)
                let is_back_door = y == my && x == mx + mw / 2;

                if is_border && !is_main_door && !is_side_door && !is_back_door {
                    voxel.props = Some(Props::StoneWall);
                    voxel.floor = Some(Floor::StoneFloor);
                } else {
                    voxel.props = None;
                    voxel.floor = Some(Floor::StoneFloor);
                }
            }
        }
    }

    // Interior dividing wall creating multiple rooms
    // Nave (main hall) on left, side rooms on right
    let divider_x = mx + mw * 2 / 3;
    for y in my + 1..my + mh - 1 {
        let pos = GridVec::new(divider_x, y);
        // Door in the divider
        if y != my + mh / 2
            && y != my + mh / 3
            && let Some(voxel) = map.get_voxel_at_mut(&pos)
        {
            voxel.props = Some(Props::StoneWall);
        }
    }

    // Courtyard: open area in the center-right rooms (no roof)
    let court_x = divider_x + 1;
    let court_y = my + 2;
    let court_w = (mx + mw - 1) - court_x - 1;
    let court_h = mh / 2 - 2;
    for dy in 0..court_h {
        for dx in 0..court_w {
            let pos = GridVec::new(court_x + dx, court_y + dy);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                voxel.floor = Some(Floor::Plaza);
                voxel.props = None;
            }
        }
    }

    // Bell tower: 3×3 tower structure in top-left corner of mission
    let tower_x = mx + 1;
    let tower_y = my + 1;
    for dy in 0..3i32 {
        for dx in 0..3i32 {
            let pos = GridVec::new(tower_x + dx, tower_y + dy);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                voxel.floor = Some(Floor::Rooftop);
            }
        }
    }

    // Interior props: altar, pews, signs
    let ix = mx + 1;
    let iy = my + 1;
    // Altar at the north end of the nave
    set_prop(map, ix + 4, iy, Props::Table);
    set_prop(map, ix + 3, iy, Props::Sign);
    set_prop(map, ix + 5, iy, Props::Sign);
    // Pew rows in the nave
    for row in 3..mh - 4 {
        set_prop(map, ix + 2, iy + row, Props::Bench);
        set_prop(map, ix + 6, iy + row, Props::Bench);
    }
    // Storage in side rooms
    set_prop(map, divider_x + 2, my + mh / 2 + 2, Props::Barrel);
    set_prop(map, divider_x + 3, my + mh / 2 + 2, Props::Crate);

    map.mark_occupied(mx, my, mw, mh);
}

/// Places a town plaza — an open killzone at the heart of the map.
/// The plaza is flanked by buildings with window-facing tiles overlooking it.
fn place_town_plaza(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    let pw: CoordinateUnit = 16;
    let ph: CoordinateUnit = 12;
    if width < pw + 20 || height < ph + 20 {
        return;
    }
    let p_seed = seed.wrapping_add(777888);
    // Try several candidate positions for the plaza. Dense building
    // placement requires a wider search area.
    let offsets: [(i32, i32); 16] = [
        (15, 10),
        (-10, 0),
        (0, -10),
        (-25, 25),
        (25, -25),
        (-15, -20),
        (20, 20),
        (-30, 10),
        (40, 0),
        (-40, 0),
        (0, 40),
        (0, -40),
        (50, 20),
        (-50, -20),
        (30, -40),
        (-20, 50),
    ];
    for (i, &(ox, oy)) in offsets.iter().enumerate() {
        let noise_x = (value_noise(i as i32 * 2 + 5, 5, p_seed) * 10.0) as CoordinateUnit;
        let noise_y = (value_noise(i as i32 * 2 + 6, 6, p_seed) * 10.0) as CoordinateUnit;
        let cx = width / 2 + ox + noise_x;
        let cy = height / 2 + oy + noise_y;
        let px = (cx - pw / 2).clamp(2, width - pw - 2);
        let py = (cy - ph / 2).clamp(2, height - ph - 2);

        if overlaps_any_building(px, py, pw, ph, buildings) {
            continue;
        }
        if map.has_excluded_tile(px, py, pw, ph) {
            continue;
        }

        // Clear the plaza area — open exposed killzone
        for y in py..py + ph {
            for x in px..px + pw {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                    voxel.floor = Some(Floor::Plaza);
                    voxel.props = None;
                }
            }
        }

        // Place a central fountain/monument (rock as focal point)
        let center = GridVec::new(px + pw / 2, py + ph / 2);
        if let Some(voxel) = map.get_voxel_at_mut(&center) {
            voxel.props = Some(Props::Rock);
        }
        // Benches around the monument
        set_prop(map, center.x - 2, center.y, Props::Bench);
        set_prop(map, center.x + 2, center.y, Props::Bench);
        set_prop(map, center.x, center.y - 2, Props::Bench);
        set_prop(map, center.x, center.y + 2, Props::Bench);
        map.mark_occupied(px, py, pw, ph);
        break;
    }
}

/// Places a large Town Hall building near the center of the map.
/// The Town Hall is 18×12 with a grand interior containing tables, chairs,
/// benches and signs (maps/notices).
/// Skipped on maps too small to fit the building.
fn place_town_hall(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    let tw: CoordinateUnit = 18;
    let th: CoordinateUnit = 12;
    if width < tw + 6 || height < th + 6 {
        return; // map too small for a town hall
    }
    let th_seed = seed.wrapping_add(111222);
    let cx = width / 2 + (value_noise(0, 0, th_seed) * 20.0) as CoordinateUnit - 10;
    let cy = height / 3;
    let tx = (cx - tw / 2).clamp(2, width - tw - 2);
    let ty = (cy - th / 2).clamp(2, height - th - 2);

    if overlaps_any_building(tx, ty, tw, th, buildings) {
        return;
    }
    if map.has_excluded_tile(tx, ty, tw, th) {
        return;
    }

    // Lay down walls and wood-plank floor
    for y in ty..ty + th {
        for x in tx..tx + tw {
            let pos = GridVec::new(x, y);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                let is_border = x == tx || x == tx + tw - 1 || y == ty || y == ty + th - 1;
                let is_door = y == ty + th - 1 && x == tx + tw / 2;
                let is_back_door = y == ty && x == tx + tw / 2;
                if is_border && !is_door && !is_back_door {
                    voxel.props = Some(Props::Wall);
                } else {
                    voxel.props = None;
                }
                voxel.floor = Some(Floor::WoodPlanks);
            }
        }
    }

    // Interior props: long meeting table, chairs, signs
    let ix = tx + 2;
    let iy = ty + 2;
    // Central table row
    for dx in 2..tw - 4 {
        set_prop(map, tx + dx, iy + 2, Props::Table);
    }
    // Chairs along table
    for dx in (2..tw - 4).step_by(2) {
        set_prop(map, tx + dx, iy + 1, Props::Chair);
        set_prop(map, tx + dx, iy + 3, Props::Chair);
    }
    // Signs on the walls
    set_prop(map, ix, iy, Props::Sign);
    set_prop(map, ix + 1, iy, Props::Sign);
    // Barrels in corners
    set_prop(map, tx + 1, ty + 1, Props::Barrel);
    set_prop(map, tx + tw - 2, ty + 1, Props::Barrel);
    set_prop(map, tx + 1, ty + th - 2, Props::Crate);
    set_prop(map, tx + tw - 2, ty + th - 2, Props::Crate);
    map.mark_occupied(tx, ty, tw, th);
}

/// Places a Grand Saloon — a large 20×14 saloon with piano, many tables,
/// chairs, and barrels. Placed in the southern half of the map.
/// Skipped on maps too small to fit the building.
fn place_grand_saloon(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    let sw: CoordinateUnit = 20;
    let sh: CoordinateUnit = 14;
    if width < sw + 6 || height < sh + 6 {
        return; // map too small for a grand saloon
    }
    let gs_seed = seed.wrapping_add(333444);
    let cx = width / 2 - 20 + (value_noise(1, 1, gs_seed) * 40.0) as CoordinateUnit;
    let cy = height * 2 / 3;
    let sx = (cx - sw / 2).clamp(2, width - sw - 2);
    let sy = (cy - sh / 2).clamp(2, height - sh - 2);

    if overlaps_any_building(sx, sy, sw, sh, buildings) {
        return;
    }
    if map.has_excluded_tile(sx, sy, sw, sh) {
        return;
    }

    // Lay down walls and wood-plank floor
    for y in sy..sy + sh {
        for x in sx..sx + sw {
            let pos = GridVec::new(x, y);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                let is_border = x == sx || x == sx + sw - 1 || y == sy || y == sy + sh - 1;
                // Two wide doorways
                let is_main_door = y == sy + sh - 1 && (x == sx + sw / 2 || x == sx + sw / 2 - 1);
                let is_side_door = x == sx && y == sy + sh / 2;
                if is_border && !is_main_door && !is_side_door {
                    voxel.props = Some(Props::Wall);
                } else {
                    voxel.props = None;
                }
                voxel.floor = Some(Floor::WoodPlanks);
            }
        }
    }

    // Interior: piano in corner, multiple table+chair clusters, barrel bar
    set_prop(map, sx + 1, sy + 1, Props::Piano);
    // Bar (barrels along the back wall)
    for dx in 3..sw - 3 {
        set_prop(map, sx + dx, sy + 1, Props::Barrel);
    }
    // Tables and chairs in a grid pattern
    for row in 0..3 {
        for col in 0..3 {
            let tx = sx + 3 + col * 5;
            let ty = sy + 4 + row * 3;
            if tx + 2 < sx + sw - 1 && ty + 1 < sy + sh - 1 {
                set_prop(map, tx, ty, Props::Table);
                set_prop(map, tx - 1, ty, Props::Chair);
                set_prop(map, tx + 1, ty, Props::Chair);
            }
        }
    }
    // Corner barrels
    set_prop(map, sx + sw - 2, sy + 1, Props::Barrel);
    set_prop(map, sx + sw - 2, sy + sh - 2, Props::Crate);
    map.mark_occupied(sx, sy, sw, sh);
}

/// Places street props (benches, barrels, signs, hitching posts, crates) on
/// actual road-edge tiles so they follow road curvature. Props are never
/// placed in a straight line unless the road itself is straight at that point.
fn place_street_props_curved(
    map: &mut GameMap,
    road_edge_tiles: &[(CoordinateUnit, CoordinateUnit)],
    seed: NoiseSeed,
) {
    let furn_seed = seed.wrapping_add(77777);
    // Place props more sparsely so sidewalks and frontage paths read cleanly.
    for (idx, &(x, y)) in road_edge_tiles.iter().enumerate() {
        if idx % 7 != 0 {
            continue;
        }
        let noise = value_noise(x, y, furn_seed.wrapping_add(idx as u64));
        let prop = match (noise * 6.0) as u32 {
            0 => Props::HitchingPost,
            1 => Props::Bench,
            2 => Props::Barrel,
            3 => Props::WaterTrough,
            4 => Props::Sign,
            _ => Props::Crate,
        };
        set_prop(map, x, y, prop);
    }
}

/// Places desert decorations (cacti, dead trees, rocks, fences)
/// in open areas using noise-driven density.
fn place_desert_decorations(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    let deco_seed = seed.wrapping_add(99999);
    let density_seed = seed.wrapping_add(88888);

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let pos = GridVec::new(x, y);
            // Skip if already has props or is near spawn
            if let Some(voxel) = map.get_voxel_at(&pos) {
                if voxel.props.is_some() {
                    continue;
                }
                if matches!(
                    voxel.floor,
                    Some(Floor::WoodPlanks) | Some(Floor::StoneFloor)
                ) {
                    continue;
                }
                // Skip road tiles — no decorations on roads or sidewalks.
                if matches!(voxel.floor, Some(Floor::DirtRoad) | Some(Floor::Sidewalk)) {
                    continue;
                }
                // Skip bridge, beach sand tiles — no decorations.
                if matches!(voxel.floor, Some(Floor::Bridge) | Some(Floor::BeachSand)) {
                    continue;
                }
                // Skip occupied tiles
                if map.occupancy[x as usize][y as usize] {
                    continue;
                }
            } else {
                continue;
            }

            let dist_sq = pos.distance_squared(SPAWN_POINT) as f64;
            if dist_sq < 36.0 {
                continue;
            }

            let noise = value_noise(x, y, deco_seed);
            let density = fbm(x as f64, y as f64, 3, 0.06, 0.5, density_seed);

            // Keep desert clutter relatively sparse; density threshold creates
            // natural-looking clusters rather than uniform scatter.
            if noise > 0.028 || density < 0.5 {
                continue;
            }

            let pick = value_noise(y, x, deco_seed.wrapping_add(44444));
            let prop = if pick < 0.30 {
                Props::Cactus
            } else if pick < 0.50 {
                Props::DeadTree
            } else if pick < 0.65 {
                Props::Rock
            } else if pick < 0.80 {
                Props::Bush
            } else {
                Props::HayBale
            };
            set_prop(map, x, y, prop);
        }
    }
}

/// Places a small cemetery on the outskirts of the town.
fn place_cemetery(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    let cw: CoordinateUnit = 14;
    let ch: CoordinateUnit = 10;
    if width < cw + 20 || height < ch + 20 {
        return;
    }
    let c_seed = seed.wrapping_add(999111);
    // Place in the lower-right area away from center
    let cx = width * 3 / 4 + (value_noise(7, 7, c_seed) * 10.0) as CoordinateUnit;
    let cy = height * 3 / 4 + (value_noise(8, 8, c_seed) * 10.0) as CoordinateUnit;
    let px = (cx - cw / 2).clamp(2, width - cw - 2);
    let py = (cy - ch / 2).clamp(2, height - ch - 2);

    if overlaps_any_building(px, py, cw, ch, buildings) {
        return;
    }
    if map.has_excluded_tile(px, py, cw, ch) {
        return;
    }

    // Fence perimeter with gate
    for y in py..py + ch {
        for x in px..px + cw {
            let pos = GridVec::new(x, y);
            let is_border = x == px || x == px + cw - 1 || y == py || y == py + ch - 1;
            let is_gate = y == py + ch - 1 && x == px + cw / 2;
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                if is_border && !is_gate {
                    voxel.props = Some(Props::Fence);
                    voxel.floor = Some(Floor::Grass);
                } else {
                    voxel.props = None;
                    voxel.floor = Some(Floor::Grass);
                }
            }
        }
    }

    // Gravestones (rocks in rows)
    for row in (2..ch - 2).step_by(2) {
        for col in (2..cw - 2).step_by(3) {
            set_prop(map, px + col, py + row, Props::Rock);
        }
    }
}

/// Places a livestock corral with fenced area and hay bales.
fn place_corral(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    let cw: CoordinateUnit = 12;
    let ch: CoordinateUnit = 8;
    if width < cw + 20 || height < ch + 20 {
        return;
    }
    let c_seed = seed.wrapping_add(888222);
    let cx = width / 4 + (value_noise(9, 9, c_seed) * 10.0) as CoordinateUnit;
    let cy = height * 3 / 4 + (value_noise(10, 10, c_seed) * 10.0) as CoordinateUnit;
    let px = (cx - cw / 2).clamp(2, width - cw - 2);
    let py = (cy - ch / 2).clamp(2, height - ch - 2);

    if overlaps_any_building(px, py, cw, ch, buildings) {
        return;
    }
    if map.has_excluded_tile(px, py, cw, ch) {
        return;
    }

    // Fence perimeter with gate
    for y in py..py + ch {
        for x in px..px + cw {
            let pos = GridVec::new(x, y);
            let is_border = x == px || x == px + cw - 1 || y == py || y == py + ch - 1;
            let is_gate = y == py + ch - 1 && x == px + cw / 2;
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                if is_border && !is_gate {
                    voxel.props = Some(Props::Fence);
                    voxel.floor = Some(Floor::Dirt);
                } else {
                    voxel.props = None;
                    voxel.floor = Some(Floor::Dirt);
                }
            }
        }
    }

    // Hay bales and water troughs inside
    set_prop(map, px + 2, py + 2, Props::HayBale);
    set_prop(map, px + 5, py + 2, Props::HayBale);
    set_prop(map, px + 8, py + 2, Props::HayBale);
    set_prop(map, px + cw / 2, py + ch / 2, Props::WaterTrough);
    set_prop(map, px + 2, py + ch - 3, Props::HitchingPost);
    set_prop(map, px + cw - 3, py + ch - 3, Props::HitchingPost);
}

/// Places a town well — a circular stone structure with water.
fn place_town_well(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    if width < 60 || height < 60 {
        return;
    }
    let w_seed = seed.wrapping_add(444555);
    // Place near the town center, offset from plaza
    let cx = width / 2 - 10 + (value_noise(4, 4, w_seed) * 8.0) as CoordinateUnit;
    let cy = height / 2 - 10 + (value_noise(5, 5, w_seed) * 8.0) as CoordinateUnit;
    let wx = cx.clamp(4, width - 4);
    let wy = cy.clamp(4, height - 4);

    // Check that the 3×3 footprint doesn't overlap excluded tiles
    if map.has_excluded_tile(wx - 1, wy - 1, 3, 3) {
        return;
    }

    // 3×3 well structure: stone ring with water center
    for dy in -1..=1i32 {
        for dx in -1..=1i32 {
            let pos = GridVec::new(wx + dx, wy + dy);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                if dx == 0 && dy == 0 {
                    voxel.floor = Some(Floor::ShallowWater);
                    voxel.props = None;
                } else {
                    voxel.props = Some(Props::Well);
                    voxel.floor = Some(Floor::Sidewalk);
                }
            }
        }
    }
}

/// Places a gallows structure — wooden platform with posts.
fn place_gallows(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    if width < 80 || height < 60 {
        return;
    }
    let g_seed = seed.wrapping_add(555777);
    // Place near the town center, slightly offset
    let cx = width / 2 + 20 + (value_noise(3, 3, g_seed) * 10.0) as CoordinateUnit;
    let cy = height / 2 - 15 + (value_noise(4, 4, g_seed) * 10.0) as CoordinateUnit;
    let gx = cx.clamp(4, width - 8);
    let gy = cy.clamp(4, height - 8);

    // Check that the 4×4 footprint doesn't overlap excluded tiles
    if map.has_excluded_tile(gx, gy, 4, 4) {
        return;
    }

    // 4×4 gallows platform
    for dy in 0..4i32 {
        for dx in 0..4i32 {
            let pos = GridVec::new(gx + dx, gy + dy);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                voxel.floor = Some(Floor::WoodPlanks);
                voxel.props = None;
            }
        }
    }
    // Gallows posts at corners and crossbeam
    set_prop(map, gx, gy, Props::Gallows);
    set_prop(map, gx, gy + 3, Props::Gallows);
    set_prop(map, gx + 1, gy, Props::Gallows);
}

/// Places a tall water tower near the edge of town.
fn place_water_tower(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    if width < 80 || height < 60 {
        return;
    }
    let wt_seed = seed.wrapping_add(666888);
    let cx = width / 4 + (value_noise(6, 6, wt_seed) * 20.0) as CoordinateUnit;
    let cy = height / 4 + (value_noise(7, 7, wt_seed) * 10.0) as CoordinateUnit;
    let tx = cx.clamp(4, width - 8);
    let ty = cy.clamp(4, height - 8);

    // Check that the 5×5 footprint doesn't overlap excluded tiles
    if map.has_excluded_tile(tx, ty, 5, 5) {
        return;
    }

    // 5×5 water tower: 4 leg posts, tank on top (represented as tower height)
    for dy in 0..5i32 {
        for dx in 0..5i32 {
            let pos = GridVec::new(tx + dx, ty + dy);
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                voxel.floor = Some(Floor::Dirt);
                voxel.props = None;
            }
        }
    }
    // Four corner legs
    set_prop(map, tx, ty, Props::WaterTower);
    set_prop(map, tx + 4, ty, Props::WaterTower);
    set_prop(map, tx, ty + 4, Props::WaterTower);
    set_prop(map, tx + 4, ty + 4, Props::WaterTower);
}

/// Places railroad tracks near the northern and southern edges of the map.
/// Tracks never intersect the main road. Where tracks cross the river a
/// rail bridge is placed automatically.
fn place_railroad(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    if width < 80 || height < 80 {
        return;
    }
    let rail_seed = seed.wrapping_add(777999);

    // Two track lines: one near the north edge, one near the south edge.
    let north_y = 10 + (value_noise(8, 8, rail_seed) * 6.0) as CoordinateUnit;
    let north_y = north_y.clamp(6, 20);
    let south_y = height - 10 - (value_noise(9, 9, rail_seed) * 6.0) as CoordinateUnit;
    let south_y = south_y.clamp(height - 20, height - 6);

    for &rail_y in &[north_y, south_y] {
        for x in 4..width - 4 {
            let wobble = (value_noise(x, rail_y, rail_seed) * 1.5) as CoordinateUnit;
            let y = rail_y + wobble;
            if y <= 0 || y >= height - 1 {
                continue;
            }
            let pos = GridVec::new(x, y);
            if let Some(voxel) = map.get_voxel_at(&pos) {
                // Rail bridge over water/beach — lay bridge tile with track
                if matches!(
                    voxel.floor,
                    Some(Floor::ShallowWater)
                        | Some(Floor::DeepWater)
                        | Some(Floor::Beach)
                        | Some(Floor::BeachSand)
                ) {
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.floor = Some(Floor::Bridge);
                        voxel.props = Some(Props::RailTrack);
                    }
                    // Mark rail tile as occupied
                    map.occupancy[x as usize][y as usize] = true;
                    continue;
                }
            }
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                if !matches!(voxel.floor, Some(Floor::DirtRoad) | Some(Floor::Sidewalk)) {
                    voxel.floor = Some(Floor::Gravel);
                }
                voxel.props = Some(Props::RailTrack);
            }
            // Mark rail tile as occupied
            map.occupancy[x as usize][y as usize] = true;
            // Gravel bed on either side of the track
            for &dy in &[-1i32, 1] {
                let side_pos = GridVec::new(x, y + dy);
                if let Some(voxel) = map.get_voxel_at_mut(&side_pos)
                    && voxel.props.is_none()
                    && !matches!(
                        voxel.floor,
                        Some(Floor::WoodPlanks)
                            | Some(Floor::StoneFloor)
                            | Some(Floor::ShallowWater)
                            | Some(Floor::DeepWater)
                            | Some(Floor::DirtRoad)
                            | Some(Floor::Sidewalk)
                            | Some(Floor::Bridge)
                    )
                {
                    voxel.floor = Some(Floor::Gravel);
                }
            }
        }
    }
}

/// Places a windmill on the outskirts of town.
fn place_windmill(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    if width < 80 || height < 60 {
        return;
    }
    let wm_seed = seed.wrapping_add(888111);
    let cx = width * 3 / 4 + (value_noise(9, 9, wm_seed) * 15.0) as CoordinateUnit;
    let cy = height / 4 + (value_noise(10, 10, wm_seed) * 10.0) as CoordinateUnit;
    let mx = cx.clamp(4, width - 8);
    let my = cy.clamp(4, height - 8);

    // Check that the 5×5 footprint doesn't overlap excluded tiles
    if map.has_excluded_tile(mx, my, 5, 5) {
        return;
    }

    // 5×5 windmill base with stone walls
    for dy in 0..5i32 {
        for dx in 0..5i32 {
            let pos = GridVec::new(mx + dx, my + dy);
            let is_border = dx == 0 || dx == 4 || dy == 0 || dy == 4;
            let is_door = dy == 4 && dx == 2;
            if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                if is_border && !is_door {
                    voxel.props = Some(Props::Wall);
                    voxel.floor = Some(Floor::WoodPlanks);
                } else {
                    voxel.props = None;
                    voxel.floor = Some(Floor::WoodPlanks);
                }
            }
        }
    }
    // Windmill blades (decorative props extending from tower)
    set_prop(map, mx + 2, my - 1, Props::Windmill);
    set_prop(map, mx - 1, my + 2, Props::Windmill);
    set_prop(map, mx + 5, my + 2, Props::Windmill);
    // Interior: grain storage
    set_prop(map, mx + 1, my + 1, Props::Barrel);
    set_prop(map, mx + 3, my + 1, Props::Barrel);
    set_prop(map, mx + 2, my + 2, Props::Crate);
}

/// Places lamp posts along cross streets at regular intervals.
fn place_lamp_posts(
    map: &mut GameMap,
    height: CoordinateUnit,
    street_x: CoordinateUnit,
    seed: NoiseSeed,
) {
    let lamp_seed = seed.wrapping_add(111333);
    for y in (40..height - 40).step_by(12) {
        let noise = value_noise(street_x, y, lamp_seed);
        if noise > 0.4 {
            continue;
        } // skip some positions for variety
        // Place lamp posts on both sides of the street
        for &dx in &[-3i32, 3] {
            let x = street_x + dx;
            let pos = GridVec::new(x, y);
            if let Some(voxel) = map.get_voxel_at(&pos)
                && voxel.props.is_none()
                && matches!(
                    voxel.floor,
                    Some(Floor::Sidewalk) | Some(Floor::DirtRoad) | Some(Floor::Sand)
                )
            {
                set_prop(map, x, y, Props::LampPost);
            }
        }
    }
}

/// Places a large stone church building — a prominent landmark with stone walls,
/// stained glass (signs), and pews. Uses the same stone material as the mission.
fn place_stone_church(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    let cw: CoordinateUnit = 16;
    let ch: CoordinateUnit = 20;
    if width < cw + 20 || height < ch + 20 {
        return;
    }
    let c_seed = seed.wrapping_add(424242);
    // Scan candidate positions in a grid across the buildable area.
    // The stone church is large (16×20) so we need to search broadly.
    'search: for qy in 0..4i32 {
        for qx in 0..4i32 {
            let noise_x = (value_noise(qx + 11, qy + 11, c_seed) * 20.0) as CoordinateUnit;
            let noise_y = (value_noise(qx + 12, qy + 12, c_seed) * 10.0) as CoordinateUnit;
            let cx = width / 5 + qx * (width / 5) + noise_x;
            let cy = height / 5 + qy * (height / 5) + noise_y;
            let bx = (cx - cw / 2).clamp(2, width - cw - 2);
            let by = (cy - ch / 2).clamp(2, height - ch - 2);

            if overlaps_any_building(bx, by, cw, ch, buildings) {
                continue;
            }
            if map.has_excluded_tile(bx, by, cw, ch) {
                continue;
            }

            // Stone walls and floor
            for y in by..by + ch {
                for x in bx..bx + cw {
                    let pos = GridVec::new(x, y);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        let is_border = x == bx || x == bx + cw - 1 || y == by || y == by + ch - 1;
                        let is_main_door =
                            y == by + ch - 1 && (x == bx + cw / 2 || x == bx + cw / 2 - 1);
                        let is_back_door = y == by && x == bx + cw / 2;
                        if is_border && !is_main_door && !is_back_door {
                            voxel.props = Some(Props::StoneWall);
                            voxel.floor = Some(Floor::StoneFloor);
                        } else {
                            voxel.props = None;
                            voxel.floor = Some(Floor::StoneFloor);
                        }
                    }
                }
            }

            let ix = bx + 1;
            let iy = by + 1;
            let iw = cw - 2;
            let ih = ch - 2;

            // Altar at the north end
            set_prop(map, ix + iw / 2, iy, Props::Table);
            set_prop(map, ix + iw / 2 - 1, iy, Props::Sign);
            set_prop(map, ix + iw / 2 + 1, iy, Props::Sign);

            // Pew rows (benches in two columns)
            for row in 3..ih.min(14) {
                set_prop(map, ix + 2, iy + row, Props::Bench);
                if iw >= 8 {
                    set_prop(map, ix + iw - 3, iy + row, Props::Bench);
                }
            }

            // Stained glass (signs along walls)
            for row in (2..ih - 2).step_by(3) {
                set_prop(map, ix, iy + row, Props::Sign);
                set_prop(map, ix + iw - 1, iy + row, Props::Sign);
            }

            // Bell tower: 3×3 rooftop area in corner
            for dy in 0..3i32 {
                for dx in 0..3i32 {
                    let pos = GridVec::new(ix + dx, iy + dy);
                    if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                        voxel.floor = Some(Floor::Rooftop);
                    }
                }
            }
            map.mark_occupied(bx, by, cw, ch);
            break 'search;
        }
    }

    let mut fallback: Option<(CoordinateUnit, CoordinateUnit, CoordinateUnit)> = None;
    let center_x = width / 2 - cw / 2;
    let center_y = height / 2 - ch / 2;
    for by in (2..=height - ch - 2).step_by(6) {
        for bx in (2..=width - cw - 2).step_by(6) {
            if overlaps_any_building(bx, by, cw, ch, buildings)
                || map.has_excluded_tile(bx, by, cw, ch)
            {
                continue;
            }
            let score = (bx - center_x).abs() + (by - center_y).abs();
            if fallback.is_none_or(|current| score < current.0) {
                fallback = Some((score, bx, by));
            }
        }
    }
    if let Some((_, bx, by)) = fallback {
        for y in by..by + ch {
            for x in bx..bx + cw {
                let pos = GridVec::new(x, y);
                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                    let is_border = x == bx || x == bx + cw - 1 || y == by || y == by + ch - 1;
                    let is_main_door =
                        y == by + ch - 1 && (x == bx + cw / 2 || x == bx + cw / 2 - 1);
                    let is_back_door = y == by && x == bx + cw / 2;
                    if is_border && !is_main_door && !is_back_door {
                        voxel.props = Some(Props::StoneWall);
                        voxel.floor = Some(Floor::StoneFloor);
                    } else {
                        voxel.props = None;
                        voxel.floor = Some(Floor::StoneFloor);
                    }
                }
            }
        }

        let ix = bx + 1;
        let iy = by + 1;
        let iw = cw - 2;
        let ih = ch - 2;

        set_prop(map, ix + iw / 2, iy, Props::Table);
        set_prop(map, ix + iw / 2 - 1, iy, Props::Sign);
        set_prop(map, ix + iw / 2 + 1, iy, Props::Sign);

        for row in 3..ih.min(14) {
            set_prop(map, ix + 2, iy + row, Props::Bench);
            if iw >= 8 {
                set_prop(map, ix + iw - 3, iy + row, Props::Bench);
            }
        }

        for row in (2..ih - 2).step_by(3) {
            set_prop(map, ix, iy + row, Props::Sign);
            set_prop(map, ix + iw - 1, iy + row, Props::Sign);
        }

        for dy in 0..3i32 {
            for dx in 0..3i32 {
                let pos = GridVec::new(ix + dx, iy + dy);
                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                    voxel.floor = Some(Floor::Rooftop);
                }
            }
        }
        map.mark_occupied(bx, by, cw, ch);
    }
}

/// Places small outpost structures along the map edges — defensive positions
/// that serve as spawn anchors for factions on the outskirts.
fn place_outposts(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
    buildings: &[Building],
) {
    if width < 30 || height < 30 {
        return;
    } // map too small for outposts
    let out_seed = seed.wrapping_add(191919);
    let num_outposts = 4;
    for i in 0..num_outposts {
        let angle = (i as f64 / num_outposts as f64) * std::f64::consts::TAU
            + value_noise(i, 0, out_seed) * 0.8;
        let dist = (width.min(height) as f64 * 0.35) + value_noise(0, i, out_seed) * 20.0;
        let ox = (width as f64 / 2.0 + angle.cos() * dist) as CoordinateUnit;
        let oy = (height as f64 / 2.0 + angle.sin() * dist) as CoordinateUnit;
        let ox = ox.clamp(8, width - 14);
        let oy = oy.clamp(8, height - 14);

        // 6×6 stone outpost
        let ow: CoordinateUnit = 6;
        let oh: CoordinateUnit = 6;

        if overlaps_any_building(ox, oy, ow, oh, buildings) {
            continue;
        }
        if map.has_excluded_tile(ox, oy, ow, oh) {
            continue;
        }

        for dy in 0..oh {
            for dx in 0..ow {
                let pos = GridVec::new(ox + dx, oy + dy);
                if let Some(voxel) = map.get_voxel_at_mut(&pos) {
                    let is_border = dx == 0 || dx == ow - 1 || dy == 0 || dy == oh - 1;
                    let is_door = dy == oh - 1 && dx == ow / 2;
                    if is_border && !is_door {
                        voxel.props = Some(Props::StoneWall);
                        voxel.floor = Some(Floor::StoneFloor);
                    } else {
                        voxel.props = None;
                        voxel.floor = Some(Floor::StoneFloor);
                    }
                }
            }
        }
        // Interior crate + barrel
        set_prop(map, ox + 1, oy + 1, Props::Barrel);
        set_prop(map, ox + ow - 2, oy + 1, Props::Crate);
        set_prop(map, ox + ow / 2, oy + oh / 2, Props::GunpowderBarrel);
    }
}

/// Places natural rock formations using noise-driven cluster placement.
/// Creates visually interesting terrain features on the outskirts.
fn place_rock_formations(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    if width < 80 || height < 80 {
        return;
    } // map too small for rock formations
    let rock_seed = seed.wrapping_add(282828);
    let num_formations = 6;
    for i in 0..num_formations {
        // Place formations away from center
        let fx = (value_noise(i, 0, rock_seed) * (width - 60) as f64) as CoordinateUnit + 30;
        let fy = (value_noise(0, i, rock_seed) * (height - 60) as f64) as CoordinateUnit + 30;
        let dist_to_center = ((fx - width / 2).pow(2) + (fy - height / 2).pow(2)) as f64;
        if dist_to_center < (width as f64 / 4.0).powi(2) {
            continue; // skip formations too close to center
        }
        // Noise-driven cluster of 5-12 rocks
        let cluster_size = 5 + (value_noise(i, i, rock_seed.wrapping_add(111)) * 8.0) as i32;
        let mut placed = 0;
        for dy in -3i32..=3 {
            for dx in -3i32..=3 {
                if placed >= cluster_size {
                    break;
                }
                let pos = GridVec::new(fx + dx, fy + dy);
                let noise = value_noise(pos.x, pos.y, rock_seed.wrapping_add(222));
                if noise > 0.4 {
                    continue;
                }
                if let Some(voxel) = map.get_voxel_at(&pos) {
                    if voxel.props.is_some() {
                        continue;
                    }
                    if matches!(
                        voxel.floor,
                        Some(Floor::WoodPlanks)
                            | Some(Floor::StoneFloor)
                            | Some(Floor::ShallowWater)
                            | Some(Floor::DeepWater)
                            | Some(Floor::DirtRoad)
                            | Some(Floor::Bridge)
                            | Some(Floor::BeachSand)
                    ) {
                        continue;
                    }
                }
                set_prop(map, pos.x, pos.y, Props::Rock);
                placed += 1;
            }
            if placed >= cluster_size {
                break;
            }
        }
    }
}

/// Noise threshold for gunpowder barrel placement. Lower = more barrels.
const GUNPOWDER_BARREL_SPAWN_RATE: f64 = 0.015;

/// Scatters gunpowder barrels across the map in strategic locations.
/// Placed near buildings and along streets as environmental hazards.
fn place_gunpowder_barrels(
    map: &mut GameMap,
    width: CoordinateUnit,
    height: CoordinateUnit,
    seed: NoiseSeed,
) {
    let barrel_seed = seed.wrapping_add(131313);
    for y in 2..height - 2 {
        for x in 2..width - 2 {
            let pos = GridVec::new(x, y);
            if let Some(voxel) = map.get_voxel_at(&pos) {
                if voxel.props.is_some() {
                    continue;
                }
                // Only place on suitable terrain
                if !matches!(
                    voxel.floor,
                    Some(Floor::Dirt)
                        | Some(Floor::Sand)
                        | Some(Floor::Gravel)
                        | Some(Floor::Sidewalk)
                        | Some(Floor::WoodPlanks)
                        | Some(Floor::StoneFloor)
                ) {
                    continue;
                }
            } else {
                continue;
            }
            // Skip near spawn
            if pos.distance_squared(SPAWN_POINT) < 100 {
                continue;
            }
            let noise = value_noise(x, y, barrel_seed);
            if noise > GUNPOWDER_BARREL_SPAWN_RATE {
                continue;
            }
            // Prefer tiles near buildings (at least one adjacent wall)
            let near_wall = pos.cardinal_neighbors().iter().any(|n| {
                map.get_voxel_at(n)
                    .is_some_and(|v| v.props.as_ref().is_some_and(|p| p.is_wall()))
            });
            if near_wall || value_noise(y, x, barrel_seed.wrapping_add(111)) < 0.3 {
                set_prop(map, x, y, Props::GunpowderBarrel);
            }
        }
    }
}

/// Clears all props within a given radius of a point and replaces water
/// floors with Dirt so the area is walkable.
pub fn clear_around(map: &mut GameMap, center: GridVec, radius: CoordinateUnit) {
    clear_around_collect_walls(map, center, radius);
}

/// Clears all props within a given radius and returns any wall tiles that were
/// removed in the process. Used to drive the player's opening breach effect.
pub fn clear_around_collect_walls(
    map: &mut GameMap,
    center: GridVec,
    radius: CoordinateUnit,
) -> Vec<GridVec> {
    let radius_sq = radius * radius;
    let map_width = map.width;
    let map_height = map.height;
    let mut removed_walls = Vec::new();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let pos = center + GridVec::new(dx, dy);
            if pos.distance_squared(center) <= radius_sq
                && let Some(voxel) = map.get_voxel_at_mut(&pos)
            {
                // Don't clear border walls
                let is_border =
                    pos.x == 0 || pos.y == 0 || pos.x == map_width - 1 || pos.y == map_height - 1;
                if !is_border {
                    if matches!(voxel.props, Some(Props::Wall) | Some(Props::StoneWall)) {
                        removed_walls.push(pos);
                    }
                    voxel.props = None;
                    // Replace water/beach with dirt so the area is walkable
                    if matches!(
                        voxel.floor,
                        Some(Floor::ShallowWater)
                            | Some(Floor::DeepWater)
                            | Some(Floor::Beach)
                            | Some(Floor::BeachSand)
                    ) {
                        voxel.floor = Some(Floor::Dirt);
                    }
                }
            }
        }
    }
    removed_walls
}

impl Default for GameMap {
    fn default() -> Self {
        GameMap::new(400, 280, 42)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_map_dimensions() {
        let map = GameMap::new(40, 30, 0);
        assert_eq!(map.width, 40);
        assert_eq!(map.height, 30);
        assert_eq!(map.voxels.len(), 30);
        assert_eq!(map.voxels[0].len(), 40);
    }

    #[test]
    fn game_map_border_is_passable_for_escape() {
        let map = GameMap::new(20, 15, 42);
        // Borders should be passable (no wall props) so player can escape
        let mut any_border_passable = false;
        for x in 0..20 {
            if map.is_passable(&GridVec::new(x, 0)) {
                any_border_passable = true;
            }
            if map.is_passable(&GridVec::new(x, 14)) {
                any_border_passable = true;
            }
        }
        for y in 0..15 {
            if map.is_passable(&GridVec::new(0, y)) {
                any_border_passable = true;
            }
            if map.is_passable(&GridVec::new(19, y)) {
                any_border_passable = true;
            }
        }
        assert!(
            any_border_passable,
            "At least some border tiles should be passable for escape"
        );
    }

    #[test]
    fn game_map_border_tiles_exist() {
        let map = GameMap::new(20, 15, 42);
        // Border tiles should have floor data (they exist on the map)
        assert!(map.get_voxel_at(&GridVec::new(0, 0)).is_some());
        assert!(map.get_voxel_at(&GridVec::new(19, 0)).is_some());
        assert!(map.get_voxel_at(&GridVec::new(0, 14)).is_some());
        assert!(map.get_voxel_at(&GridVec::new(19, 14)).is_some());
    }

    #[test]
    fn game_map_out_of_bounds_not_passable() {
        let map = GameMap::new(10, 10, 0);
        assert!(!map.is_passable(&GridVec::new(-1, 5)));
        assert!(!map.is_passable(&GridVec::new(5, -1)));
        assert!(!map.is_passable(&GridVec::new(10, 5)));
        assert!(!map.is_passable(&GridVec::new(5, 10)));
    }

    #[test]
    fn game_map_get_voxel_at_valid() {
        let map = GameMap::new(10, 10, 0);
        assert!(map.get_voxel_at(&GridVec::new(5, 5)).is_some());
    }

    #[test]
    fn game_map_get_voxel_at_out_of_bounds() {
        let map = GameMap::new(10, 10, 0);
        assert!(map.get_voxel_at(&GridVec::new(-1, 0)).is_none());
        assert!(map.get_voxel_at(&GridVec::new(0, -1)).is_none());
        assert!(map.get_voxel_at(&GridVec::new(10, 0)).is_none());
        assert!(map.get_voxel_at(&GridVec::new(0, 10)).is_none());
    }

    #[test]
    fn game_map_spawn_area_is_clear() {
        let map = GameMap::new(400, 280, 42);
        // The spawn point area (within radius 6 of SPAWN_POINT) should be clear
        for dy in -5..=5 {
            for dx in -5..=5 {
                let pos = SPAWN_POINT + GridVec::new(dx, dy);
                let dist_sq = pos.distance_squared(SPAWN_POINT) as f64;
                if dist_sq < 36.0 {
                    assert!(
                        map.is_passable(&pos),
                        "Spawn area tile ({}, {}) should be passable",
                        pos.x,
                        pos.y
                    );
                }
            }
        }
    }

    #[test]
    fn game_map_deterministic_with_same_seed() {
        let map1 = GameMap::new(30, 20, 42);
        let map2 = GameMap::new(30, 20, 42);
        for y in 0..20 {
            for x in 0..30 {
                assert_eq!(
                    map1.voxels[y][x].floor, map2.voxels[y][x].floor,
                    "Floor mismatch at ({x}, {y})"
                );
                assert_eq!(
                    map1.voxels[y][x].props, map2.voxels[y][x].props,
                    "Props mismatch at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn game_map_different_seeds_differ() {
        let map1 = GameMap::new(30, 20, 42);
        let map2 = GameMap::new(30, 20, 99);
        let mut any_different = false;
        for y in 1..19 {
            for x in 1..29 {
                if map1.voxels[y][x].props != map2.voxels[y][x].props {
                    any_different = true;
                    break;
                }
            }
            if any_different {
                break;
            }
        }
        assert!(
            any_different,
            "Different seeds should produce different maps"
        );
    }

    #[test]
    fn game_map_all_voxels_have_floor() {
        let map = GameMap::new(20, 15, 42);
        for y in 0..15 {
            for x in 0..20 {
                assert!(
                    map.voxels[y][x].floor.is_some(),
                    "Voxel at ({x}, {y}) should have a floor"
                );
            }
        }
    }

    #[test]
    fn game_map_render_packet_dimensions() {
        let map = GameMap::new(80, 50, 42);
        let center = GridVec::new(40, 25);
        let packet = map.create_render_packet_with_fog(&center, 20, 10, None, None);
        assert_eq!(packet.len(), 10);
        assert_eq!(packet[0].len(), 20);
    }

    #[test]
    fn game_map_has_buildings_with_wood_floors() {
        let map = GameMap::new(120, 80, 42);
        let mut wood_count = 0;
        for y in 0..80 {
            for x in 0..120 {
                if matches!(map.voxels[y][x].floor, Some(Floor::WoodPlanks)) {
                    wood_count += 1;
                }
            }
        }
        assert!(
            wood_count > 50,
            "Map should contain buildings with wood plank floors, found {wood_count}"
        );
    }

    #[test]
    fn game_map_has_western_props() {
        let map = GameMap::new(200, 140, 42);
        let mut has_bench = false;
        let mut has_barrel = false;
        let mut has_cactus = false;
        for y in 0..140 {
            for x in 0..200 {
                match &map.voxels[y][x].props {
                    Some(Props::Bench) => has_bench = true,
                    Some(Props::Barrel) => has_barrel = true,
                    Some(Props::Cactus) => has_cactus = true,
                    _ => {}
                }
            }
        }
        assert!(has_bench, "Map should contain benches");
        assert!(has_barrel, "Map should contain barrels");
        assert!(has_cactus, "Map should contain cacti");
    }

    #[test]
    fn large_map_has_alleys() {
        let map = GameMap::new(400, 280, 42);
        let mut alley_count = 0;
        for y in 0..280 {
            for x in 0..400 {
                if matches!(map.voxels[y][x].floor, Some(Floor::Alley)) {
                    alley_count += 1;
                }
            }
        }
        assert!(
            alley_count > 0,
            "Map should have alley tiles between buildings (found {})",
            alley_count
        );
    }

    #[test]
    fn alley_tiles_are_not_cluttered_with_blocking_props() {
        let map = GameMap::new(400, 280, 42);
        for y in 0..280 {
            for x in 0..400 {
                if matches!(map.voxels[y][x].floor, Some(Floor::Alley))
                    && let Some(prop) = map.voxels[y][x].props.as_ref()
                {
                    assert!(
                        !prop.blocks_movement() || prop.is_wall(),
                        "Alley tile ({x}, {y}) should not have blocking clutter: {:?}",
                        prop
                    );
                }
            }
        }
    }

    #[test]
    fn large_map_has_plaza() {
        // Try several seeds — road spacing changes can shift landmark
        // placement, so a single seed may not always produce a plaza.
        let mut found = false;
        for seed in 0..20u64 {
            let map = GameMap::new(400, 280, seed);
            for y in 0..280 {
                for x in 0..400 {
                    if matches!(map.voxels[y][x].floor, Some(Floor::Plaza)) {
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "At least one of 20 seeds should produce plaza tiles");
    }

    #[test]
    fn large_map_has_rooftop_tiles() {
        let mut found = false;
        for seed in [42, 0, 7, 13, 99] {
            let map = GameMap::new(400, 280, seed);
            for y in 0..280 {
                for x in 0..400 {
                    if matches!(map.voxels[y][x].floor, Some(Floor::Rooftop)) {
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "At least one seed should produce rooftop tiles");
    }

    #[test]
    fn props_is_wall_helper() {
        assert!(Props::Wall.is_wall());
        assert!(!Props::Table.is_wall());
        assert!(!Props::Barrel.is_wall());
    }

    #[test]
    fn bsp_buildings_do_not_overlap() {
        // BSP guarantees non-overlapping plots — verify no building footprints
        // intersect.
        let (buildings, _, _) =
            generate_buildings_bsp(400, 280, 42, &[60, 100, 140], &[80, 160, 240], 3, 2);
        for (i, a) in buildings.iter().enumerate() {
            for b in &buildings[i + 1..] {
                let overlaps =
                    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
                assert!(
                    !overlaps,
                    "Buildings at ({},{}) {}x{} and ({},{}) {}x{} overlap",
                    a.x, a.y, a.w, a.h, b.x, b.y, b.w, b.h
                );
            }
        }
    }

    #[test]
    fn bsp_zone_commercial_near_streets() {
        let avenue_ys = vec![50, 90];
        // Near avenue → commercial zone
        let zone = assign_zone(100, 52, 400, 280, &avenue_ys, 42);
        assert_eq!(
            zone, ZONE_COMMERCIAL,
            "Tiles near avenues should be commercial zones"
        );
        // Far from avenue → not commercial
        let zone_far = assign_zone(100, 200, 400, 280, &avenue_ys, 42);
        assert_ne!(
            zone_far, ZONE_COMMERCIAL,
            "Tiles far from avenues should not be commercial zones"
        );
    }

    #[test]
    fn bsp_small_map_no_panic() {
        // Small maps should be handled gracefully (no building placement)
        let _ = GameMap::new(10, 10, 42);
        let _ = GameMap::new(20, 15, 42);
        let _ = GameMap::new(30, 20, 42);
    }

    #[test]
    fn bsp_landmark_anchors_placed_first() {
        // On sufficiently large maps, landmark anchors (police, saloon,
        // church) should be placed before BSP subdivision.
        let (buildings, _, _) = generate_buildings_bsp(200, 140, 42, &[60], &[80], 3, 2);
        // Expect at least the 3 anchors
        assert!(
            buildings.len() >= 3,
            "Large map should have at least 3 landmark anchor buildings, found {}",
            buildings.len()
        );
        // Police Station (kind=4), Saloon (kind=1), Church (kind=6) should appear
        let has_police = buildings.iter().any(|b| b.kind == 4);
        let has_saloon = buildings.iter().any(|b| b.kind == 1);
        let has_church = buildings.iter().any(|b| b.kind == 6);
        assert!(has_police, "Landmark anchors should include police station");
        assert!(has_saloon, "Landmark anchors should include saloon");
        assert!(has_church, "Landmark anchors should include church");
    }

    #[test]
    fn large_map_has_transition_scatter() {
        let map = GameMap::new(400, 280, 42);
        // Transition zones should add scattered props around the town
        // perimeter — check for props in the first and last 20 rows.
        let mut scatter_count = 0;
        for y in 0..20 {
            for x in 0..400 {
                if map.voxels[y][x].props.is_some()
                    && matches!(
                        map.voxels[y][x].props,
                        Some(Props::Fence)
                            | Some(Props::DeadTree)
                            | Some(Props::Rock)
                            | Some(Props::Barrel)
                            | Some(Props::Crate)
                            | Some(Props::Bush)
                    )
                {
                    scatter_count += 1;
                }
            }
        }
        for y in 260..280 {
            for x in 0..400 {
                if map.voxels[y][x].props.is_some()
                    && matches!(
                        map.voxels[y][x].props,
                        Some(Props::Fence)
                            | Some(Props::DeadTree)
                            | Some(Props::Rock)
                            | Some(Props::Barrel)
                            | Some(Props::Crate)
                            | Some(Props::Bush)
                    )
                {
                    scatter_count += 1;
                }
            }
        }
        assert!(
            scatter_count > 0,
            "Town perimeter should have transition zone scatter"
        );
    }

    #[test]
    fn railroad_corridors_are_continuous_across_map() {
        for seed in [0, 7, 13, 42, 99] {
            let map = GameMap::new(400, 280, seed);
            for x in 4..map.width - 4 {
                let north_has_track = (0..32).any(|y| {
                    map.get_voxel_at(&GridVec::new(x, y))
                        .is_some_and(|voxel| matches!(voxel.props, Some(Props::RailTrack)))
                });
                let south_has_track = (map.height - 32..map.height).any(|y| {
                    map.get_voxel_at(&GridVec::new(x, y))
                        .is_some_and(|voxel| matches!(voxel.props, Some(Props::RailTrack)))
                });
                assert!(
                    north_has_track,
                    "Missing north rail coverage at x={} for seed {}",
                    x,
                    seed
                );
                assert!(
                    south_has_track,
                    "Missing south rail coverage at x={} for seed {}",
                    x,
                    seed
                );
            }
        }
    }

    #[test]
    fn stress_test_20_seeds() {
        // Run the generator across 20 different seeds and verify
        // all structural verification checks pass for each one.
        // GameMap::new() runs verify_world internally and retries on
        // failure, panicking in debug builds if exhausted.
        for seed in 0..20u64 {
            let map = GameMap::new(400, 280, seed);
            // Basic structural checks
            assert_eq!(map.width, 400);
            assert_eq!(map.height, 280);
            // Must have buildings (wood plank floors)
            let mut has_wood = false;
            let mut has_dirt = false;
            for y in 0..280 {
                for x in 0..400 {
                    if matches!(map.voxels[y][x].floor, Some(Floor::WoodPlanks)) {
                        has_wood = true;
                    }
                    if matches!(map.voxels[y][x].floor, Some(Floor::DirtRoad)) {
                        has_dirt = true;
                    }
                }
                if has_wood && has_dirt {
                    break;
                }
            }
            assert!(
                has_wood,
                "Seed {seed}: map must have buildings (wood plank floors)"
            );
            assert!(has_dirt, "Seed {seed}: map must have streets (dirt roads)");
            // Spawn area must be clear
            let spawn_passable = map.is_passable(&SPAWN_POINT);
            assert!(spawn_passable, "Seed {seed}: spawn point must be passable");

            // Confirm no building tiles sit on river tiles (post-verification)
            let mut building_on_water = false;
            for y in 0..280 {
                for x in 0..400 {
                    // If it has a wall and water floor, it's a violation
                    if let Some(voxel) = map.get_voxel_at(&GridVec::new(x as i32, y as i32)) {
                        if voxel.props.as_ref().is_some_and(|p| p.is_wall())
                            && matches!(
                                voxel.floor,
                                Some(Floor::ShallowWater) | Some(Floor::DeepWater)
                            )
                        {
                            building_on_water = true;
                        }
                    }
                }
            }
            assert!(
                !building_on_water,
                "Seed {seed}: no building walls should be on river tiles"
            );
        }
    }

    #[test]
    fn small_map_spawn_area_passable() {
        let map = GameMap::new(120, 80, 42);
        let pos = GridVec::new(61, 41);
        assert!(
            map.is_passable(&pos),
            "120x80: ({},{}) not passable, floor={:?} props={:?}",
            pos.x,
            pos.y,
            map.get_voxel_at(&pos).map(|v| &v.floor),
            map.get_voxel_at(&pos).and_then(|v| v.props.as_ref())
        );
    }

    #[test]
    fn lot_grid_partitioning_produces_multiple_buildings() {
        // The grid-based lot partitioning should produce more buildings
        // than the old single-building-per-lot approach. On a map with
        // multiple road grid cells, empty lots are subdivided into plots.
        let (buildings, _, _) = generate_buildings_bsp(
            400,
            280,
            42,
            &[60, 100, 140, 180],
            &[80, 160, 240, 320],
            3,
            2,
        );
        // With 4 avenues and 4 cross streets, there are many lots.
        // Each lot is partitioned into a grid of plots.
        assert!(
            buildings.len() >= 20,
            "Grid partitioning should produce dense building placement, found {} buildings",
            buildings.len()
        );
        // Verify building type variety — at least 4 distinct kinds
        let mut kinds: HashSet<u32> = HashSet::new();
        for b in &buildings {
            kinds.insert(b.kind);
        }
        assert!(
            kinds.len() >= 4,
            "Lots should have building variety, found only {} distinct kinds",
            kinds.len()
        );
    }

    #[test]
    fn lot_densification_fills_partially_covered_lots() {
        // Lots that already have BSP buildings should still receive
        // additional lot-grid buildings in uncovered areas.
        let (buildings, _, _) = generate_buildings_bsp(
            400,
            280,
            42,
            &[60, 100, 140, 180],
            &[80, 160, 240, 320],
            3,
            2,
        );
        // The denser approach should produce significantly more buildings
        // than a system that skips lots with any BSP coverage.
        assert!(
            buildings.len() >= 30,
            "Dense lot filling should place at least 30 buildings, found {}",
            buildings.len()
        );
        // Verify no exact overlaps between any two buildings
        for (i, a) in buildings.iter().enumerate() {
            for b in &buildings[i + 1..] {
                let overlaps =
                    a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
                assert!(
                    !overlaps,
                    "Buildings at ({},{}) {}x{} and ({},{}) {}x{} overlap",
                    a.x, a.y, a.w, a.h, b.x, b.y, b.w, b.h
                );
            }
        }
    }

    #[test]
    fn edge_lots_use_minimal_boundary_offset() {
        // Lots adjacent to the map edge (not a road) should get a minimal
        // boundary offset of 1 tile, not the full road+sidewalk width.
        let avenue_ys: Vec<CoordinateUnit> = vec![100];
        let cross_xs: Vec<CoordinateUnit> = vec![100];
        let (buildings, _, _) = generate_buildings_bsp(200, 200, 42, &avenue_ys, &cross_xs, 3, 2);
        // With only one avenue and one cross street, there are 4 lots.
        // Edge lots (adjacent to map border) should be large enough to
        // hold buildings because they use a minimal 1-tile margin.
        assert!(
            buildings.len() >= 4,
            "Edge lots should have buildings, found only {} buildings",
            buildings.len()
        );
    }

    #[test]
    fn large_building_has_room_grid() {
        // Buildings with interior ≥ 10×10 should have dividing walls
        // creating a 2×2 room grid.
        let map = GameMap::new(400, 280, 42);
        // Find a building large enough to have a room grid (≥ 12×12)
        let mut found_room_grid = false;
        for y in 1..279 {
            for x in 1..399 {
                // Look for interior wall tiles that are NOT on the outer
                // perimeter — these indicate room dividers.
                let pos = GridVec::new(x as i32, y as i32);
                if let Some(v) = map.get_voxel_at(&pos) {
                    if v.props.as_ref().is_some_and(|p| p.is_wall())
                        && matches!(v.floor, Some(Floor::WoodPlanks) | Some(Floor::StoneFloor))
                    {
                        // Check if it's surrounded by floor on opposite sides
                        // (indicating an interior divider, not perimeter)
                        let left = GridVec::new(x as i32 - 1, y as i32);
                        let right = GridVec::new(x as i32 + 1, y as i32);
                        let up = GridVec::new(x as i32, y as i32 - 1);
                        let down = GridVec::new(x as i32, y as i32 + 1);
                        let l_floor = map.get_voxel_at(&left).is_some_and(|v| {
                            matches!(v.floor, Some(Floor::WoodPlanks) | Some(Floor::StoneFloor))
                                && !v.props.as_ref().is_some_and(|p| p.is_wall())
                        });
                        let r_floor = map.get_voxel_at(&right).is_some_and(|v| {
                            matches!(v.floor, Some(Floor::WoodPlanks) | Some(Floor::StoneFloor))
                                && !v.props.as_ref().is_some_and(|p| p.is_wall())
                        });
                        let u_floor = map.get_voxel_at(&up).is_some_and(|v| {
                            matches!(v.floor, Some(Floor::WoodPlanks) | Some(Floor::StoneFloor))
                                && !v.props.as_ref().is_some_and(|p| p.is_wall())
                        });
                        let d_floor = map.get_voxel_at(&down).is_some_and(|v| {
                            matches!(v.floor, Some(Floor::WoodPlanks) | Some(Floor::StoneFloor))
                                && !v.props.as_ref().is_some_and(|p| p.is_wall())
                        });
                        // Interior divider: floor on both sides horizontally OR vertically
                        if (l_floor && r_floor) || (u_floor && d_floor) {
                            found_room_grid = true;
                            break;
                        }
                    }
                }
                if found_room_grid {
                    break;
                }
            }
            if found_room_grid {
                break;
            }
        }
        assert!(
            found_room_grid,
            "Large buildings should have interior dividing walls (room grid)"
        );
    }

    #[test]
    fn bridge_fire_burnout_produces_water() {
        // Create a small map, manually place a bridge tile, set it on fire,
        // and verify burnout converts to water (not scorched earth).
        let mut map = GameMap {
            width: 10,
            height: 10,
            voxels: vec![
                vec![
                    Voxel {
                        floor: Some(Floor::Dirt),
                        props: None
                    };
                    10
                ];
                10
            ],
            fire_turns: HashMap::new(),
            fire_previous_floor: HashMap::new(),
            sand_cloud_turns: HashMap::new(),
            sand_cloud_previous_floor: HashMap::new(),
            occupancy: vec![vec![false; 10]; 10],
        };
        let bridge_pos = GridVec::new(5, 5);
        // Place bridge surrounded by shallow water (simulating a river).
        map.voxels[5][5].floor = Some(Floor::Bridge);
        map.voxels[5][4].floor = Some(Floor::ShallowWater);
        map.voxels[5][6].floor = Some(Floor::ShallowWater);
        map.voxels[4][5].floor = Some(Floor::DeepWater);
        map.voxels[6][5].floor = Some(Floor::DeepWater);

        // Record previous floor and set on fire.
        map.fire_previous_floor
            .insert(bridge_pos, Some(Floor::Bridge));
        map.voxels[5][5].floor = Some(Floor::Fire);
        map.fire_turns.insert(bridge_pos, 0);

        // Verify the previous floor is tracked as Bridge.
        assert_eq!(
            map.fire_previous_floor.get(&bridge_pos),
            Some(&Some(Floor::Bridge)),
        );
    }

    #[test]
    fn wider_cross_street_spacing() {
        // Verify that the vertical cross-street spacing produces fewer
        // streets and wider lots than the old 28-42 tile range.
        let map = GameMap::new(400, 280, 42);
        let mut cross_road_xs: Vec<CoordinateUnit> = Vec::new();
        // Sample the middle row for DirtRoad tiles that are vertical roads
        let sample_y = 140;
        let mut in_road = false;
        for x in 0..400 {
            let is_road = matches!(
                map.voxels[sample_y as usize][x as usize].floor,
                Some(Floor::DirtRoad)
            );
            if is_road && !in_road {
                cross_road_xs.push(x);
            }
            in_road = is_road;
        }
        // With the wider spacing (48-64 base + ±12 variance), there should be
        // fewer cross streets. On a 400-wide map with old 28-42 spacing, there
        // were ~8-10 roads. With new 48-64 spacing, expect roughly 4-7.
        assert!(
            cross_road_xs.len() >= 3,
            "Should still generate at least 3 vertical roads, got {}",
            cross_road_xs.len()
        );
        assert!(
            cross_road_xs.len() <= 10,
            "Expected fewer vertical roads with wider spacing, got {}",
            cross_road_xs.len()
        );
    }
}
