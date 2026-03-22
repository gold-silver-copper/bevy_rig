use crate::typedefs::{GraphicTriple, RatColor};
use crate::typeenums::{Floor, Props};
use crate::voxel::dim;

/// Trait for game elements that can be rendered as a GraphicTriple.
pub trait GraphicElement {
    fn symbol(&self) -> String;
    fn fg_color(&self) -> RatColor;
    fn bg_color(&self) -> RatColor;

    fn to_graphic_triple(&self) -> GraphicTriple {
        (self.symbol(), self.fg_color(), self.bg_color())
    }
}

impl GraphicElement for Floor {
    fn symbol(&self) -> String {
        match self {
            Floor::Gravel => ",".into(),
            Floor::Dirt => " ".into(),
            Floor::Grass => " ".into(),
            Floor::Sand => " ".into(),
            Floor::TallGrass => ";".into(),
            Floor::ScorchedEarth => ".".into(),
            Floor::WoodPlanks => " ".into(),
            Floor::Fire => "^".into(),
            Floor::Water => "~".into(),
            Floor::SandCloud => "*".into(),
            Floor::ShallowWater => "~".into(),
            Floor::DeepWater => "≈".into(),
            Floor::Beach => ".".into(),
            Floor::Bridge => "=".into(),
            Floor::Sidewalk => ".".into(),
            Floor::Rooftop => "^".into(),
            Floor::Plaza => ".".into(),
            Floor::Alley => " ".into(),
            Floor::StoneFloor => ".".into(),
            Floor::DirtRoad => " ".into(),
            Floor::BeachSand => ".".into(),
        }
    }

    fn fg_color(&self) -> RatColor {
        match self {
            Floor::Sand => RatColor::Rgb(180, 160, 120),
            Floor::Dirt => RatColor::Rgb(100, 78, 42),
            Floor::Gravel => RatColor::Rgb(110, 95, 75),
            Floor::Grass => RatColor::Rgb(50, 80, 40),
            Floor::TallGrass => RatColor::Rgb(60, 100, 45),
            Floor::ScorchedEarth => RatColor::Rgb(90, 40, 20),
            Floor::WoodPlanks => RatColor::Rgb(130, 95, 50),
            Floor::Fire => RatColor::Rgb(255, 140, 0),
            Floor::Water => RatColor::Rgb(80, 140, 200),
            Floor::SandCloud => RatColor::Rgb(210, 180, 120),
            Floor::ShallowWater => RatColor::Rgb(100, 170, 220),
            Floor::DeepWater => RatColor::Rgb(30, 80, 180),
            Floor::Beach => RatColor::Rgb(220, 200, 140),
            Floor::Bridge => RatColor::Rgb(140, 100, 55),
            Floor::Sidewalk => RatColor::Rgb(140, 120, 85),
            Floor::Rooftop => RatColor::Rgb(160, 110, 60),
            Floor::Plaza => RatColor::Rgb(170, 150, 110),
            Floor::Alley => RatColor::Rgb(70, 55, 35),
            Floor::StoneFloor => RatColor::Rgb(150, 145, 135),
            Floor::DirtRoad => RatColor::Rgb(110, 85, 50),
            Floor::BeachSand => RatColor::Rgb(210, 190, 130),
        }
    }

    fn bg_color(&self) -> RatColor {
        match self {
            Floor::Sand => RatColor::Rgb(160, 140, 100),
            Floor::Dirt => RatColor::Rgb(80, 62, 35),
            Floor::Gravel => RatColor::Rgb(85, 75, 60),
            Floor::Grass => RatColor::Rgb(35, 60, 28),
            Floor::TallGrass => RatColor::Rgb(40, 70, 30),
            Floor::ScorchedEarth => RatColor::Rgb(55, 30, 15),
            Floor::WoodPlanks => RatColor::Rgb(95, 70, 38),
            Floor::Fire => RatColor::Rgb(200, 60, 0),
            Floor::Water => RatColor::Rgb(40, 80, 140),
            Floor::SandCloud => RatColor::Rgb(170, 140, 90),
            Floor::ShallowWater => RatColor::Rgb(60, 120, 180),
            Floor::DeepWater => RatColor::Rgb(15, 40, 120),
            Floor::Beach => RatColor::Rgb(190, 170, 110),
            Floor::Bridge => RatColor::Rgb(100, 72, 40),
            Floor::Sidewalk => RatColor::Rgb(110, 95, 65),
            Floor::Rooftop => RatColor::Rgb(120, 80, 40),
            Floor::Plaza => RatColor::Rgb(140, 125, 90),
            Floor::Alley => RatColor::Rgb(45, 35, 22),
            Floor::StoneFloor => RatColor::Rgb(120, 115, 105),
            Floor::DirtRoad => RatColor::Rgb(88, 68, 38),
            Floor::BeachSand => RatColor::Rgb(180, 160, 100),
        }
    }
}

impl GraphicElement for Props {
    fn symbol(&self) -> String {
        match self {
            Props::Wall => "#".into(),
            Props::Tree => "T".into(),
            Props::Bush => "%".into(),
            Props::Rock => "o".into(),
            Props::DeadTree => "t".into(),
            Props::Bench => "H".into(),
            Props::Barrel => "0".into(),
            Props::Crate => "B".into(),
            Props::Cactus => "Y".into(),
            Props::HitchingPost => "F".into(),
            Props::WaterTrough => "U".into(),
            Props::Fence => "-".into(),
            Props::Table => "n".into(),
            Props::Chair => "h".into(),
            Props::Piano => "M".into(),
            Props::Sign => "]".into(),
            Props::HayBale => "&".into(),
            Props::Well => "O".into(),
            Props::Gallows => "P".into(),
            Props::WaterTower => "A".into(),
            Props::RailTrack => "=".into(),
            Props::Windmill => "X".into(),
            Props::LampPost => "i".into(),
            Props::StoneWall => "#".into(),
            Props::GunpowderBarrel => "0".into(),
            Props::Window => "~".into(),
        }
    }

    fn fg_color(&self) -> RatColor {
        match self {
            Props::Wall => RatColor::Rgb(120, 90, 45),
            Props::Tree => RatColor::Rgb(40, 100, 35),
            Props::Bush => RatColor::Rgb(55, 110, 40),
            Props::Rock => RatColor::Rgb(110, 105, 95),
            Props::DeadTree => RatColor::Rgb(90, 70, 45),
            Props::Bench => RatColor::Rgb(120, 80, 40),
            Props::Barrel => RatColor::Rgb(120, 80, 40),
            Props::Crate => RatColor::Rgb(130, 100, 50),
            Props::Cactus => RatColor::Rgb(50, 100, 40),
            Props::HitchingPost => RatColor::Rgb(105, 75, 38),
            Props::WaterTrough => RatColor::Rgb(65, 110, 145),
            Props::Fence => RatColor::Rgb(130, 100, 50),
            Props::Table => RatColor::Rgb(120, 80, 40),
            Props::Chair => RatColor::Rgb(105, 75, 38),
            Props::Piano => RatColor::Rgb(50, 45, 40),
            Props::Sign => RatColor::Rgb(150, 125, 70),
            Props::HayBale => RatColor::Rgb(200, 180, 80),
            Props::Well => RatColor::Rgb(130, 130, 130),
            Props::Gallows => RatColor::Rgb(100, 70, 35),
            Props::WaterTower => RatColor::Rgb(110, 80, 45),
            Props::RailTrack => RatColor::Rgb(90, 85, 80),
            Props::Windmill => RatColor::Rgb(140, 110, 60),
            Props::LampPost => RatColor::Rgb(80, 80, 90),
            Props::StoneWall => RatColor::Rgb(140, 135, 125),
            Props::GunpowderBarrel => RatColor::Rgb(80, 60, 40),
            Props::Window => RatColor::Rgb(150, 190, 220),
        }
    }

    fn bg_color(&self) -> RatColor {
        dim(self.fg_color(), 0.8)
    }
}
