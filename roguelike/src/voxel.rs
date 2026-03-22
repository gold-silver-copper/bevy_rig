use crate::graphic_trait::GraphicElement;
use crate::typedefs::{GraphicTriple, RatColor};
use crate::typeenums::{Floor, Props};

/// A single cell in the game map grid.
///
/// Position is implicit from the `(row, col)` indices in the `GameMap`'s
/// 2D voxel array, so no position field is stored here.
#[derive(Clone, Debug)]
pub struct Voxel {
    pub floor: Option<Floor>,
    pub props: Option<Props>,
}

impl Voxel {
    /// Converts this voxel to a GraphicTriple based on visibility.
    /// Layers: floor → props. Unseen tiles are dimmed.
    #[inline]
    pub fn to_graphic(&self, visible: bool) -> GraphicTriple {
        let floor = match &self.floor {
            Some(fl) => fl.to_graphic_triple(),
            None => (" ".into(), RatColor::Black, RatColor::Black),
        };

        let plus_props: GraphicTriple = match &self.props {
            Some(prop) => (prop.symbol(), prop.fg_color(), floor.2),
            None => floor,
        };

        if visible {
            plus_props
        } else {
            let mut dimmed = plus_props;
            dimmed.1 = dim(dimmed.1, 0.3);
            dimmed.2 = dim(dimmed.2, 0.5);
            dimmed
        }
    }
}

/// Dims a color by a factor. Clamps RGB values to 0..=127.
pub fn dim(color: RatColor, factor: f32) -> RatColor {
    match color {
        RatColor::Rgb(r, g, b) => RatColor::Rgb(
            ((r as f32 * factor).clamp(0.0, 127.0)) as u8,
            ((g as f32 * factor).clamp(0.0, 127.0)) as u8,
            ((b as f32 * factor).clamp(0.0, 127.0)) as u8,
        ),
        _ => RatColor::Gray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typeenums::{Floor, Props};

    #[test]
    fn dim_rgb_reduces_values() {
        let color = RatColor::Rgb(200, 100, 50);
        let dimmed = dim(color, 0.5);
        assert_eq!(dimmed, RatColor::Rgb(100, 50, 25));
    }

    #[test]
    fn dim_rgb_clamps_to_127() {
        let color = RatColor::Rgb(255, 255, 255);
        let dimmed = dim(color, 1.0);
        assert_eq!(dimmed, RatColor::Rgb(127, 127, 127));
    }

    #[test]
    fn dim_non_rgb_returns_gray() {
        let dimmed = dim(RatColor::White, 0.5);
        assert_eq!(dimmed, RatColor::Gray);
    }

    #[test]
    fn voxel_to_graphic_visible_no_props() {
        let voxel = Voxel {
            floor: Some(Floor::Grass),
            props: None,
        };
        let graphic = voxel.to_graphic(true);
        // Should have the floor's symbol
        assert_eq!(graphic.0, " ");
    }

    #[test]
    fn voxel_to_graphic_visible_with_props() {
        let voxel = Voxel {
            floor: Some(Floor::Grass),
            props: Some(Props::Tree),
        };
        let graphic = voxel.to_graphic(true);
        // Props symbol overrides floor symbol
        assert_eq!(graphic.0, "T");
    }

    #[test]
    fn voxel_to_graphic_not_visible_is_dimmed() {
        let voxel = Voxel {
            floor: Some(Floor::Grass),
            props: None,
        };
        let visible = voxel.to_graphic(true);
        let dimmed = voxel.to_graphic(false);
        // Dimmed version should have different (darker) colors
        assert_ne!(visible.1, dimmed.1);
    }

    #[test]
    fn voxel_no_floor_shows_space() {
        let voxel = Voxel {
            floor: None,
            props: None,
        };
        let graphic = voxel.to_graphic(true);
        assert_eq!(graphic.0, " ");
    }
}
