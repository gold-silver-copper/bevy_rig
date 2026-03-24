/// `GridVec` — a 2D integer vector forming an Abelian group under addition.
///
/// This is the fundamental coordinate type for the roguelike grid. It provides:
/// - **Algebraic structure**: `Add`, `Sub`, `Neg`, `AddAssign`, `SubAssign`
///   (the integers under addition form an Abelian group: associative,
///   commutative, identity element `ZERO`, and every element has an inverse).
/// - **Distance metrics**: Manhattan (L₁), Chebyshev (L∞), squared Euclidean.
/// - **Zero-cost abstraction**: `Copy` + `#[repr(C)]` + inline arithmetic.
///
/// Using a named struct instead of a raw tuple `(i32, i32)` gives us type
/// safety (cannot accidentally swap x/y with unrelated tuples), enables
/// method syntax, and makes the code self-documenting.
use std::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use crate::typedefs::CoordinateUnit;

/// A 2D integer vector on the game grid.
///
/// Forms an **Abelian group** (ℤ², +) with:
/// - **Closure**: GridVec + GridVec → GridVec
/// - **Associativity**: (a + b) + c = a + (b + c)
/// - **Identity**: GridVec::ZERO (additive identity)
/// - **Inverse**: -v for every v (additive inverse via `Neg`)
/// - **Commutativity**: a + b = b + a
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(C)]
pub struct GridVec {
    pub x: CoordinateUnit,
    pub y: CoordinateUnit,
}

impl GridVec {
    /// The additive identity element: (0, 0).
    pub const ZERO: Self = Self { x: 0, y: 0 };

    /// Cardinal direction unit vectors.
    pub const NORTH: Self = Self { x: 0, y: 1 };
    pub const SOUTH: Self = Self { x: 0, y: -1 };
    pub const EAST: Self = Self { x: 1, y: 0 };
    pub const WEST: Self = Self { x: -1, y: 0 };

    /// Diagonal direction unit vectors.
    pub const NORTHEAST: Self = Self { x: 1, y: 1 };
    pub const NORTHWEST: Self = Self { x: -1, y: 1 };
    pub const SOUTHEAST: Self = Self { x: 1, y: -1 };
    pub const SOUTHWEST: Self = Self { x: -1, y: -1 };

    /// All 8 directional unit vectors (Moore neighbourhood).
    pub const DIRECTIONS_8: [Self; 8] = [
        Self::NORTH,
        Self::NORTHEAST,
        Self::EAST,
        Self::SOUTHEAST,
        Self::SOUTH,
        Self::SOUTHWEST,
        Self::WEST,
        Self::NORTHWEST,
    ];

    /// The 4 cardinal directions (Von Neumann neighbourhood).
    pub const DIRECTIONS_4: [Self; 4] = [Self::NORTH, Self::EAST, Self::SOUTH, Self::WEST];

    /// Construct a new `GridVec`.
    #[inline]
    pub const fn new(x: CoordinateUnit, y: CoordinateUnit) -> Self {
        Self { x, y }
    }

    /// Manhattan distance (L₁ norm): |Δx| + |Δy|.
    ///
    /// The natural distance metric for 4-connected grids.
    /// Counts the minimum number of cardinal moves to reach `other`.
    #[inline]
    pub fn manhattan_distance(self, other: Self) -> CoordinateUnit {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }

    /// Chebyshev distance (L∞ norm): max(|Δx|, |Δy|).
    ///
    /// The natural distance metric for 8-connected grids (king moves).
    /// Counts the minimum number of 8-directional moves to reach `other`.
    #[inline]
    pub fn chebyshev_distance(self, other: Self) -> CoordinateUnit {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }

    /// Squared Euclidean distance: Δx² + Δy².
    ///
    /// Avoids the `sqrt` call — use for comparisons where the exact
    /// distance is not needed, only the ordering (e.g., "is A closer
    /// than B?"). Correct because √ is monotonic.
    #[inline]
    pub fn distance_squared(self, other: Self) -> CoordinateUnit {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }

    /// Squared length (norm²) of this vector: x² + y².
    ///
    /// Equivalent to `self.distance_squared(ZERO)` but more idiomatic
    /// for expressing magnitude. Useful as the denominator in
    /// projection formulas and for sorting by distance from origin.
    ///
    /// ‖v‖² = v · v (the dot product of a vector with itself).
    #[inline]
    pub fn norm_squared(self) -> CoordinateUnit {
        self.x * self.x + self.y * self.y
    }

    /// Euclidean distance (L₂ norm): √(Δx² + Δy²).
    ///
    /// The true straight-line distance between two grid points.
    /// Prefer `distance_squared` for comparisons (avoids `sqrt`);
    /// use this when the actual distance value is needed (e.g.,
    /// for attenuation, display, or non-monotonic formulas).
    #[inline]
    pub fn euclidean_distance(self, other: Self) -> f64 {
        (self.distance_squared(other) as f64).sqrt()
    }

    /// Returns `true` if this vector is the zero (identity) element.
    #[inline]
    pub fn is_zero(self) -> bool {
        self.x == 0 && self.y == 0
    }

    /// Dot product (inner product): a · b = aₓbₓ + aᵧbᵧ.
    ///
    /// The dot product captures the projection of one vector onto another.
    /// Useful for determining alignment:
    /// - Positive → vectors point in the same half-plane.
    /// - Zero → vectors are perpendicular (orthogonal).
    /// - Negative → vectors point in opposite half-planes.
    ///
    /// Also equals ‖a‖‖b‖cos θ, making it a key building block for
    /// angle calculations, line-of-sight checks, and lighting.
    #[inline]
    pub fn dot(self, other: Self) -> CoordinateUnit {
        self.x * other.x + self.y * other.y
    }

    /// 2D cross product (wedge product / perp-dot product):
    ///   a × b = aₓbᵧ − aᵧbₓ
    ///
    /// Geometrically, this is the **signed area** of the parallelogram
    /// spanned by `self` and `other`, and equals the determinant of the
    /// 2×2 matrix [self | other]:
    ///
    ///   det | aₓ  bₓ |
    ///       | aᵧ  bᵧ |  = aₓbᵧ − aᵧbₓ
    ///
    /// The sign determines orientation:
    /// - **Positive** → `other` is counter-clockwise from `self`.
    /// - **Zero** → vectors are collinear (parallel or anti-parallel).
    /// - **Negative** → `other` is clockwise from `self`.
    ///
    /// Essential for line-of-sight, convex hull, and turn-direction tests.
    #[inline]
    pub fn cross(self, other: Self) -> CoordinateUnit {
        self.x * other.y - self.y * other.x
    }

    /// Normalizes to a unit king-move step: each component clamped to {−1, 0, 1}.
    ///
    /// Projects any vector onto the Chebyshev unit ball, yielding the
    /// single-step 8-directional movement that best approximates the
    /// vector's direction. Equivalent to component-wise `signum`.
    #[inline]
    pub fn king_step(self) -> Self {
        Self {
            x: self.x.signum(),
            y: self.y.signum(),
        }
    }

    /// 90° clockwise rotation on the ℤ² lattice.
    ///
    /// Applies the rotation matrix R₉₀_cw = [[0, 1], [−1, 0]]:
    ///   (x, y) ↦ (y, −x)
    ///
    /// This is an element of SO(2) restricted to the integer lattice,
    /// forming a cyclic group of order 4: {I, R, R², R³}.
    /// Four applications return to the original vector (R⁴ = I).
    #[inline]
    pub fn rotate_90_cw(self) -> Self {
        Self {
            x: self.y,
            y: -self.x,
        }
    }

    /// 90° counter-clockwise rotation on the ℤ² lattice.
    ///
    /// Applies the rotation matrix R₉₀_ccw = [[0, −1], [1, 0]]:
    ///   (x, y) ↦ (−y, x)
    ///
    /// Inverse of `rotate_90_cw`: `v.rotate_90_cw().rotate_90_ccw() == v`.
    #[inline]
    pub fn rotate_90_ccw(self) -> Self {
        Self {
            x: -self.y,
            y: self.x,
        }
    }

    /// Approximate 45° clockwise rotation on the ℤ² lattice for king-step vectors.
    /// Maps each direction to the next clockwise direction in `DIRECTIONS_8`.
    /// For non-unit vectors, normalizes first via `king_step`.
    #[inline]
    pub fn rotate_45_cw(self) -> Self {
        let n = self.king_step();
        let idx = Self::DIRECTIONS_8
            .iter()
            .position(|&d| d == n)
            .map(|i| (i + 1) % 8)
            .unwrap_or(0);
        Self::DIRECTIONS_8[idx]
    }

    /// Bresenham line from `self` to `other`, inclusive of both endpoints.
    ///
    /// Returns the sequence of integer grid points that best approximates
    /// the straight line segment between two grid positions. Uses the
    /// standard Bresenham algorithm with integer-only arithmetic.
    ///
    /// **Properties:**
    /// - **Exact endpoints**: first element is `self`, last is `other`.
    /// - **Connectivity**: each successive pair differs by at most 1 in
    ///   each axis (8-connected), and by exactly 1 in the major axis.
    /// - **Deterministic**: pure function of endpoints, no floating-point.
    /// - **Time**: O(max(|Δx|, |Δy|)) — visits each pixel exactly once.
    /// - **Symmetry**: `a.bresenham_line(b)` and `b.bresenham_line(a)`
    ///   traverse the same set of points (in reverse order).
    pub fn bresenham_line(self, other: Self) -> Vec<Self> {
        let dx = (other.x - self.x).abs();
        let dy = (other.y - self.y).abs();
        let sx = if self.x < other.x { 1 } else { -1 };
        let sy = if self.y < other.y { 1 } else { -1 };
        let mut err = dx - dy;

        let mut points = Vec::with_capacity((dx.max(dy) + 1) as usize);
        let mut current = self;

        loop {
            points.push(current);
            if current == other {
                break;
            }
            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                current.x += sx;
            }
            if e2 < dx {
                err += dx;
                current.y += sy;
            }
        }

        points
    }

    /// Returns the 4 cardinal (Von Neumann) neighbours of this point.
    #[inline]
    pub fn cardinal_neighbors(self) -> [Self; 4] {
        [
            self + Self::NORTH,
            self + Self::EAST,
            self + Self::SOUTH,
            self + Self::WEST,
        ]
    }

    /// Returns all 8 (Moore) neighbours of this point.
    #[inline]
    pub fn all_neighbors(self) -> [Self; 8] {
        [
            self + Self::NORTH,
            self + Self::NORTHEAST,
            self + Self::EAST,
            self + Self::SOUTHEAST,
            self + Self::SOUTH,
            self + Self::SOUTHWEST,
            self + Self::WEST,
            self + Self::NORTHWEST,
        ]
    }
}

// ─── Abelian group operations ───────────────────────────────────────────────

impl Add for GridVec {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Sub for GridVec {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl Neg for GridVec {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
        }
    }
}

impl AddAssign for GridVec {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl SubAssign for GridVec {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

/// Scalar multiplication (ℤ-module structure): allows `grid_vec * scalar`.
impl Mul<CoordinateUnit> for GridVec {
    type Output = Self;
    #[inline]
    fn mul(self, scalar: CoordinateUnit) -> Self {
        Self {
            x: self.x * scalar,
            y: self.y * scalar,
        }
    }
}

/// Scalar multiplication assignment: allows `grid_vec *= scalar`.
impl MulAssign<CoordinateUnit> for GridVec {
    #[inline]
    fn mul_assign(&mut self, scalar: CoordinateUnit) {
        self.x *= scalar;
        self.y *= scalar;
    }
}

// ─── Conversions ────────────────────────────────────────────────────────────

impl From<(CoordinateUnit, CoordinateUnit)> for GridVec {
    #[inline]
    fn from((x, y): (CoordinateUnit, CoordinateUnit)) -> Self {
        Self { x, y }
    }
}

impl From<GridVec> for (CoordinateUnit, CoordinateUnit) {
    #[inline]
    fn from(v: GridVec) -> Self {
        (v.x, v.y)
    }
}

// ─── Display ────────────────────────────────────────────────────────────────

impl std::fmt::Display for GridVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_identity() {
        let v = GridVec::new(3, -7);
        assert_eq!(v + GridVec::ZERO, v);
        assert_eq!(GridVec::ZERO + v, v);
    }

    #[test]
    fn additive_inverse() {
        let v = GridVec::new(5, -3);
        assert_eq!(v + (-v), GridVec::ZERO);
    }

    #[test]
    fn commutativity() {
        let a = GridVec::new(2, 3);
        let b = GridVec::new(-1, 7);
        assert_eq!(a + b, b + a);
    }

    #[test]
    fn associativity() {
        let a = GridVec::new(1, 2);
        let b = GridVec::new(3, 4);
        let c = GridVec::new(5, 6);
        assert_eq!((a + b) + c, a + (b + c));
    }

    #[test]
    fn sub_is_add_neg() {
        let a = GridVec::new(10, 20);
        let b = GridVec::new(3, 7);
        assert_eq!(a - b, a + (-b));
    }

    #[test]
    fn manhattan_distance() {
        let a = GridVec::new(0, 0);
        let b = GridVec::new(3, 4);
        assert_eq!(a.manhattan_distance(b), 7);
        // Symmetric
        assert_eq!(b.manhattan_distance(a), 7);
    }

    #[test]
    fn chebyshev_distance() {
        let a = GridVec::new(0, 0);
        let b = GridVec::new(3, 4);
        assert_eq!(a.chebyshev_distance(b), 4);
        assert_eq!(b.chebyshev_distance(a), 4);
    }

    #[test]
    fn distance_squared() {
        let a = GridVec::new(0, 0);
        let b = GridVec::new(3, 4);
        assert_eq!(a.distance_squared(b), 25); // 3² + 4² = 25
    }

    #[test]
    fn scalar_multiplication() {
        let v = GridVec::new(2, -3);
        assert_eq!(v * 3, GridVec::new(6, -9));
        let zero = 0;
        assert_eq!(v * zero, GridVec::ZERO);
    }

    #[test]
    fn tuple_round_trip() {
        let v = GridVec::new(42, -17);
        let tuple: (i32, i32) = v.into();
        let back: GridVec = tuple.into();
        assert_eq!(v, back);
    }

    #[test]
    fn directions_are_unit_length() {
        for dir in &GridVec::DIRECTIONS_4 {
            assert_eq!(dir.manhattan_distance(GridVec::ZERO), 1);
        }
    }

    // ─── Dot product tests ──────────────────────────────────────
    #[test]
    fn dot_product_orthogonal_is_zero() {
        let a = GridVec::EAST;
        let b = GridVec::NORTH;
        assert_eq!(a.dot(b), 0);
    }

    #[test]
    fn dot_product_parallel_positive() {
        let a = GridVec::new(3, 4);
        assert_eq!(a.dot(a), 25); // 3² + 4² = 25
    }

    #[test]
    fn dot_product_antiparallel_negative() {
        let a = GridVec::new(1, 0);
        let b = GridVec::new(-1, 0);
        assert!(a.dot(b) < 0);
    }

    #[test]
    fn dot_product_commutative() {
        let a = GridVec::new(2, 5);
        let b = GridVec::new(-3, 7);
        assert_eq!(a.dot(b), b.dot(a));
    }

    // ─── Cross product tests ────────────────────────────────────
    #[test]
    fn cross_product_collinear_is_zero() {
        let a = GridVec::new(2, 4);
        let b = GridVec::new(1, 2);
        assert_eq!(a.cross(b), 0);
    }

    #[test]
    fn cross_product_perpendicular() {
        let a = GridVec::EAST;
        let b = GridVec::NORTH;
        assert_eq!(a.cross(b), 1); // counter-clockwise
    }

    #[test]
    fn cross_product_anticommutative() {
        let a = GridVec::new(3, 1);
        let b = GridVec::new(-2, 5);
        assert_eq!(a.cross(b), -b.cross(a));
    }

    #[test]
    fn cross_product_self_is_zero() {
        let v = GridVec::new(7, -3);
        assert_eq!(v.cross(v), 0);
    }

    // ─── King step tests ────────────────────────────────────────
    #[test]
    fn king_step_normalizes_large_vector() {
        let v = GridVec::new(10, -5);
        assert_eq!(v.king_step(), GridVec::new(1, -1));
    }

    #[test]
    fn king_step_zero_stays_zero() {
        assert_eq!(GridVec::ZERO.king_step(), GridVec::ZERO);
    }

    #[test]
    fn king_step_unit_stays_same() {
        for dir in &GridVec::DIRECTIONS_8 {
            assert_eq!(dir.king_step(), *dir);
        }
    }

    // ─── Rotation tests ─────────────────────────────────────────

    #[test]
    fn rotate_90_cw_cardinal_directions() {
        // N → E → S → W → N (clockwise cycle)
        assert_eq!(GridVec::NORTH.rotate_90_cw(), GridVec::EAST);
        assert_eq!(GridVec::EAST.rotate_90_cw(), GridVec::SOUTH);
        assert_eq!(GridVec::SOUTH.rotate_90_cw(), GridVec::WEST);
        assert_eq!(GridVec::WEST.rotate_90_cw(), GridVec::NORTH);
    }

    #[test]
    fn rotate_90_ccw_cardinal_directions() {
        // N → W → S → E → N (counter-clockwise cycle)
        assert_eq!(GridVec::NORTH.rotate_90_ccw(), GridVec::WEST);
        assert_eq!(GridVec::WEST.rotate_90_ccw(), GridVec::SOUTH);
        assert_eq!(GridVec::SOUTH.rotate_90_ccw(), GridVec::EAST);
        assert_eq!(GridVec::EAST.rotate_90_ccw(), GridVec::NORTH);
    }

    #[test]
    fn rotate_cyclic_group_order_4() {
        // R⁴ = I: four CW rotations return to original.
        let v = GridVec::new(3, -7);
        let r1 = v.rotate_90_cw();
        let r2 = r1.rotate_90_cw();
        let r3 = r2.rotate_90_cw();
        let r4 = r3.rotate_90_cw();
        assert_eq!(r4, v, "R⁴ should equal identity");
    }

    #[test]
    fn rotate_cw_ccw_are_inverses() {
        let v = GridVec::new(5, -2);
        assert_eq!(v.rotate_90_cw().rotate_90_ccw(), v);
        assert_eq!(v.rotate_90_ccw().rotate_90_cw(), v);
    }

    #[test]
    fn rotate_preserves_squared_magnitude() {
        // Rotation is an isometry: |Rv|² = |v|².
        let v = GridVec::new(3, 4);
        let origin = GridVec::ZERO;
        assert_eq!(
            v.distance_squared(origin),
            v.rotate_90_cw().distance_squared(origin)
        );
    }

    #[test]
    fn rotate_zero_stays_zero() {
        assert_eq!(GridVec::ZERO.rotate_90_cw(), GridVec::ZERO);
        assert_eq!(GridVec::ZERO.rotate_90_ccw(), GridVec::ZERO);
    }

    // ─── MulAssign tests ────────────────────────────────────────

    #[test]
    fn mul_assign_scalar() {
        let mut v = GridVec::new(3, -4);
        v *= 2;
        assert_eq!(v, GridVec::new(6, -8));
    }

    #[test]
    fn mul_assign_zero() {
        let mut v = GridVec::new(5, 7);
        v *= 0;
        assert_eq!(v, GridVec::ZERO);
    }

    // ─── Bresenham line tests ───────────────────────────────────

    #[test]
    fn bresenham_single_point() {
        let p = GridVec::new(5, 5);
        let line = p.bresenham_line(p);
        assert_eq!(line, vec![p]);
    }

    #[test]
    fn bresenham_horizontal() {
        let a = GridVec::new(0, 0);
        let b = GridVec::new(4, 0);
        let line = a.bresenham_line(b);
        assert_eq!(line.len(), 5);
        assert_eq!(line[0], a);
        assert_eq!(line[4], b);
        // All points should have y=0.
        for p in &line {
            assert_eq!(p.y, 0);
        }
    }

    #[test]
    fn bresenham_vertical() {
        let a = GridVec::new(3, 1);
        let b = GridVec::new(3, 5);
        let line = a.bresenham_line(b);
        assert_eq!(line.len(), 5);
        assert_eq!(line[0], a);
        assert_eq!(line[4], b);
        for p in &line {
            assert_eq!(p.x, 3);
        }
    }

    #[test]
    fn bresenham_diagonal() {
        let a = GridVec::new(0, 0);
        let b = GridVec::new(3, 3);
        let line = a.bresenham_line(b);
        assert_eq!(line.len(), 4);
        for p in &line {
            assert_eq!(p.x, p.y, "Diagonal line should have x == y");
        }
    }

    #[test]
    fn bresenham_negative_direction() {
        let a = GridVec::new(5, 5);
        let b = GridVec::new(2, 3);
        let line = a.bresenham_line(b);
        assert_eq!(line[0], a);
        assert_eq!(*line.last().unwrap(), b);
    }

    #[test]
    fn bresenham_8_connected() {
        // Verify that consecutive points differ by at most 1 in each axis.
        let a = GridVec::new(0, 0);
        let b = GridVec::new(7, 3);
        let line = a.bresenham_line(b);
        for i in 1..line.len() {
            let dx = (line[i].x - line[i - 1].x).abs();
            let dy = (line[i].y - line[i - 1].y).abs();
            assert!(
                dx <= 1 && dy <= 1,
                "Step {i} not 8-connected: dx={dx}, dy={dy}"
            );
        }
    }

    #[test]
    fn bresenham_reverse_same_points() {
        // a→b and b→a should cover the same set of grid points.
        let a = GridVec::new(1, 2);
        let b = GridVec::new(8, 5);
        let forward = a.bresenham_line(b);
        let backward = b.bresenham_line(a);
        let mut fwd_set: Vec<GridVec> = forward.clone();
        let mut bwd_set: Vec<GridVec> = backward.clone();
        fwd_set.sort();
        bwd_set.sort();
        assert_eq!(
            fwd_set, bwd_set,
            "Forward and backward should cover same points"
        );
    }

    // ─── Neighbor tests ─────────────────────────────────────────

    #[test]
    fn cardinal_neighbors_count() {
        let p = GridVec::new(5, 5);
        assert_eq!(p.cardinal_neighbors().len(), 4);
    }

    #[test]
    fn cardinal_neighbors_at_distance_1() {
        let p = GridVec::new(5, 5);
        for n in &p.cardinal_neighbors() {
            assert_eq!(p.manhattan_distance(*n), 1);
        }
    }

    #[test]
    fn all_neighbors_count() {
        let p = GridVec::new(5, 5);
        assert_eq!(p.all_neighbors().len(), 8);
    }

    #[test]
    fn all_neighbors_at_chebyshev_distance_1() {
        let p = GridVec::new(5, 5);
        for n in &p.all_neighbors() {
            assert_eq!(p.chebyshev_distance(*n), 1);
        }
    }

    // ─── norm_squared tests ─────────────────────────────────────

    #[test]
    fn norm_squared_zero() {
        assert_eq!(GridVec::ZERO.norm_squared(), 0);
    }

    #[test]
    fn norm_squared_unit_vectors() {
        for dir in &GridVec::DIRECTIONS_4 {
            assert_eq!(dir.norm_squared(), 1);
        }
    }

    #[test]
    fn norm_squared_equals_dot_self() {
        let v = GridVec::new(3, 4);
        assert_eq!(v.norm_squared(), v.dot(v));
    }

    #[test]
    fn norm_squared_equals_distance_squared_to_zero() {
        let v = GridVec::new(7, -2);
        assert_eq!(v.norm_squared(), v.distance_squared(GridVec::ZERO));
    }

    #[test]
    fn norm_squared_pythagorean() {
        let v = GridVec::new(3, 4);
        assert_eq!(v.norm_squared(), 25); // 3² + 4² = 25
    }

    // ─── euclidean_distance tests ────────────────────────────────

    #[test]
    fn euclidean_distance_pythagorean() {
        let a = GridVec::new(0, 0);
        let b = GridVec::new(3, 4);
        assert!((a.euclidean_distance(b) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn euclidean_distance_symmetric() {
        let a = GridVec::new(1, 2);
        let b = GridVec::new(5, 7);
        assert!((a.euclidean_distance(b) - b.euclidean_distance(a)).abs() < 1e-10);
    }

    #[test]
    fn euclidean_distance_zero_self() {
        let v = GridVec::new(3, 4);
        assert_eq!(v.euclidean_distance(v), 0.0);
    }

    // ─── is_zero tests ──────────────────────────────────────────

    #[test]
    fn is_zero_true_for_zero() {
        assert!(GridVec::ZERO.is_zero());
    }

    #[test]
    fn is_zero_false_for_nonzero() {
        assert!(!GridVec::new(1, 0).is_zero());
        assert!(!GridVec::new(0, 1).is_zero());
        assert!(!GridVec::new(-1, -1).is_zero());
    }

    #[test]
    fn is_zero_consistent_with_eq() {
        let v = GridVec::new(3, -7);
        assert_eq!(v.is_zero(), v == GridVec::ZERO);
    }
}
