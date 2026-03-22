/// Floor tile types for the game map.
#[derive(Clone, Debug, PartialEq)]
pub enum Floor {
    Gravel,
    Sand,
    Dirt,
    Grass,
    TallGrass,
    ScorchedEarth,
    WoodPlanks,
    /// Ground that is actively burning.
    Fire,
    /// Spilled water from a destroyed water trough.
    Water,
    /// Persistent sand/smoke cloud that blocks line of sight.
    /// Placed by sand throws and gun smoke; ticks down per game turn.
    SandCloud,
    /// Shallow river water — impassable (blocks all movement).
    ShallowWater,
    /// Deep river water — impassable (blocks all movement).
    DeepWater,
    /// Sandy beach along the river banks.
    Beach,
    /// Bridge over the river — normal movement speed.
    Bridge,
    /// Packed-dirt sidewalk flanking carriage roads.
    Sidewalk,
    /// Accessible rooftop tile — reached via interior stairs or ladders.
    Rooftop,
    /// Open plaza/market square — exposed killzone.
    Plaza,
    /// Narrow alley between buildings — ambush terrain.
    Alley,
    /// Stone floor — used in churches, missions, and other stone buildings.
    StoneFloor,
    /// Packed-dirt carriage road — distinct from plain Dirt so placement
    /// checks can reject road tiles unambiguously. Nothing may spawn here.
    DirtRoad,
    /// Sandy beach buffer along the river — distinct from plain Sand.
    /// Nothing may spawn here.
    BeachSand,
}

/// Props (obstacles/structures) placed on tiles.
#[derive(Clone, Debug, PartialEq)]
pub enum Props {
    Wall,
    Tree,
    Bush,
    Rock,
    DeadTree,
    Bench,
    Barrel,
    Crate,
    Cactus,
    HitchingPost,
    WaterTrough,
    Fence,
    Table,
    Chair,
    Piano,
    Sign,
    /// A bale of hay — blocks movement but not vision, and is flammable.
    HayBale,
    /// Stone well — blocks movement, source of water.
    Well,
    /// Gallows post — blocks movement and vision.
    Gallows,
    /// Wooden water tower leg — blocks movement and vision.
    WaterTower,
    /// Railroad track — passable, does not block vision.
    RailTrack,
    /// Windmill blade/structure — blocks movement and vision.
    Windmill,
    /// Iron lamp post — blocks movement but not vision.
    LampPost,
    /// Stone wall — blocks movement and vision, indestructible, not flammable.
    StoneWall,
    /// Gunpowder barrel — explodes when destroyed, causing fire in a radius.
    GunpowderBarrel,
    /// Window — blocks movement but allows vision and projectiles through.
    Window,
}

impl Props {
    /// Returns `true` if this prop is any kind of wall.
    #[inline]
    pub fn is_wall(&self) -> bool {
        matches!(self, Props::Wall | Props::StoneWall)
    }

    /// Returns `true` if this prop blocks entity movement.
    /// Most solid objects block movement; low/open objects like fences and
    /// water troughs allow passage. Windows block movement but not projectiles.
    pub fn blocks_movement(&self) -> bool {
        !matches!(self, Props::Fence | Props::WaterTrough | Props::RailTrack)
    }

    /// Returns `true` if this prop blocks projectiles (bullets).
    /// Windows allow projectiles through; all other movement-blocking props stop them.
    pub fn blocks_projectiles(&self) -> bool {
        self.blocks_movement() && !matches!(self, Props::Window)
    }

    /// Returns `true` if this prop blocks line-of-sight.
    /// Tall opaque objects block vision; short or transparent objects do not.
    pub fn blocks_vision(&self) -> bool {
        match self {
            // Short/open objects: you can see over/through them
            Props::Fence
            | Props::WaterTrough
            | Props::Bush
            | Props::Bench
            | Props::Chair
            | Props::HayBale
            | Props::Sign
            | Props::RailTrack
            | Props::LampPost
            | Props::Barrel
            | Props::Crate
            | Props::Table
            | Props::HitchingPost
            | Props::Rock
            | Props::Cactus
            | Props::GunpowderBarrel
            | Props::Window => false,
            _ => true,
        }
    }

    /// Returns the maximum health for this prop. Indestructible props return i32::MAX.
    pub fn max_health(&self) -> i32 {
        match self {
            Props::Rock | Props::Well | Props::StoneWall => i32::MAX, // indestructible
            Props::Wall => 45,
            Props::Tree
            | Props::DeadTree
            | Props::Gallows
            | Props::WaterTower
            | Props::Windmill => 30,
            Props::Piano => 25,
            Props::Barrel | Props::Crate | Props::Table | Props::Bench => 15,
            Props::Chair | Props::Sign | Props::Fence | Props::HayBale => 10,
            Props::Bush
            | Props::Cactus
            | Props::LampPost
            | Props::HitchingPost
            | Props::WaterTrough => 20,
            Props::GunpowderBarrel => 10,
            Props::Window => 5,
            Props::RailTrack => i32::MAX,
        }
    }

    /// Returns `true` if this prop can be set on fire (destroyed by fire).
    pub fn is_flammable(&self) -> bool {
        matches!(
            self,
            Props::Wall
                | Props::Tree
                | Props::DeadTree
                | Props::Bush
                | Props::Barrel
                | Props::Crate
                | Props::Table
                | Props::Chair
                | Props::Piano
                | Props::Bench
                | Props::HayBale
                | Props::Sign
                | Props::Fence
                | Props::Gallows
                | Props::WaterTower
                | Props::Windmill
                | Props::GunpowderBarrel
                | Props::Window
        )
    }
}

impl std::fmt::Display for Props {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Props::Wall => write!(f, "Wall"),
            Props::Tree => write!(f, "Tree"),
            Props::Bush => write!(f, "Bush"),
            Props::Rock => write!(f, "Rock"),
            Props::DeadTree => write!(f, "Dead Tree"),
            Props::Bench => write!(f, "Bench"),
            Props::Barrel => write!(f, "Barrel"),
            Props::Crate => write!(f, "Crate"),
            Props::Cactus => write!(f, "Cactus"),
            Props::HitchingPost => write!(f, "Hitch Post"),
            Props::WaterTrough => write!(f, "Water Trgh"),
            Props::Fence => write!(f, "Fence"),
            Props::Table => write!(f, "Table"),
            Props::Chair => write!(f, "Chair"),
            Props::Piano => write!(f, "Piano"),
            Props::Sign => write!(f, "Sign"),
            Props::HayBale => write!(f, "Hay Bale"),
            Props::Well => write!(f, "Well"),
            Props::Gallows => write!(f, "Gallows"),
            Props::WaterTower => write!(f, "Water Twr"),
            Props::RailTrack => write!(f, "Rail Track"),
            Props::Windmill => write!(f, "Windmill"),
            Props::LampPost => write!(f, "Lamp Post"),
            Props::StoneWall => write!(f, "Stone Wall"),
            Props::GunpowderBarrel => write!(f, "Gunpowder"),
            Props::Window => write!(f, "Window"),
        }
    }
}
