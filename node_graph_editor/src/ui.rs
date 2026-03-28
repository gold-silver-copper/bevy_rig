use std::collections::{HashMap, HashSet};

use bevy::{
    ecs::hierarchy::ChildSpawnerCommands,
    math::Rot2,
    prelude::*,
    ui::{ComputedNode, RelativeCursorPosition},
};

use crate::graph::{
    DraggingWire, GraphEditorState, GraphNode, NodeId, NodeKind, NodeTemplate, PortAddress,
    PortDirection, PortType,
};

const SIDEBAR_WIDTH: f32 = 248.0;
const TOOLBAR_HEIGHT: f32 = 58.0;
const NODE_WIDTH: f32 = 320.0;
const NODE_HEADER_HEIGHT: f32 = 34.0;
const NODE_PADDING: f32 = 12.0;
const PORT_ROW_HEIGHT: f32 = 28.0;
const PORT_DOT_SIZE: f32 = 14.0;
const PORT_X_OFFSET: f32 = 22.0;
const WIRE_THICKNESS: f32 = 3.0;
const GRID_SPACING: f32 = 28.0;

pub struct NodeGraphEditorPlugin;

impl Plugin for NodeGraphEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GraphEditorState>()
            .init_resource::<GraphUiRegistry>()
            .add_systems(Startup, setup_editor_ui)
            .add_systems(
                Update,
                (
                    handle_editor_buttons,
                    sync_node_views,
                    update_node_view_state,
                    update_sidebar_text,
                    rebuild_canvas_overlay,
                ),
            );
    }
}

#[derive(Resource, Default)]
struct GraphUiRegistry {
    canvas: Option<Entity>,
    overlay_layer: Option<Entity>,
    node_layer: Option<Entity>,
    selection_text: Option<Entity>,
    status_text: Option<Entity>,
    node_views: HashMap<NodeId, Entity>,
    overlay_entities: Vec<Entity>,
}

#[derive(Component)]
struct CanvasSurface;

#[derive(Component)]
struct NodeView {
    id: NodeId,
}

#[derive(Component)]
struct NodeHeaderView {
    id: NodeId,
}

#[derive(Component)]
struct SidebarSelectionText;

#[derive(Component)]
struct ToolbarStatusText;

#[derive(Component, Clone, Copy)]
enum EditorAction {
    ResetDemo,
    AddNode(NodeTemplate),
}

#[derive(Component)]
struct EditorButton(EditorAction);

fn setup_editor_ui(mut commands: Commands, mut registry: ResMut<GraphUiRegistry>) {
    commands.spawn(Camera2d);
    let mut selection_text = None;
    let mut status_text = None;

    let root = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                ..default()
            },
            BackgroundColor(Color::srgb_u8(20, 21, 24)),
        ))
        .id();

    let sidebar = commands
        .spawn((
            Node {
                width: Val::Px(SIDEBAR_WIDTH),
                height: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(16.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                border: UiRect::right(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(Color::srgb_u8(26, 27, 30)),
            BorderColor {
                right: Color::srgb_u8(48, 50, 56),
                ..BorderColor::DEFAULT
            },
        ))
        .id();
    commands.entity(root).add_child(sidebar);

    commands.entity(sidebar).with_children(|parent| {
        parent.spawn((
            Text::new("Bevy Node Graph"),
            TextFont {
                font_size: 28.0,
                ..default()
            },
            TextColor(Color::srgb_u8(236, 238, 241)),
        ));

        parent.spawn((
            Text::new("ComfyUI-style graph editor MVP built with Bevy UI only."),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgb_u8(170, 175, 184)),
        ));

        let selection = parent
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    margin: UiRect::top(Val::Px(8.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb_u8(34, 36, 40)),
                BorderColor::all(Color::srgb_u8(58, 60, 66)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("Selection"),
                    TextFont {
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(248, 249, 251)),
                ));
                let text_entity = panel
                    .spawn((
                        Text::new("Nothing selected."),
                        TextFont {
                            font_size: 13.0,
                            ..default()
                        },
                        TextColor(Color::srgb_u8(185, 188, 194)),
                        SidebarSelectionText,
                    ))
                    .id();
                selection_text = Some(text_entity);
            })
            .id();
        let _ = selection;

        parent.spawn((
            Text::new("Add nodes"),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb_u8(230, 231, 235)),
        ));

        for template in NodeTemplate::ALL {
            spawn_editor_button(parent, template.label(), EditorAction::AddNode(template));
        }

        parent.spawn(Node {
            height: Val::Px(12.0),
            ..default()
        });

        spawn_editor_button(parent, "Reset Demo", EditorAction::ResetDemo);

        parent.spawn((
            Node {
                margin: UiRect::top(Val::Px(12.0)),
                ..default()
            },
            Text::new(
                "Controls\n\
                 • Drag node headers with the left mouse button\n\
                 • Middle-drag on the canvas to pan\n\
                 • Click one port, then another compatible port to connect\n\
                 • Click the canvas to clear selection or cancel a pending wire",
            ),
            TextFont {
                font_size: 13.0,
                ..default()
            },
            TextColor(Color::srgb_u8(155, 160, 168)),
        ));
    });

    let main = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgb_u8(20, 21, 24)),
        ))
        .id();
    commands.entity(root).add_child(main);

    let toolbar = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(TOOLBAR_HEIGHT),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::axes(Val::Px(18.0), Val::Px(12.0)),
                border: UiRect::bottom(Val::Px(1.0)),
                border_radius: BorderRadius::ZERO,
                ..default()
            },
            BackgroundColor(Color::srgb_u8(25, 26, 29)),
            BorderColor {
                bottom: Color::srgb_u8(49, 51, 58),
                ..BorderColor::DEFAULT
            },
        ))
        .id();
    commands.entity(main).add_child(toolbar);

    commands.entity(toolbar).with_children(|parent| {
        parent.spawn((
            Text::new("Native Bevy UI canvas"),
            TextFont {
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::srgb_u8(236, 238, 241)),
        ));
        let text_entity = parent
            .spawn((
                Text::new(""),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgb_u8(166, 171, 180)),
                ToolbarStatusText,
            ))
            .id();
        status_text = Some(text_entity);
    });

    let canvas = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Relative,
                overflow: Overflow::clip(),
                ..default()
            },
            CanvasSurface,
            RelativeCursorPosition::default(),
            Pickable {
                should_block_lower: false,
                is_hoverable: true,
            },
            BackgroundColor(Color::srgb_u8(24, 25, 29)),
        ))
        .observe(
            |mut click: On<Pointer<Click>>, mut graph: ResMut<GraphEditorState>| {
                if click.button == PointerButton::Primary {
                    click.propagate(false);
                    graph.selected_node = None;
                    graph.dragging_wire = None;
                }
            },
        )
        .observe(
            |mut drag: On<Pointer<Drag>>, mut graph: ResMut<GraphEditorState>| {
                if drag.button == PointerButton::Middle {
                    drag.propagate(false);
                    graph.pan += drag.delta;
                }
            },
        )
        .id();
    commands.entity(main).add_child(canvas);

    let overlay_layer = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    commands.entity(canvas).add_child(overlay_layer);

    let node_layer = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    commands.entity(canvas).add_child(node_layer);

    registry.canvas = Some(canvas);
    registry.overlay_layer = Some(overlay_layer);
    registry.node_layer = Some(node_layer);
    registry.selection_text = selection_text;
    registry.status_text = status_text;
}

fn spawn_editor_button(parent: &mut ChildSpawnerCommands, label: &str, action: EditorAction) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(36.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(9.0)),
                ..default()
            },
            EditorButton(action),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .with_child((
            Text::new(label),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgb_u8(232, 234, 238)),
        ));
}

fn handle_editor_buttons(
    mut graph: ResMut<GraphEditorState>,
    mut interactions: Query<
        (&Interaction, &EditorButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut background) in &mut interactions {
        background.0 = button_background(*interaction);

        if *interaction != Interaction::Pressed {
            continue;
        }

        match button.0 {
            EditorAction::ResetDemo => graph.reset_demo(),
            EditorAction::AddNode(template) => {
                let slot = graph.nodes.len() as f32;
                let spawn = Vec2::new(
                    180.0 - graph.pan.x + (slot.rem_euclid(3.0) * 34.0),
                    130.0 - graph.pan.y + ((slot / 3.0).floor() * 28.0),
                );
                graph.add_node(template.instantiate(), spawn);
            }
        }
    }
}

fn sync_node_views(
    mut commands: Commands,
    graph: Res<GraphEditorState>,
    mut registry: ResMut<GraphUiRegistry>,
) {
    let Some(node_layer) = registry.node_layer else {
        return;
    };

    let live_ids: HashSet<NodeId> = graph.node_ids().into_iter().collect();
    let stale_ids: Vec<NodeId> = registry
        .node_views
        .keys()
        .copied()
        .filter(|id| !live_ids.contains(id))
        .collect();

    for id in stale_ids {
        if let Some(entity) = registry.node_views.remove(&id) {
            commands.entity(entity).despawn();
        }
    }

    for node in &graph.nodes {
        let node_snapshot = node.clone();
        let node_id = node_snapshot.id;

        if registry.node_views.contains_key(&node_id) {
            continue;
        }

        let root = commands
            .spawn((
                NodeView { id: node_id },
                node_root_style(&node_snapshot, graph.pan),
                BackgroundColor(Color::srgb_u8(58, 58, 61)),
                BorderColor::all(Color::srgb_u8(74, 76, 82)),
                GlobalZIndex(4),
            ))
            .observe(
                move |mut click: On<Pointer<Click>>, mut graph: ResMut<GraphEditorState>| {
                    if click.button == PointerButton::Primary {
                        click.propagate(false);
                        graph.selected_node = Some(node_id);
                    }
                },
            )
            .id();
        commands.entity(node_layer).add_child(root);

        commands.entity(root).with_children(|parent| {
            parent
                .spawn((
                    NodeHeaderView { id: node_id },
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(NODE_HEADER_HEIGHT),
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        border: UiRect::bottom(Val::Px(1.0)),
                        border_radius: BorderRadius::ZERO,
                        ..default()
                    },
                    BackgroundColor(Color::srgb_u8(74, 74, 78)),
                    BorderColor {
                        bottom: Color::srgb_u8(88, 90, 96),
                        ..BorderColor::DEFAULT
                    },
                    Pickable {
                        should_block_lower: true,
                        is_hoverable: true,
                    },
                ))
                .observe(
                    move |mut click: On<Pointer<Click>>, mut graph: ResMut<GraphEditorState>| {
                        if click.button == PointerButton::Primary {
                            click.propagate(false);
                            graph.selected_node = Some(node_id);
                        }
                    },
                )
                .observe(
                    move |mut drag: On<Pointer<Drag>>, mut graph: ResMut<GraphEditorState>| {
                        if drag.button == PointerButton::Primary {
                            drag.propagate(false);
                            graph.selected_node = Some(node_id);
                            if let Some(node_state) = graph.node_mut(node_id) {
                                node_state.position += drag.delta;
                            }
                        }
                    },
                )
                .with_child((
                    Text::new(node_snapshot.kind.title()),
                    TextFont {
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(238, 240, 243)),
                ));

            parent.spawn((
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::all(Val::Px(NODE_PADDING)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(Color::srgb_u8(58, 58, 61)),
            ));
        });

        commands.entity(root).with_children(|parent| {
            let inputs = node_snapshot.kind.inputs();
            let outputs = node_snapshot.kind.outputs();
            let row_count = inputs.len().max(outputs.len()).max(1);

            parent
                .spawn(Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(NODE_PADDING), Val::Px(10.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|ports| {
                    for row in 0..row_count {
                        ports
                            .spawn(Node {
                                width: Val::Percent(100.0),
                                min_height: Val::Px(PORT_ROW_HEIGHT),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::SpaceBetween,
                                ..default()
                            })
                            .with_children(|row_parent| {
                                if let Some(spec) = inputs.get(row).copied() {
                                    row_parent
                                        .spawn(Node {
                                            display: Display::Flex,
                                            align_items: AlignItems::Center,
                                            column_gap: Val::Px(8.0),
                                            ..default()
                                        })
                                        .with_children(|group| {
                                            spawn_port_button(
                                                group,
                                                PortAddress {
                                                    node: node_id,
                                                    direction: PortDirection::Input,
                                                    index: row,
                                                },
                                                spec.ty,
                                            );
                                            group.spawn((
                                                Text::new(spec.name),
                                                TextFont {
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb_u8(205, 208, 214)),
                                            ));
                                        });
                                } else {
                                    row_parent.spawn(Node {
                                        width: Val::Px(110.0),
                                        ..default()
                                    });
                                }

                                if let Some(spec) = outputs.get(row).copied() {
                                    row_parent
                                        .spawn(Node {
                                            display: Display::Flex,
                                            align_items: AlignItems::Center,
                                            column_gap: Val::Px(8.0),
                                            ..default()
                                        })
                                        .with_children(|group| {
                                            group.spawn((
                                                Text::new(spec.name),
                                                TextFont {
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb_u8(205, 208, 214)),
                                            ));
                                            spawn_port_button(
                                                group,
                                                PortAddress {
                                                    node: node_id,
                                                    direction: PortDirection::Output,
                                                    index: row,
                                                },
                                                spec.ty,
                                            );
                                        });
                                } else {
                                    row_parent.spawn(Node {
                                        width: Val::Px(110.0),
                                        ..default()
                                    });
                                }
                            });
                    }
                });

            parent
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        padding: UiRect::all(Val::Px(NODE_PADDING)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        border: UiRect::top(Val::Px(1.0)),
                        border_radius: BorderRadius::ZERO,
                        ..default()
                    },
                    BorderColor {
                        top: Color::srgb_u8(77, 79, 86),
                        ..BorderColor::DEFAULT
                    },
                ))
                .with_children(|body| {
                    for line in node_body_lines(&node_snapshot.kind) {
                        body.spawn((
                            Text::new(line),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(Color::srgb_u8(176, 181, 190)),
                        ));
                    }
                });
        });

        registry.node_views.insert(node_id, root);
    }
}

fn spawn_port_button(parent: &mut ChildSpawnerCommands, address: PortAddress, ty: PortType) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(PORT_DOT_SIZE),
                height: Val::Px(PORT_DOT_SIZE),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(PORT_DOT_SIZE)),
                ..default()
            },
            BackgroundColor(port_color(ty)),
            BorderColor::all(Color::BLACK),
        ))
        .observe(
            move |mut click: On<Pointer<Click>>, mut graph: ResMut<GraphEditorState>| {
                if click.button != PointerButton::Primary {
                    return;
                }

                click.propagate(false);
                graph.selected_node = Some(address.node);

                if let Some(dragging) = graph.dragging_wire {
                    if dragging.from == address {
                        graph.dragging_wire = None;
                        return;
                    }

                    if graph.connect(dragging.from, address) {
                        graph.dragging_wire = None;
                    } else {
                        graph.dragging_wire = graph.port_spec(address).map(|spec| DraggingWire {
                            from: address,
                            ty: spec.ty,
                        });
                    }
                } else if let Some(spec) = graph.port_spec(address) {
                    graph.dragging_wire = Some(DraggingWire {
                        from: address,
                        ty: spec.ty,
                    });
                }
            },
        );
}

fn update_node_view_state(
    graph: Res<GraphEditorState>,
    mut nodes: Query<(&NodeView, &mut Node, &mut BorderColor, &mut GlobalZIndex)>,
    mut headers: Query<(&NodeHeaderView, &mut BackgroundColor)>,
) {
    for (view, mut node_style, mut border, mut z_index) in &mut nodes {
        let Some(node) = graph.node(view.id) else {
            continue;
        };

        *node_style = node_root_style(node, graph.pan);
        let selected = graph.selected_node == Some(view.id);
        border.set_all(if selected {
            Color::srgb_u8(248, 205, 88)
        } else {
            Color::srgb_u8(74, 76, 82)
        });
        z_index.0 = if selected { 16 } else { 4 };
    }

    for (header, mut color) in &mut headers {
        color.0 = if graph.selected_node == Some(header.id) {
            Color::srgb_u8(90, 90, 96)
        } else {
            Color::srgb_u8(74, 74, 78)
        };
    }
}

fn update_sidebar_text(
    graph: Res<GraphEditorState>,
    registry: Res<GraphUiRegistry>,
    mut texts: Query<&mut Text>,
) {
    if let Some(entity) = registry.selection_text {
        if let Ok(mut text) = texts.get_mut(entity) {
            text.0 = selection_summary(&graph);
        }
    }

    if let Some(entity) = registry.status_text {
        if let Ok(mut text) = texts.get_mut(entity) {
            let pending = if let Some(dragging) = graph.dragging_wire {
                let direction = match dragging.from.direction {
                    PortDirection::Input => "input",
                    PortDirection::Output => "output",
                };
                format!(
                    "Pending wire: {} port {}:{}",
                    direction, dragging.from.node, dragging.from.index
                )
            } else {
                "No pending connection".to_string()
            };
            text.0 = format!(
                "{}  •  nodes={}  •  edges={}  •  middle-drag to pan",
                pending,
                graph.nodes.len(),
                graph.edges.len()
            );
        }
    }
}

fn rebuild_canvas_overlay(
    mut commands: Commands,
    graph: Res<GraphEditorState>,
    mut registry: ResMut<GraphUiRegistry>,
    canvas_query: Query<(&ComputedNode, &RelativeCursorPosition), With<CanvasSurface>>,
) {
    let (Some(overlay_layer), Some(canvas)) = (registry.overlay_layer, registry.canvas) else {
        return;
    };

    let Ok((canvas_node, cursor)) = canvas_query.get(canvas) else {
        return;
    };

    for entity in registry.overlay_entities.drain(..) {
        commands.entity(entity).despawn();
    }

    let canvas_size = canvas_node.size();

    spawn_grid_lines(
        &mut commands,
        overlay_layer,
        &mut registry.overlay_entities,
        canvas_size,
        graph.pan,
    );

    for edge in &graph.edges {
        let Some(start) = port_center(&graph, edge.from) else {
            continue;
        };
        let Some(end) = port_center(&graph, edge.to) else {
            continue;
        };
        let color = graph
            .port_spec(edge.from)
            .map(|spec| port_color(spec.ty))
            .unwrap_or(Color::srgb_u8(160, 164, 172));
        spawn_wire_segment(
            &mut commands,
            overlay_layer,
            &mut registry.overlay_entities,
            start,
            end,
            color,
            WIRE_THICKNESS,
        );
    }

    if let Some(dragging) = graph.dragging_wire {
        if let Some(pointer) = cursor.normalized {
            let pointer = (pointer + Vec2::splat(0.5)) * canvas_size;
            if let Some(start) = port_center(&graph, dragging.from) {
                spawn_wire_segment(
                    &mut commands,
                    overlay_layer,
                    &mut registry.overlay_entities,
                    start,
                    pointer,
                    port_color(dragging.ty),
                    WIRE_THICKNESS,
                );
            }
        }
    }
}

fn spawn_grid_lines(
    commands: &mut Commands,
    overlay_layer: Entity,
    overlay_entities: &mut Vec<Entity>,
    canvas_size: Vec2,
    pan: Vec2,
) {
    let offset_x = pan.x.rem_euclid(GRID_SPACING);
    let offset_y = pan.y.rem_euclid(GRID_SPACING);

    let columns = ((canvas_size.x / GRID_SPACING).ceil() as i32) + 2;
    let rows = ((canvas_size.y / GRID_SPACING).ceil() as i32) + 2;

    for column in 0..columns {
        let x = offset_x + column as f32 * GRID_SPACING;
        let entity = commands
            .spawn(grid_line_node(
                Vec2::new(x, canvas_size.y * 0.5),
                Vec2::new(1.0, canvas_size.y),
                if column.rem_euclid(4) == 0 {
                    Color::srgb_u8(43, 45, 50)
                } else {
                    Color::srgb_u8(34, 36, 40)
                },
            ))
            .id();
        commands.entity(overlay_layer).add_child(entity);
        overlay_entities.push(entity);
    }

    for row in 0..rows {
        let y = offset_y + row as f32 * GRID_SPACING;
        let entity = commands
            .spawn(grid_line_node(
                Vec2::new(canvas_size.x * 0.5, y),
                Vec2::new(canvas_size.x, 1.0),
                if row.rem_euclid(4) == 0 {
                    Color::srgb_u8(43, 45, 50)
                } else {
                    Color::srgb_u8(34, 36, 40)
                },
            ))
            .id();
        commands.entity(overlay_layer).add_child(entity);
        overlay_entities.push(entity);
    }
}

fn spawn_wire_segment(
    commands: &mut Commands,
    overlay_layer: Entity,
    overlay_entities: &mut Vec<Entity>,
    start: Vec2,
    end: Vec2,
    color: Color,
    thickness: f32,
) {
    let delta = end - start;
    let length = delta.length();
    if length < 1.0 {
        return;
    }

    let center = (start + end) * 0.5;
    let angle = delta.y.atan2(delta.x);
    let entity = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(center.x - length * 0.5),
                top: Val::Px(center.y - thickness * 0.5),
                width: Val::Px(length),
                height: Val::Px(thickness),
                border_radius: BorderRadius::all(Val::Px(thickness * 0.5)),
                ..default()
            },
            UiTransform::from_rotation(Rot2::radians(angle)),
            BackgroundColor(color),
            GlobalZIndex(2),
            Pickable::IGNORE,
        ))
        .id();
    commands.entity(overlay_layer).add_child(entity);
    overlay_entities.push(entity);
}

fn grid_line_node(center: Vec2, size: Vec2, color: Color) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(center.x - size.x * 0.5),
            top: Val::Px(center.y - size.y * 0.5),
            width: Val::Px(size.x.max(1.0)),
            height: Val::Px(size.y.max(1.0)),
            ..default()
        },
        BackgroundColor(color),
        Pickable::IGNORE,
    )
}

fn node_root_style(node: &GraphNode, pan: Vec2) -> Node {
    let size = node_dimensions(&node.kind);
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(node.position.x + pan.x),
        top: Val::Px(node.position.y + pan.y),
        width: Val::Px(size.x),
        min_height: Val::Px(size.y),
        flex_direction: FlexDirection::Column,
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(12.0)),
        ..default()
    }
}

fn node_dimensions(kind: &NodeKind) -> Vec2 {
    let row_count = kind.inputs().len().max(kind.outputs().len()).max(1) as f32;
    let body_rows = node_body_lines(kind).len() as f32;
    let height = NODE_HEADER_HEIGHT
        + 16.0
        + (row_count * (PORT_ROW_HEIGHT + 4.0))
        + 12.0
        + (body_rows * 20.0);
    Vec2::new(NODE_WIDTH, height.max(160.0))
}

fn port_center(graph: &GraphEditorState, address: PortAddress) -> Option<Vec2> {
    let node = graph.node(address.node)?;
    let row = address.index as f32;
    let y =
        node.position.y + graph.pan.y + NODE_HEADER_HEIGHT + 26.0 + row * (PORT_ROW_HEIGHT + 4.0);
    let x = match address.direction {
        PortDirection::Input => node.position.x + graph.pan.x + PORT_X_OFFSET,
        PortDirection::Output => node.position.x + graph.pan.x + NODE_WIDTH - PORT_X_OFFSET,
    };
    Some(Vec2::new(x, y))
}

fn node_body_lines(kind: &NodeKind) -> Vec<String> {
    match kind {
        NodeKind::LoadCheckpoint { checkpoint } => vec![
            "Outputs: model, clip, vae".to_string(),
            format!("ckpt_name = {}", checkpoint),
        ],
        NodeKind::ClipTextEncode { text, .. } => {
            let preview = if text.len() > 88 {
                format!("{}…", &text[..88])
            } else {
                text.clone()
            };
            vec!["conditioning prompt".to_string(), preview]
        }
        NodeKind::EmptyLatentImage {
            width,
            height,
            batch_size,
        } => vec![
            format!("width = {}", width),
            format!("height = {}", height),
            format!("batch = {}", batch_size),
        ],
        NodeKind::KSampler { seed, steps, cfg } => vec![
            format!("seed = {}", seed),
            format!("steps = {}", steps),
            format!("cfg = {:.1}", cfg),
        ],
        NodeKind::VaeDecode => vec!["latent -> image".to_string()],
        NodeKind::SaveImage { filename_prefix } => {
            vec![format!("filename_prefix = {}", filename_prefix)]
        }
    }
}

fn selection_summary(graph: &GraphEditorState) -> String {
    let Some(selected) = graph.selected_node else {
        return "Nothing selected.\n\nUse the left palette to add nodes, then drag their headers around the canvas.".into();
    };
    let Some(node) = graph.node(selected) else {
        return "Nothing selected.".into();
    };

    let incoming = graph
        .edges
        .iter()
        .filter(|edge| edge.to.node == selected)
        .count();
    let outgoing = graph
        .edges
        .iter()
        .filter(|edge| edge.from.node == selected)
        .count();

    format!(
        "{}\n\nposition: {:.0}, {:.0}\ninputs: {}\noutputs: {}\nincoming edges: {}\noutgoing edges: {}",
        node.kind.title(),
        node.position.x,
        node.position.y,
        node.kind.inputs().len(),
        node.kind.outputs().len(),
        incoming,
        outgoing
    )
}

fn button_background(interaction: Interaction) -> Color {
    match interaction {
        Interaction::Pressed => Color::srgb_u8(84, 88, 96),
        Interaction::Hovered => Color::srgb_u8(62, 65, 71),
        Interaction::None => Color::srgb_u8(46, 48, 54),
    }
}

fn port_color(port_type: PortType) -> Color {
    match port_type {
        PortType::Model => Color::srgb_u8(183, 149, 255),
        PortType::Clip => Color::srgb_u8(245, 211, 61),
        PortType::Vae => Color::srgb_u8(255, 114, 132),
        PortType::Conditioning => Color::srgb_u8(255, 175, 87),
        PortType::Latent => Color::srgb_u8(224, 128, 255),
        PortType::Image => Color::srgb_u8(109, 193, 255),
    }
}
