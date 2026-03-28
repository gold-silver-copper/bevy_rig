use std::collections::HashMap;

use bevy::{
    ecs::hierarchy::ChildSpawnerCommands,
    input::{
        ButtonState,
        keyboard::{KeyCode, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
    },
    math::Rot2,
    prelude::*,
    ui::{ComputedNode, RelativeCursorPosition},
};

use crate::{
    graph::{
        DraggingWire, GraphEditorState, GraphNode, NodeId, NodeKind, NodeTemplate, PortAddress,
        PortDirection, PortType,
    },
    runtime::{RigEditorRuntime, compile_selected_agent_run},
};

const SIDEBAR_WIDTH: f32 = 340.0;
const TOOLBAR_HEIGHT: f32 = 58.0;
const NODE_WIDTH: f32 = 340.0;
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
            .init_resource::<TextEditingState>()
            .add_systems(Startup, setup_editor_ui)
            .add_systems(
                Update,
                (
                    handle_editor_buttons,
                    handle_node_buttons,
                    handle_text_edit_input,
                    handle_canvas_zoom,
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
    status_text: Option<Entity>,
    node_views: HashMap<NodeId, Entity>,
    overlay_entities: Vec<Entity>,
    last_graph_revision: u64,
    last_edit_revision: u64,
}

#[derive(Resource, Default)]
struct TextEditingState {
    target: Option<NodeId>,
    buffer: String,
    revision: u64,
}

impl TextEditingState {
    fn clear(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.target = None;
        self.buffer.clear();
    }

    fn begin_if_needed(&mut self, target: NodeId, value: &str) {
        if self.target == Some(target) {
            return;
        }
        self.target = Some(target);
        self.buffer = value.to_string();
        self.revision = self.revision.wrapping_add(1);
    }

    fn mark_changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
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
struct ToolbarStatusText;

#[derive(Component, Clone, Copy)]
enum EditorAction {
    ResetDemo,
    RunSelectedAgent,
    RefreshOllama,
    AddNode(NodeTemplate),
}

#[derive(Component)]
struct EditorButton(EditorAction);

#[derive(Component, Clone, Copy)]
enum NodeAction {
    SaveText(NodeId),
    CancelText(NodeId),
    PreviousSetting(NodeId),
    NextSetting(NodeId),
}

#[derive(Component)]
struct NodeActionButton(NodeAction);

fn select_node_for_editing(
    graph: &mut GraphEditorState,
    editing: &mut TextEditingState,
    node_id: NodeId,
) {
    let selection_changed = graph.selected_node != Some(node_id);
    graph.selected_node = Some(node_id);
    if let Some(value) = graph.node_kind(node_id).and_then(NodeKind::editable_text) {
        editing.begin_if_needed(node_id, value);
    } else if selection_changed || editing.target.is_some() {
        editing.clear();
    }
}

fn setup_editor_ui(mut commands: Commands, mut registry: ResMut<GraphUiRegistry>) {
    commands.spawn(Camera2d);

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
                row_gap: Val::Px(12.0),
                border: UiRect::right(Val::Px(1.0)),
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
            Text::new("Bevy Rig Graph"),
            TextFont {
                font_size: 28.0,
                ..default()
            },
            TextColor(Color::srgb_u8(236, 238, 241)),
        ));

        parent.spawn((
            Text::new("Compose an agent from individual Rig field nodes, target a local Ollama model, and route the response into a text sink."),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgb_u8(170, 175, 184)),
        ));

        parent
            .spawn(Node {
                width: Val::Percent(100.0),
                display: Display::Flex,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(8.0),
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                spawn_editor_button(row, "Run Selected Agent", EditorAction::RunSelectedAgent, 48.0);
                spawn_editor_button(row, "Refresh Ollama", EditorAction::RefreshOllama, 48.0);
                spawn_editor_button(row, "Reset Demo", EditorAction::ResetDemo, 48.0);
            });

        parent.spawn((
            Text::new("Add nodes"),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb_u8(230, 231, 235)),
        ));

        parent
            .spawn(Node {
                width: Val::Percent(100.0),
                display: Display::Flex,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(8.0),
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|grid| {
                for template in NodeTemplate::ALL {
                    spawn_editor_button(grid, template.label(), EditorAction::AddNode(template), 48.0);
                }
            });

        parent.spawn((
            Text::new(
                "Controls\n\
                 • Left-drag node headers to move them\n\
                 • Middle-drag the canvas to pan\n\
                 • Cmd/Ctrl + scroll to zoom the canvas\n\
                 • Click one port, then another compatible port to connect\n\
                 • Select a text node and type directly inside it\n\
                 • Save or cancel edits on the node itself\n\
                 • Run the selected Agent node to produce Text Output",
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

    let mut status_text = None;
    let toolbar = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(TOOLBAR_HEIGHT),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::axes(Val::Px(18.0), Val::Px(12.0)),
                border: UiRect::bottom(Val::Px(1.0)),
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
            Text::new("Node canvas"),
            TextFont {
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::srgb_u8(236, 238, 241)),
        ));
        status_text = Some(
            parent
                .spawn((
                    Text::new("Starting local Ollama discovery…"),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(166, 171, 180)),
                    ToolbarStatusText,
                ))
                .id(),
        );
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
            |mut click: On<Pointer<Click>>,
             mut graph: ResMut<GraphEditorState>,
             mut editing: ResMut<TextEditingState>| {
                if click.button == PointerButton::Primary {
                    click.propagate(false);
                    graph.selected_node = None;
                    graph.dragging_wire = None;
                    editing.clear();
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
    registry.status_text = status_text;
}

fn spawn_editor_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: EditorAction,
    width_percent: f32,
) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Percent(width_percent),
                min_height: Val::Px(34.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
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
                font_size: 13.0,
                ..default()
            },
            TextColor(Color::srgb_u8(232, 234, 238)),
        ));
}

fn spawn_node_action_button(parent: &mut ChildSpawnerCommands, label: &str, action: NodeAction) {
    parent
        .spawn((
            Button,
            Node {
                min_width: Val::Px(74.0),
                min_height: Val::Px(30.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            NodeActionButton(action),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .with_child((
            Text::new(label),
            TextFont {
                font_size: 12.0,
                ..default()
            },
            TextColor(Color::srgb_u8(232, 234, 238)),
        ));
}

fn handle_editor_buttons(
    mut graph: ResMut<GraphEditorState>,
    mut runtime: ResMut<RigEditorRuntime>,
    mut editing: ResMut<TextEditingState>,
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
            EditorAction::ResetDemo => {
                graph.reset_demo();
                editing.clear();
                runtime.last_status = "Graph reset to the default Ollama agent flow.".into();
            }
            EditorAction::RunSelectedAgent => match compile_selected_agent_run(&graph, &runtime) {
                Ok(request) => {
                    let output_node = request.output_node;
                    let model = request.model.clone();
                    let agent_label = request
                        .agent_name
                        .clone()
                        .unwrap_or_else(|| format!("agent#{}", request.agent_id));
                    graph.set_output_result(
                        output_node,
                        "Running selected graph…".into(),
                        format!("queued via Ollama / {model} as {agent_label}"),
                    );
                    if let Err(error) = runtime.request_run(request) {
                        runtime.last_status = error.to_string();
                    }
                }
                Err(error) => runtime.last_status = error.to_string(),
            },
            EditorAction::RefreshOllama => runtime.request_model_refresh(),
            EditorAction::AddNode(template) => {
                let slot = graph.nodes.len() as f32;
                let spawn = graph
                    .selected_node
                    .and_then(|node_id| graph.node(node_id))
                    .map(|node| node.position + Vec2::new(420.0, 24.0))
                    .unwrap_or_else(|| {
                        Vec2::new(
                            (180.0 - graph.pan.x + (slot.rem_euclid(3.0) * 36.0)) / graph.zoom,
                            (120.0 - graph.pan.y + ((slot / 3.0).floor() * 28.0)) / graph.zoom,
                        )
                    });
                graph.add_node(template.instantiate(), spawn);
            }
        }
    }
}

fn handle_node_buttons(
    mut graph: ResMut<GraphEditorState>,
    mut runtime: ResMut<RigEditorRuntime>,
    mut editing: ResMut<TextEditingState>,
    mut interactions: Query<
        (&Interaction, &NodeActionButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut background) in &mut interactions {
        background.0 = button_background(*interaction);
        if *interaction != Interaction::Pressed {
            continue;
        }

        match button.0 {
            NodeAction::SaveText(node_id) => {
                if editing.target == Some(node_id)
                    && graph.set_node_text_value(node_id, editing.buffer.clone())
                {
                    runtime.last_status = "Saved node text.".into();
                    editing.clear();
                } else {
                    runtime.last_status = "No active inline text edit for this node.".into();
                }
            }
            NodeAction::CancelText(node_id) => {
                if editing.target == Some(node_id) {
                    editing.clear();
                    runtime.last_status = "Cancelled node text edit.".into();
                }
            }
            NodeAction::PreviousSetting(node_id) => {
                graph.selected_node = Some(node_id);
                if !graph.cycle_selected_setting(-1, &runtime.ollama_models) {
                    runtime.last_status = "This node has no previous setting to cycle.".into();
                }
            }
            NodeAction::NextSetting(node_id) => {
                graph.selected_node = Some(node_id);
                if !graph.cycle_selected_setting(1, &runtime.ollama_models) {
                    runtime.last_status = "This node has no next setting to cycle.".into();
                }
            }
        }
    }
}

fn handle_text_edit_input(
    mut key_events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut editing: ResMut<TextEditingState>,
    mut graph: ResMut<GraphEditorState>,
) {
    let Some(target) = editing.target else {
        return;
    };

    let ctrl_pressed = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    for event in key_events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }

        match event.key_code {
            KeyCode::Escape => {
                editing.clear();
                return;
            }
            KeyCode::Backspace => {
                editing.buffer.pop();
                editing.mark_changed();
                continue;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if ctrl_pressed {
                    graph.set_node_text_value(target, editing.buffer.clone());
                    editing.clear();
                    return;
                }
                editing.buffer.push('\n');
                editing.mark_changed();
                continue;
            }
            KeyCode::Tab => continue,
            _ => {}
        }

        if let Some(text) = &event.text {
            editing.buffer.push_str(text);
            editing.mark_changed();
        }
    }
}

fn handle_canvas_zoom(
    mut wheel_events: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    mut graph: ResMut<GraphEditorState>,
    canvas_query: Query<(&ComputedNode, &RelativeCursorPosition), With<CanvasSurface>>,
) {
    let modifier_pressed = keys.any_pressed([
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
    ]);
    if !modifier_pressed {
        return;
    }

    let Ok((canvas_node, cursor)) = canvas_query.single() else {
        return;
    };
    let Some(pointer) = cursor.normalized else {
        return;
    };

    let mut scroll_delta = 0.0;
    for event in wheel_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y * 0.05,
        };
    }
    if scroll_delta.abs() < f32::EPSILON {
        return;
    }

    let canvas_size = canvas_node.size();
    let pointer_screen = (pointer + Vec2::splat(0.5)) * canvas_size;
    let old_zoom = graph.zoom.max(0.001);
    let new_zoom = (old_zoom * 1.1_f32.powf(scroll_delta)).clamp(0.4, 2.5);
    if (new_zoom - old_zoom).abs() < f32::EPSILON {
        return;
    }

    let world_at_pointer = (pointer_screen - graph.pan) / old_zoom;
    graph.zoom = new_zoom;
    graph.pan = pointer_screen - world_at_pointer * new_zoom;
}

fn sync_node_views(
    mut commands: Commands,
    graph: Res<GraphEditorState>,
    editing: Res<TextEditingState>,
    mut registry: ResMut<GraphUiRegistry>,
) {
    let Some(node_layer) = registry.node_layer else {
        return;
    };

    if registry.last_graph_revision == graph.revision
        && registry.last_edit_revision == editing.revision
    {
        return;
    }

    for (_, entity) in registry.node_views.drain() {
        commands.entity(entity).despawn();
    }

    for node in &graph.nodes {
        let node_snapshot = node.clone();
        let node_id = node_snapshot.id;
        let is_selected = graph.selected_node == Some(node_id);
        let is_text_editing =
            editing.target == Some(node_id) && node_snapshot.kind.editable_text().is_some();
        let is_cycle_node = matches!(
            node_snapshot.kind,
            NodeKind::Model { .. }
                | NodeKind::Temperature { .. }
                | NodeKind::MaxTokens { .. }
                | NodeKind::ToolChoice { .. }
                | NodeKind::DefaultMaxTurns { .. }
        );

        let root = commands
            .spawn((
                NodeView { id: node_id },
                node_root_style(&node_snapshot, graph.pan, graph.zoom),
                BackgroundColor(Color::srgb_u8(58, 58, 61)),
                BorderColor::all(Color::srgb_u8(74, 76, 82)),
                GlobalZIndex(4),
                UiTransform::from_scale(Vec2::splat(graph.zoom)),
                Pickable {
                    should_block_lower: true,
                    is_hoverable: true,
                },
            ))
            .observe(
                move |mut click: On<Pointer<Click>>,
                      mut graph: ResMut<GraphEditorState>,
                      mut editing: ResMut<TextEditingState>| {
                    if click.button == PointerButton::Primary {
                        click.propagate(false);
                        select_node_for_editing(&mut graph, &mut editing, node_id);
                    }
                },
            )
            .observe(
                move |mut drag: On<Pointer<Drag>>, mut graph: ResMut<GraphEditorState>| {
                    if drag.button == PointerButton::Primary {
                        drag.propagate(false);
                        let zoom = graph.zoom.max(0.001);
                        graph.move_node(node_id, drag.delta / zoom);
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
                    move |mut click: On<Pointer<Click>>,
                          mut graph: ResMut<GraphEditorState>,
                          mut editing: ResMut<TextEditingState>| {
                        if click.button == PointerButton::Primary {
                            click.propagate(false);
                            select_node_for_editing(&mut graph, &mut editing, node_id);
                        }
                    },
                )
                .observe(
                    move |mut drag: On<Pointer<Drag>>, mut graph: ResMut<GraphEditorState>| {
                        if drag.button == PointerButton::Primary {
                            drag.propagate(false);
                            let zoom = graph.zoom.max(0.001);
                            graph.move_node(node_id, drag.delta / zoom);
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
                    Pickable::IGNORE,
                ));

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
                .insert(Pickable::IGNORE)
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
                            .insert(Pickable::IGNORE)
                            .with_children(|row_parent| {
                                if let Some(spec) = inputs.get(row).copied() {
                                    row_parent
                                        .spawn(Node {
                                            display: Display::Flex,
                                            align_items: AlignItems::Center,
                                            column_gap: Val::Px(8.0),
                                            ..default()
                                        })
                                        .insert(Pickable::IGNORE)
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
                                                Text::new(format!(
                                                    "{}{}",
                                                    spec.name,
                                                    if spec.required { " *" } else { "" }
                                                )),
                                                TextFont {
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb_u8(205, 208, 214)),
                                                Pickable::IGNORE,
                                            ));
                                        });
                                } else {
                                    row_parent.spawn((
                                        Node {
                                            width: Val::Px(124.0),
                                            ..default()
                                        },
                                        Pickable::IGNORE,
                                    ));
                                }

                                if let Some(spec) = outputs.get(row).copied() {
                                    row_parent
                                        .spawn(Node {
                                            display: Display::Flex,
                                            align_items: AlignItems::Center,
                                            column_gap: Val::Px(8.0),
                                            ..default()
                                        })
                                        .insert(Pickable::IGNORE)
                                        .with_children(|group| {
                                            group.spawn((
                                                Text::new(spec.name),
                                                TextFont {
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb_u8(205, 208, 214)),
                                                Pickable::IGNORE,
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
                                    row_parent.spawn((
                                        Node {
                                            width: Val::Px(124.0),
                                            ..default()
                                        },
                                        Pickable::IGNORE,
                                    ));
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
                        ..default()
                    },
                    BorderColor {
                        top: Color::srgb_u8(77, 79, 86),
                        ..BorderColor::DEFAULT
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|body| {
                    if is_text_editing {
                        body.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                padding: UiRect::all(Val::Px(8.0)),
                                margin: UiRect::bottom(Val::Px(4.0)),
                                min_height: Val::Px(72.0),
                                border_radius: BorderRadius::all(Val::Px(6.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb_u8(58, 60, 67)),
                            Pickable::IGNORE,
                        ))
                        .with_children(|panel| {
                            panel.spawn((
                                Text::new(inline_editor_value_text(&editing)),
                                TextFont {
                                    font_size: 16.0,
                                    ..default()
                                },
                                TextColor(Color::srgb_u8(229, 232, 238)),
                                Pickable::IGNORE,
                            ));
                        });

                        body.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                                margin: UiRect::bottom(Val::Px(2.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(2.0),
                                border_radius: BorderRadius::all(Val::Px(6.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb_u8(43, 45, 50)),
                            Pickable::IGNORE,
                        ))
                        .with_children(|panel| {
                            panel.spawn((
                                Text::new("Editor"),
                                TextFont {
                                    font_size: 10.0,
                                    ..default()
                                },
                                TextColor(Color::srgb_u8(132, 138, 149)),
                                Pickable::IGNORE,
                            ));
                            panel.spawn((
                                Text::new(
                                    "Type directly into this node.\nCmd/Ctrl+Enter saves.\nEsc or Cancel discards.",
                                ),
                                TextFont {
                                    font_size: 11.0,
                                    ..default()
                                },
                                TextColor(Color::srgb_u8(150, 156, 167)),
                                Pickable::IGNORE,
                            ));
                        });
                    } else {
                        for line in node_body_lines(&node_snapshot.kind, is_text_editing, &editing) {
                            body.spawn((
                                Text::new(line),
                                TextFont {
                                    font_size: 13.0,
                                    ..default()
                                },
                                TextColor(Color::srgb_u8(176, 181, 190)),
                                Pickable::IGNORE,
                            ));
                        }
                    }

                    if is_selected && is_text_editing {
                        body.spawn(Node {
                            width: Val::Percent(100.0),
                            display: Display::Flex,
                            column_gap: Val::Px(8.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        })
                        .with_children(|row| {
                            spawn_node_action_button(row, "Save", NodeAction::SaveText(node_id));
                            spawn_node_action_button(
                                row,
                                "Cancel",
                                NodeAction::CancelText(node_id),
                            );
                        });
                    } else if is_selected && is_cycle_node {
                        body.spawn(Node {
                            width: Val::Percent(100.0),
                            display: Display::Flex,
                            column_gap: Val::Px(8.0),
                            margin: UiRect::top(Val::Px(4.0)),
                            ..default()
                        })
                        .with_children(|row| {
                            spawn_node_action_button(
                                row,
                                "Prev",
                                NodeAction::PreviousSetting(node_id),
                            );
                            spawn_node_action_button(row, "Next", NodeAction::NextSetting(node_id));
                        });
                    }
                });
        });

        registry.node_views.insert(node_id, root);
    }

    registry.last_graph_revision = graph.revision;
    registry.last_edit_revision = editing.revision;
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
            move |mut click: On<Pointer<Click>>,
                  mut graph: ResMut<GraphEditorState>,
                  mut editing: ResMut<TextEditingState>| {
                if click.button != PointerButton::Primary {
                    return;
                }

                click.propagate(false);
                select_node_for_editing(&mut graph, &mut editing, address.node);

                if let Some(dragging) = graph.dragging_wire {
                    if dragging.from == address {
                        graph.dragging_wire = None;
                        return;
                    }

                    if graph.connect(dragging.from, address) {
                        graph.dragging_wire = None;
                    } else if let Some(spec) = graph.port_spec(address) {
                        graph.dragging_wire = Some(DraggingWire {
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
    mut nodes: Query<(
        &NodeView,
        &mut Node,
        &mut BorderColor,
        &mut GlobalZIndex,
        &mut UiTransform,
    )>,
    mut headers: Query<(&NodeHeaderView, &mut BackgroundColor)>,
) {
    for (view, mut node_style, mut border, mut z_index, mut transform) in &mut nodes {
        let Some(node) = graph.node(view.id) else {
            continue;
        };

        *node_style = node_root_style(node, graph.pan, graph.zoom);
        *transform = UiTransform::from_scale(Vec2::splat(graph.zoom));
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
    runtime: Res<RigEditorRuntime>,
    editing: Res<TextEditingState>,
    registry: Res<GraphUiRegistry>,
    mut texts: Query<&mut Text>,
) {
    if let Some(entity) = registry.status_text {
        if let Ok(mut text) = texts.get_mut(entity) {
            let pending = runtime
                .pending_request
                .map(|request_id| format!("pending run #{request_id}"))
                .unwrap_or_else(|| "idle".into());
            let model_status = if runtime.ollama_ready {
                format!("ready / {} model(s)", runtime.ollama_models.len())
            } else {
                "offline".into()
            };
            let selection_status = graph
                .selected_node
                .and_then(|node_id| graph.node(node_id))
                .map(|node| node.kind.title().to_string())
                .unwrap_or_else(|| "nothing selected".into());
            let editing_status = editing
                .target
                .and_then(|node_id| graph.node(node_id))
                .map(|node| format!("editing {}", node.kind.title()))
                .unwrap_or_else(|| "not editing".into());
            text.0 = format!(
                "Ollama: {}  •  {}  •  {}  •  selected={}  •  nodes={} edges={}  •  zoom={:.0}%  •  {}",
                runtime.ollama_endpoint,
                model_status,
                pending,
                selection_status,
                graph.nodes.len(),
                graph.edges.len(),
                graph.zoom * 100.0,
                editing_status
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
        graph.zoom,
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
            (WIRE_THICKNESS * graph.zoom).clamp(1.5, 6.0),
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
                    (WIRE_THICKNESS * graph.zoom).clamp(1.5, 6.0),
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
    zoom: f32,
) {
    let spacing = (GRID_SPACING * zoom).max(10.0);
    let offset_x = pan.x.rem_euclid(spacing);
    let offset_y = pan.y.rem_euclid(spacing);

    let columns = ((canvas_size.x / spacing).ceil() as i32) + 2;
    let rows = ((canvas_size.y / spacing).ceil() as i32) + 2;

    for column in 0..columns {
        let x = offset_x + column as f32 * spacing;
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
        let y = offset_y + row as f32 * spacing;
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

fn node_root_style(node: &GraphNode, pan: Vec2, zoom: f32) -> Node {
    let size = node_dimensions(&node.kind);
    let visual_left = node.position.x * zoom + pan.x;
    let visual_top = node.position.y * zoom + pan.y;
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(visual_left + size.x * (zoom - 1.0) * 0.5),
        top: Val::Px(visual_top + size.y * (zoom - 1.0) * 0.5),
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
    let body_rows = kind.summary_lines().len() as f32;
    let height = NODE_HEADER_HEIGHT
        + 16.0
        + (row_count * (PORT_ROW_HEIGHT + 4.0))
        + 12.0
        + (body_rows * 20.0);
    Vec2::new(NODE_WIDTH, height.max(170.0))
}

fn port_center(graph: &GraphEditorState, address: PortAddress) -> Option<Vec2> {
    let node = graph.node(address.node)?;
    let row = address.index as f32;
    let zoom = graph.zoom;
    let visual_left = node.position.x * zoom + graph.pan.x;
    let visual_top = node.position.y * zoom + graph.pan.y;
    let y = visual_top + (NODE_HEADER_HEIGHT + 26.0 + row * (PORT_ROW_HEIGHT + 4.0)) * zoom;
    let x = match address.direction {
        PortDirection::Input => visual_left + PORT_X_OFFSET * zoom,
        PortDirection::Output => visual_left + (NODE_WIDTH - PORT_X_OFFSET) * zoom,
    };
    Some(Vec2::new(x, y))
}

fn node_body_lines(
    kind: &NodeKind,
    is_text_editing: bool,
    editing: &TextEditingState,
) -> Vec<String> {
    if is_text_editing {
        let mut lines = vec![
            "inline editor".into(),
            "type directly here".into(),
            "Ctrl+Enter commits".into(),
        ];
        if editing.buffer.trim().is_empty() {
            lines.push("(empty)".into());
        } else {
            lines.extend(preview_multiline(&editing.buffer, 8));
        }
        return lines;
    }

    kind.summary_lines()
}

fn inline_editor_value_text(editing: &TextEditingState) -> String {
    if editing.buffer.trim().is_empty() {
        "(empty)".into()
    } else {
        preview_multiline(&editing.buffer, 10).join("\n")
    }
}

fn preview_multiline(value: &str, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for line in value.lines().take(max_lines) {
        lines.push(truncate_preview(line, 56));
    }
    if value.lines().count() > max_lines {
        lines.push("…".into());
    }
    lines
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim_end();
    if trimmed.chars().count() > max_chars {
        let preview: String = trimmed.chars().take(max_chars).collect();
        format!("{preview}…")
    } else {
        trimmed.to_string()
    }
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
        PortType::Prompt => Color::srgb_u8(245, 211, 61),
        PortType::TextResponse => Color::srgb_u8(109, 193, 255),
        PortType::Preamble | PortType::StaticContext => Color::srgb_u8(124, 201, 151),
        PortType::Temperature | PortType::MaxTokens | PortType::DefaultMaxTurns => {
            Color::srgb_u8(255, 173, 96)
        }
        PortType::AdditionalParams | PortType::OutputSchema => Color::srgb_u8(255, 126, 133),
        PortType::ToolChoice
        | PortType::ToolServerHandle
        | PortType::DynamicContext
        | PortType::Hook => Color::srgb_u8(141, 206, 255),
        PortType::AgentName | PortType::AgentDescription => Color::srgb_u8(196, 214, 112),
    }
}
