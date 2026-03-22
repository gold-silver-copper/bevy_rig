use bevy::prelude::*;

use roguelike::{
    components::{Name, Player, Position},
    events::MoveIntent,
    map::TileMap,
    resources::{GameLog, WorldClock},
    systems::simulation::resolve_move_intents_system,
};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_message::<MoveIntent>();
    app.insert_resource(TileMap::demo());
    app.insert_resource(GameLog::default());
    app.insert_resource(WorldClock::default());
    app.add_systems(Update, resolve_move_intents_system);
    app
}

#[test]
fn player_moves_on_floor_tile() {
    let mut app = test_app();
    let player = app
        .world_mut()
        .spawn((Player, Name("Ranger".into()), Position::new(8, 11)))
        .id();

    app.world_mut().write_message(MoveIntent {
        entity: player,
        dx: 1,
        dy: 0,
    });
    app.update();

    let pos = app.world().get::<Position>(player).unwrap();
    assert_eq!(*pos, Position::new(9, 11));
}

#[test]
fn player_cannot_walk_into_wall() {
    let mut app = test_app();
    let player = app
        .world_mut()
        .spawn((Player, Name("Ranger".into()), Position::new(3, 8)))
        .id();

    app.world_mut().write_message(MoveIntent {
        entity: player,
        dx: 0,
        dy: -1,
    });
    app.update();

    let pos = app.world().get::<Position>(player).unwrap();
    assert_eq!(*pos, Position::new(3, 8));
}
