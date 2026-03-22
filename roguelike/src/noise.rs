/// Deterministic procedural noise for terrain generation.
///
/// Uses a hash-based value noise with fractal Brownian motion (fBm) layering.
/// No external dependencies — the hash function is a variant of the
/// Squirrel3 noise hash (GDC 2017), which produces excellent distribution.
///
/// Seed type for reproducible generation.
pub type NoiseSeed = u64;

/// Squirrel3 positional noise hash.
///
/// Maps an integer position to a pseudo-random `u32`. The bit-mixing
/// constants are chosen to maximize avalanche (every input bit affects
/// every output bit with probability ≈ 0.5).
fn squirrel3(position: i32, seed: u64) -> u32 {
    const NOISE1: u64 = 0xB5297A4D; // large prime with good bit-spread
    const NOISE2: u64 = 0x68E31DA4;
    const NOISE3: u64 = 0x1B56C4E9;

    let mut mangled = position as u64;
    mangled = mangled.wrapping_mul(NOISE1);
    mangled = mangled.wrapping_add(seed);
    mangled ^= mangled >> 8;
    mangled = mangled.wrapping_add(NOISE2);
    mangled ^= mangled << 8;
    mangled = mangled.wrapping_mul(NOISE3);
    mangled ^= mangled >> 8;
    mangled as u32
}

/// 2D noise: combine x and y into a single position before hashing.
fn noise2d(x: i32, y: i32, seed: u64) -> u32 {
    // Combine coordinates via a large prime stride to decorrelate axes.
    let position = x.wrapping_add(y.wrapping_mul(198491));
    squirrel3(position, seed)
}

/// Returns a noise value in [0.0, 1.0) for the given grid coordinate.
pub fn value_noise(x: i32, y: i32, seed: NoiseSeed) -> f64 {
    noise2d(x, y, seed) as f64 / u32::MAX as f64
}

/// Smooth value noise with bilinear interpolation.
///
/// Samples the four lattice corners surrounding the fractional position
/// and interpolates with a smooth Hermite curve (3t² − 2t³) to
/// eliminate grid-axis artifacts.
pub fn smooth_noise(fx: f64, fy: f64, seed: NoiseSeed) -> f64 {
    let x0 = fx.floor() as i32;
    let y0 = fy.floor() as i32;

    // Fractional part within the lattice cell.
    let tx = fx - fx.floor();
    let ty = fy - fy.floor();

    // Smoothstep: eliminates first-derivative discontinuities.
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);

    // Four lattice corner values.
    let n00 = value_noise(x0, y0, seed);
    let n10 = value_noise(x0 + 1, y0, seed);
    let n01 = value_noise(x0, y0 + 1, seed);
    let n11 = value_noise(x0 + 1, y0 + 1, seed);

    // Bilinear interpolation with smoothstep.
    let nx0 = n00 + sx * (n10 - n00);
    let nx1 = n01 + sx * (n11 - n01);
    nx0 + sy * (nx1 - nx0)
}

/// Fractal Brownian Motion (fBm) — layers multiple octaves of smooth noise.
///
/// Each successive octave doubles the frequency (`lacunarity = 2.0`)
/// and halves the amplitude (`persistence = 0.5`), producing the
/// characteristic 1/f self-similar pattern found in natural terrain.
///
/// - `octaves`: number of noise layers (4 is a good default).
/// - `frequency`: base spatial frequency (lower = broader features).
/// - `persistence`: amplitude decay per octave.
/// - `seed`: deterministic seed.
pub fn fbm(x: f64, y: f64, octaves: u32, frequency: f64, persistence: f64, seed: NoiseSeed) -> f64 {
    let mut value = 0.0;
    let mut amplitude = 1.0;
    let mut freq = frequency;
    let mut max_amplitude = 0.0;

    for i in 0..octaves {
        // Offset each octave's seed so the layers aren't correlated.
        let octave_seed = seed.wrapping_add(i as u64 * 31);
        value += amplitude * smooth_noise(x * freq, y * freq, octave_seed);
        max_amplitude += amplitude;
        amplitude *= persistence;
        freq *= 2.0; // lacunarity
    }

    // Normalize to [0, 1].
    value / max_amplitude
}

/// Default per-tile color noise range (±).
/// Each RGB channel is shifted by at most this many units.
pub const TILE_COLOR_NOISE_RANGE: i16 = 2;

/// Applies deterministic per-tile color noise to an RGB color.
///
/// Uses a cheap hash of the tile's `(x, y)` world coordinates to produce
/// a stable offset in `[-range, +range]` for each RGB channel.  The noise
/// is identical every frame for the same tile, so it never flickers.
///
/// Call this as the **final** step before rendering — after lighting,
/// faction tinting, fog-of-war, and any other color logic.
#[inline]
pub fn tile_color_noise(r: u8, g: u8, b: u8, x: i32, y: i32, range: i16) -> (u8, u8, u8) {
    if range == 0 {
        return (r, g, b);
    }
    // Single hash producing enough bits for all three channels.
    let h = noise2d(x, y, 0xA3B1_C5D7);

    // Map to [-range, +range] using unsigned modulo to avoid sign issues.
    let span = (2 * range + 1) as u32;
    let dr = (h % span) as i16 - range;
    let dg = ((h >> 11) % span) as i16 - range;
    let db = ((h >> 22) % span) as i16 - range;

    let clamp = |v: i16| v.clamp(0, 255) as u8;
    (
        clamp(r as i16 + dr),
        clamp(g as i16 + dg),
        clamp(b as i16 + db),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_noise_deterministic() {
        let a = value_noise(10, 20, 42);
        let b = value_noise(10, 20, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn value_noise_in_range() {
        for x in -10..10 {
            for y in -10..10 {
                let v = value_noise(x, y, 42);
                assert!(
                    (0.0..1.0).contains(&v),
                    "value_noise({x},{y}) = {v} out of range"
                );
            }
        }
    }

    #[test]
    fn value_noise_different_positions_vary() {
        let a = value_noise(0, 0, 42);
        let b = value_noise(100, 100, 42);
        assert_ne!(a, b);
    }

    #[test]
    fn value_noise_different_seeds_vary() {
        let a = value_noise(5, 5, 0);
        let b = value_noise(5, 5, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn smooth_noise_in_range() {
        for i in 0..20 {
            let v = smooth_noise(i as f64 * 0.5, i as f64 * 0.3, 42);
            assert!((0.0..=1.0).contains(&v), "smooth_noise out of range: {v}");
        }
    }

    #[test]
    fn fbm_in_range() {
        for i in 0..20 {
            let v = fbm(i as f64, i as f64, 4, 0.1, 0.5, 42);
            assert!((0.0..=1.0).contains(&v), "fbm out of range: {v}");
        }
    }

    #[test]
    fn fbm_deterministic() {
        let a = fbm(5.5, 3.3, 4, 0.1, 0.5, 42);
        let b = fbm(5.5, 3.3, 4, 0.1, 0.5, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn tile_color_noise_deterministic() {
        let a = tile_color_noise(100, 100, 100, 10, 20, 2);
        let b = tile_color_noise(100, 100, 100, 10, 20, 2);
        assert_eq!(a, b);
    }

    #[test]
    fn tile_color_noise_in_range() {
        for x in -5..5 {
            for y in -5..5 {
                let (r, g, b) = tile_color_noise(100, 100, 100, x, y, 2);
                assert!(
                    (98..=102).contains(&r),
                    "r={r} out of ±2 range for ({x},{y})"
                );
                assert!(
                    (98..=102).contains(&g),
                    "g={g} out of ±2 range for ({x},{y})"
                );
                assert!(
                    (98..=102).contains(&b),
                    "b={b} out of ±2 range for ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn tile_color_noise_clamps_at_boundaries() {
        // Near 0: should not underflow
        let (r, _, _) = tile_color_noise(0, 128, 255, 5, 5, 2);
        assert!(r <= 2, "r={r} should be clamped near 0");
        // Near 255: should not overflow
        let (_, _, b) = tile_color_noise(0, 128, 255, 5, 5, 2);
        assert!(b >= 253, "b={b} should be clamped near 255");
    }

    #[test]
    fn tile_color_noise_different_positions_vary() {
        let a = tile_color_noise(100, 100, 100, 0, 0, 2);
        let b = tile_color_noise(100, 100, 100, 50, 50, 2);
        // Very unlikely all 3 channels are identical for different positions
        assert!(
            a != b,
            "Different positions should typically produce different noise"
        );
    }

    #[test]
    fn tile_color_noise_zero_range_is_identity() {
        let (r, g, b) = tile_color_noise(100, 150, 200, 42, 99, 0);
        assert_eq!(r, 100);
        assert_eq!(g, 150);
        assert_eq!(b, 200);
    }
}
