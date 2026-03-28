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
    ui::{ComputedNode, RelativeCursorPosition, ui_transform::UiGlobalTransform},
    window::PrimaryWindow,
};

use crate::{
    compile::{compile_agent_run, compile_selected_agent_run},
    graph::{
        DraggingWire, EditorSession, GraphDocument, GraphNode, NodeId, NodeTemplate, NodeType,
        NodeValue, PortAddress, PortDirection, PortType,
    },
    runtime::RigEditorRuntime,
};

const SIDEBAR_WIDTH: f32 = 220.0;
const INSPECTOR_WIDTH: f32 = 320.0;
const TOOLBAR_HEIGHT: f32 = 58.0;
const NODE_WIDTH: f32 = 340.0;
const NODE_HEADER_HEIGHT: f32 = 34.0;
const NODE_PADDING: f32 = 12.0;
const PORT_ROW_HEIGHT: f32 = 28.0;
const PORT_DOT_SIZE: f32 = 14.0;
const PORT_X_OFFSET: f32 = 22.0;
const WIRE_THICKNESS: f32 = 3.0;
const GRID_SPACING: f32 = 28.0;
const PALETTE_WIDTH: f32 = 248.0;

pub struct NodeGraphEditorPlugin;

impl Plugin for NodeGraphEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GraphDocument>()
            .init_resource::<EditorSession>()
            .init_resource::<GraphUiRegistry>()
            .init_resource::<TextEditingState>()
            .init_resource::<NodePaletteState>()
            .add_systems(Startup, (setup_editor_ui, initialize_editor_session))
            .add_systems(
                Update,
                (
                    handle_palette_shortcuts,
                    handle_editor_buttons,
                    handle_palette_buttons,
                    handle_node_buttons,
                    handle_text_edit_input,
                    handle_canvas_zoom,
                    sync_node_views,
                    sync_palette_view,
                    update_node_view_state,
                    update_chrome_text,
                    rebuild_canvas_overlay,
                ),
            );
    }
}

fn initialize_editor_session(document: Res<GraphDocument>, mut session: ResMut<EditorSession>) {
    if session.selected_node.is_none() {
        session.select_node(document.first_node_id_by_type(NodeType::Agent));
    }
}

#[derive(Resource, Default)]
struct GraphUiRegistry {
    canvas: Option<Entity>,
    overlay_layer: Option<Entity>,
    node_layer: Option<Entity>,
    status_text: Option<Entity>,
    inspector_title_text: Option<Entity>,
    inspector_body_text: Option<Entity>,
    palette_parent: Option<Entity>,
    palette_view: Option<Entity>,
    node_views: HashMap<NodeId, Entity>,
    overlay_entities: Vec<Entity>,
    last_graph_revision: u64,
    last_edit_revision: u64,
    last_palette_revision: u64,
    last_zoom: f32,
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

#[derive(Resource, Default)]
struct NodePaletteState {
    visible: bool,
    search: String,
    screen_position: Vec2,
    spawn_world: Vec2,
    revision: u64,
}

impl NodePaletteState {
    fn open(&mut self, screen_position: Vec2, spawn_world: Vec2) {
        self.visible = true;
        self.search.clear();
        self.screen_position = screen_position;
        self.spawn_world = spawn_world;
        self.revision = self.revision.wrapping_add(1);
    }

    fn close(&mut self) {
        if self.visible || !self.search.is_empty() {
            self.visible = false;
            self.search.clear();
            self.revision = self.revision.wrapping_add(1);
        }
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

#[derive(Component)]
struct InspectorTitleText;

#[derive(Component)]
struct InspectorBodyText;

#[derive(Component, Clone, Copy)]
enum EditorAction {
    ResetDemo,
    RunSelectedAgent,
    StopRun,
    OpenNodePalette,
    FrameSelected,
    FrameGraph,
}

#[derive(Component)]
struct EditorButton(EditorAction);

#[derive(Component, Clone, Copy)]
enum NodeAction {
    SaveText(NodeId),
    CancelText(NodeId),
    PreviousSetting(NodeId),
    NextSetting(NodeId),
    RefreshModels(NodeId),
    RunAgent(NodeId),
    ClearOutput(NodeId),
    DuplicateNode(NodeId),
    DeleteNode(NodeId),
}

#[derive(Component)]
struct NodeActionButton(NodeAction);

#[derive(Component, Clone, Copy)]
struct PaletteButton(NodeTemplate);

fn select_node_for_editing(
    document: &mut GraphDocument,
    session: &mut EditorSession,
    editing: &mut TextEditingState,
    node_id: NodeId,
) {
    let selection_changed = session.selected_node != Some(node_id);
    session.select_node(Some(node_id));
    if let Some(value) = document.node(node_id).and_then(GraphNode::editable_text) {
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
                justify_content: JustifyContent::SpaceBetween,
                row_gap: Val::Px(16.0),
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
        parent
            .spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(14.0),
                ..default()
            })
            .with_children(|top| {
                top.spawn((
                    Text::new("Bevy Rig Graph"),
                    TextFont {
                        font_size: 26.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(236, 238, 241)),
                ));

                top.spawn((
                    Text::new(
                        "Canvas-first LLM graph editor.\nRight-click or press Space to add nodes.",
                    ),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(170, 175, 184)),
                ));

                top.spawn((
                    Text::new(
                        "Shortcuts\n\
                         • Space / Tab: open node palette\n\
                         • Right-click canvas: add near cursor\n\
                         • Cmd/Ctrl + wheel: zoom\n\
                         • Middle-drag: pan",
                    ),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(145, 150, 158)),
                ));
            });

        parent
            .spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|bottom| {
                bottom.spawn((
                    Text::new("Graph-level actions live in the top bar. Edit values directly on nodes; inspect wiring on the right."),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(136, 142, 151)),
                ));
                spawn_editor_button(bottom, "Reset Demo", EditorAction::ResetDemo, 100.0);
            });
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

    let inspector = commands
        .spawn((
            Node {
                width: Val::Px(INSPECTOR_WIDTH),
                height: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(16.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(12.0),
                border: UiRect::left(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgb_u8(24, 25, 28)),
            BorderColor {
                left: Color::srgb_u8(48, 50, 56),
                ..BorderColor::DEFAULT
            },
        ))
        .id();
    commands.entity(root).add_child(inspector);

    let mut status_text = None;
    let mut inspector_title_text = None;
    let mut inspector_body_text = None;
    let toolbar = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(TOOLBAR_HEIGHT),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::axes(Val::Px(18.0), Val::Px(12.0)),
                border: UiRect::bottom(Val::Px(1.0)),
                column_gap: Val::Px(12.0),
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
        parent
            .spawn(Node {
                display: Display::Flex,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                spawn_editor_button(row, "Run", EditorAction::RunSelectedAgent, 0.0);
                spawn_editor_button(row, "Stop", EditorAction::StopRun, 0.0);
                spawn_editor_button(row, "Add Node", EditorAction::OpenNodePalette, 0.0);
                spawn_editor_button(row, "Frame Selected", EditorAction::FrameSelected, 0.0);
                spawn_editor_button(row, "Frame Graph", EditorAction::FrameGraph, 0.0);
            });
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

    commands.entity(inspector).with_children(|parent| {
        inspector_title_text = Some(
            parent
                .spawn((
                    Text::new("Inspector"),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(236, 238, 241)),
                    InspectorTitleText,
                ))
                .id(),
        );
        inspector_body_text = Some(
            parent
                .spawn((
                    Text::new("Select a node to inspect its wiring and runtime state."),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(170, 175, 184)),
                    InspectorBodyText,
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
             mut session: ResMut<EditorSession>,
             mut editing: ResMut<TextEditingState>,
             mut palette: ResMut<NodePaletteState>,
             canvas_query: Query<(&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition), With<CanvasSurface>>| {
                if click.button == PointerButton::Primary {
                    click.propagate(false);
                    session.select_node(None);
                    session.dragging_wire = None;
                    session.touch();
                    editing.clear();
                    palette.close();
                } else if click.button == PointerButton::Secondary {
                    click.propagate(false);
                    if let Some(screen_position) =
                        canvas_local_from_window_position(&canvas_query, click.pointer_location.position)
                    {
                        let spawn_world =
                            screen_to_world(screen_position, session.pan, session.zoom);
                        palette.open(screen_position, spawn_world);
                    }
                }
            },
        )
        .observe(
            |mut drag: On<Pointer<Drag>>, mut session: ResMut<EditorSession>| {
                if drag.button == PointerButton::Middle {
                    drag.propagate(false);
                    session.pan += drag.delta;
                    session.touch();
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
    registry.inspector_title_text = inspector_title_text;
    registry.inspector_body_text = inspector_body_text;
    registry.palette_parent = Some(canvas);
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
                width: if width_percent > 0.0 {
                    Val::Percent(width_percent)
                } else {
                    Val::Auto
                },
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
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .with_child((
            Text::new(label),
            TextFont {
                font_size: 13.0,
                ..default()
            },
            TextColor(Color::srgb_u8(232, 234, 238)),
        ));
}

fn spawn_scaled_node_action_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: NodeAction,
    zoom: f32,
) {
    parent
        .spawn((
            Button,
            Node {
                min_width: Val::Px(scaled(74.0, zoom)),
                min_height: Val::Px(scaled(30.0, zoom)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Val::Px(scaled(10.0, zoom)), Val::Px(scaled(6.0, zoom))),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(scaled(8.0, zoom))),
                ..default()
            },
            NodeActionButton(action),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .with_child((
            Text::new(label),
            TextFont {
                font_size: scaled_font(12.0, zoom),
                ..default()
            },
            TextColor(Color::srgb_u8(232, 234, 238)),
        ));
}

fn spawn_palette_button(parent: &mut ChildSpawnerCommands, label: &str, template: NodeTemplate) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(30.0),
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            PaletteButton(template),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .with_child((
            Text::new(label),
            TextFont {
                font_size: 13.0,
                ..default()
            },
            TextColor(Color::srgb_u8(232, 234, 238)),
            Pickable::IGNORE,
        ));
}

fn handle_editor_buttons(
    mut document: ResMut<GraphDocument>,
    mut session: ResMut<EditorSession>,
    mut runtime: ResMut<RigEditorRuntime>,
    mut editing: ResMut<TextEditingState>,
    mut palette: ResMut<NodePaletteState>,
    canvas_query: Query<&ComputedNode, With<CanvasSurface>>,
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
                document.reset_demo();
                session.reset_view();
                session.select_node(document.first_node_id_by_type(NodeType::Agent));
                editing.clear();
                palette.close();
                runtime.last_status = "Graph reset to the default Ollama agent flow.".into();
            }
            EditorAction::RunSelectedAgent => {
                match compile_selected_agent_run(&document, &session, &runtime) {
                    Ok(request) => {
                        let output_node = request.output_node;
                        let model = request.model.clone();
                        let agent_label = request
                            .agent_name
                            .clone()
                            .unwrap_or_else(|| format!("agent#{}", request.agent_id));
                        document.set_output_result(
                            output_node,
                            "Running selected graph…".into(),
                            format!("queued via Ollama / {model} as {agent_label}"),
                        );
                        if let Err(error) = runtime.request_run(request) {
                            runtime.last_status = error.to_string();
                        }
                    }
                    Err(error) => runtime.last_status = error.to_string(),
                }
            }
            EditorAction::StopRun => {
                if let Some(output_node) = runtime.stop_run() {
                    document.set_output_result(
                        output_node,
                        "Run stopped.".into(),
                        "stopped".into(),
                    );
                }
            }
            EditorAction::OpenNodePalette => {
                if let Ok(canvas) = canvas_query.single() {
                    let center = canvas.size() * 0.5;
                    let spawn = screen_to_world(center, session.pan, session.zoom);
                    palette.open(center, spawn);
                }
            }
            EditorAction::FrameSelected => {
                if let Ok(canvas) = canvas_query.single() {
                    if let Some(node_id) = session.selected_node {
                        if let Some(node) = document.node(node_id) {
                            session.pan = canvas.size() * 0.5
                                - Vec2::new(
                                    (node.position.x + NODE_WIDTH * 0.5) * session.zoom.max(0.001),
                                    (node.position.y + 120.0) * session.zoom.max(0.001),
                                );
                            session.touch();
                        } else {
                            runtime.last_status = "Select a node to frame it.".into();
                        }
                    } else {
                        runtime.last_status = "Select a node to frame it.".into();
                    }
                }
            }
            EditorAction::FrameGraph => {
                if let Ok(canvas) = canvas_query.single() {
                    if document.node_count() == 0 {
                        runtime.last_status = "Graph is empty.".into();
                        continue;
                    }
                    let mut min = Vec2::splat(f32::INFINITY);
                    let mut max = Vec2::splat(f32::NEG_INFINITY);
                    for node in document.iter_nodes() {
                        min = min.min(node.position);
                        max = max.max(node.position + Vec2::new(NODE_WIDTH, 220.0));
                    }
                    let bounds = (max - min).max(Vec2::splat(1.0));
                    let padding = Vec2::splat(96.0);
                    let available = (canvas.size() - padding).max(Vec2::splat(200.0));
                    let fit = (available.x / bounds.x)
                        .min(available.y / bounds.y)
                        .clamp(0.45, 1.4);
                    session.zoom = fit;
                    session.pan = canvas.size() * 0.5
                        - world_to_screen(min + bounds * 0.5, Vec2::ZERO, session.zoom);
                    session.touch();
                }
            }
        }
    }
}

fn handle_node_buttons(
    mut document: ResMut<GraphDocument>,
    mut session: ResMut<EditorSession>,
    mut runtime: ResMut<RigEditorRuntime>,
    mut editing: ResMut<TextEditingState>,
    mut palette: ResMut<NodePaletteState>,
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
                    && document.set_node_text_value(node_id, editing.buffer.clone())
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
                session.select_node(Some(node_id));
                if !document.cycle_setting(node_id, -1, &runtime.ollama_models) {
                    runtime.last_status = "This node has no previous setting to cycle.".into();
                }
            }
            NodeAction::NextSetting(node_id) => {
                session.select_node(Some(node_id));
                if !document.cycle_setting(node_id, 1, &runtime.ollama_models) {
                    runtime.last_status = "This node has no next setting to cycle.".into();
                }
            }
            NodeAction::RefreshModels(node_id) => {
                session.select_node(Some(node_id));
                runtime.request_model_refresh();
            }
            NodeAction::RunAgent(node_id) => {
                session.select_node(Some(node_id));
                match compile_agent_run(&document, &runtime, node_id) {
                    Ok(request) => {
                        let output_node = request.output_node;
                        let model = request.model.clone();
                        let agent_label = request
                            .agent_name
                            .clone()
                            .unwrap_or_else(|| format!("agent#{}", request.agent_id));
                        document.set_output_result(
                            output_node,
                            "Running selected graph…".into(),
                            format!("queued via Ollama / {model} as {agent_label}"),
                        );
                        if let Err(error) = runtime.request_run(request) {
                            runtime.last_status = error.to_string();
                        }
                    }
                    Err(error) => runtime.last_status = error.to_string(),
                }
            }
            NodeAction::ClearOutput(node_id) => {
                if document.clear_output(node_id) {
                    runtime.last_status = "Cleared text output.".into();
                }
            }
            NodeAction::DuplicateNode(node_id) => {
                session.select_node(Some(node_id));
                if document
                    .duplicate_node(node_id, Vec2::new(36.0, 36.0))
                    .is_some()
                {
                    if let Some(selected) = session.selected_node {
                        if let Some(value) =
                            document.node(selected).and_then(GraphNode::editable_text)
                        {
                            editing.begin_if_needed(selected, value);
                        } else {
                            editing.clear();
                        }
                    }
                    palette.close();
                    runtime.last_status = "Duplicated node.".into();
                }
            }
            NodeAction::DeleteNode(node_id) => {
                let was_editing = editing.target == Some(node_id);
                if document.remove_node(node_id) {
                    if session.selected_node == Some(node_id) {
                        session.select_node(None);
                    }
                    if matches!(session.dragging_wire, Some(DraggingWire { from, .. }) if from.node == node_id)
                    {
                        session.dragging_wire = None;
                        session.touch();
                    }
                    if was_editing {
                        editing.clear();
                    }
                    palette.close();
                    runtime.last_status = "Deleted node.".into();
                }
            }
        }
    }
}

fn handle_text_edit_input(
    mut key_events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut editing: ResMut<TextEditingState>,
    mut document: ResMut<GraphDocument>,
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
                    document.set_node_text_value(target, editing.buffer.clone());
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
    mut session: ResMut<EditorSession>,
    canvas_query: Query<(&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition), With<CanvasSurface>>,
    windows: Query<&Window, With<PrimaryWindow>>,
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

    let Some(pointer_screen) = current_canvas_cursor_local_position(&windows, &canvas_query) else {
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

    let old_zoom = session.zoom.max(0.001);
    let new_zoom = (old_zoom * 1.1_f32.powf(scroll_delta)).clamp(0.4, 2.5);
    if (new_zoom - old_zoom).abs() < f32::EPSILON {
        return;
    }

    let world_at_pointer = screen_to_world(pointer_screen, session.pan, old_zoom);
    session.zoom = new_zoom;
    session.pan = pointer_screen - world_to_screen(world_at_pointer, Vec2::ZERO, new_zoom);
    session.touch();
}

fn handle_palette_shortcuts(
    mut key_events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut document: ResMut<GraphDocument>,
    mut session: ResMut<EditorSession>,
    mut editing: ResMut<TextEditingState>,
    mut palette: ResMut<NodePaletteState>,
    canvas_query: Query<&ComputedNode, With<CanvasSurface>>,
) {
    if editing.target.is_some() {
        return;
    }

    let ctrl_pressed = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);

    for event in key_events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }

        if !palette.visible {
            match event.key_code {
                KeyCode::Space | KeyCode::Tab => {
                    if let Ok(canvas) = canvas_query.single() {
                        let center = canvas.size() * 0.5;
                        let spawn = screen_to_world(center, session.pan, session.zoom);
                        palette.open(center, spawn);
                    }
                    continue;
                }
                _ => continue,
            }
        }

        match event.key_code {
            KeyCode::Escape => {
                palette.close();
                return;
            }
            KeyCode::Backspace => {
                palette.search.pop();
                palette.mark_changed();
                continue;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(template) = filtered_node_templates(&palette.search).first().copied() {
                    let node_id = document.add_node(template.instantiate(), palette.spawn_world);
                    select_node_for_editing(&mut document, &mut session, &mut editing, node_id);
                    palette.close();
                }
                return;
            }
            KeyCode::Tab => continue,
            _ => {}
        }

        if ctrl_pressed {
            continue;
        }

        if let Some(text) = &event.text {
            if !text.chars().all(char::is_control) {
                palette.search.push_str(text);
                palette.mark_changed();
            }
        }
    }
}

fn handle_palette_buttons(
    mut document: ResMut<GraphDocument>,
    mut session: ResMut<EditorSession>,
    mut editing: ResMut<TextEditingState>,
    mut palette: ResMut<NodePaletteState>,
    mut interactions: Query<
        (&Interaction, &PaletteButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut background) in &mut interactions {
        background.0 = button_background(*interaction);
        if *interaction != Interaction::Pressed {
            continue;
        }

        let node_id = document.add_node(button.0.instantiate(), palette.spawn_world);
        select_node_for_editing(&mut document, &mut session, &mut editing, node_id);
        palette.close();
    }
}

fn sync_palette_view(
    mut commands: Commands,
    palette: Res<NodePaletteState>,
    mut registry: ResMut<GraphUiRegistry>,
    canvas_query: Query<&ComputedNode, With<CanvasSurface>>,
) {
    if registry.last_palette_revision == palette.revision {
        return;
    }

    if let Some(entity) = registry.palette_view.take() {
        commands.entity(entity).despawn();
    }

    if palette.visible {
        let Some(parent) = registry.palette_parent else {
            return;
        };
        let canvas_size = canvas_query
            .get(parent)
            .map(ComputedNode::size)
            .unwrap_or(Vec2::new(1200.0, 900.0));
        let max_left = (canvas_size.x - PALETTE_WIDTH - 18.0).max(18.0);
        let max_top = (canvas_size.y - 320.0).max(18.0);

        let panel = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(palette.screen_position.x.clamp(18.0, max_left)),
                    top: Val::Px(palette.screen_position.y.clamp(18.0, max_top)),
                    width: Val::Px(PALETTE_WIDTH),
                    padding: UiRect::all(Val::Px(12.0)),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb_u8(28, 29, 33)),
                BorderColor::all(Color::srgb_u8(66, 69, 76)),
                GlobalZIndex(40),
            ))
            .observe(|mut click: On<Pointer<Click>>| {
                click.propagate(false);
            })
            .with_children(|parent| {
                parent.spawn((
                    Text::new("Add Node"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(236, 238, 241)),
                    Pickable::IGNORE,
                ));
                parent
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                            border_radius: BorderRadius::all(Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb_u8(42, 44, 49)),
                        Pickable::IGNORE,
                    ))
                    .with_children(|field| {
                        field.spawn((
                            Text::new(if palette.search.is_empty() {
                                "Search nodes…".into()
                            } else {
                                palette.search.clone()
                            }),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(if palette.search.is_empty() {
                                Color::srgb_u8(138, 144, 154)
                            } else {
                                Color::srgb_u8(226, 229, 235)
                            }),
                            Pickable::IGNORE,
                        ));
                    });

                let matches = filtered_node_templates(&palette.search);
                if matches.is_empty() {
                    parent.spawn((
                        Text::new("No matching node types."),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(Color::srgb_u8(150, 156, 167)),
                        Pickable::IGNORE,
                    ));
                } else {
                    for template in matches.into_iter().take(10) {
                        spawn_palette_button(parent, template.label(), template);
                    }
                }

                parent.spawn((
                    Text::new("Enter adds the first result • Esc closes"),
                    TextFont {
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(Color::srgb_u8(132, 138, 149)),
                    Pickable::IGNORE,
                ));
            })
            .id();
        commands.entity(parent).add_child(panel);
        registry.palette_view = Some(panel);
    }

    registry.last_palette_revision = palette.revision;
}

fn sync_node_views(
    mut commands: Commands,
    document: Res<GraphDocument>,
    session: Res<EditorSession>,
    editing: Res<TextEditingState>,
    mut registry: ResMut<GraphUiRegistry>,
) {
    let Some(node_layer) = registry.node_layer else {
        return;
    };

    if registry.last_graph_revision == document.revision
        && registry.last_edit_revision == editing.revision
        && (registry.last_zoom - session.zoom).abs() < f32::EPSILON
    {
        return;
    }

    for (_, entity) in registry.node_views.drain() {
        commands.entity(entity).despawn();
    }

    for node in document.iter_nodes() {
        let node_snapshot = node.clone();
        let node_id = node_snapshot.id;
        let is_selected = session.selected_node == Some(node_id);
        let is_text_editing =
            editing.target == Some(node_id) && node_snapshot.editable_text().is_some();
        let zoom = session.zoom.max(0.001);
        let header_height = scaled(NODE_HEADER_HEIGHT, zoom);
        let node_padding = scaled(NODE_PADDING, zoom);
        let port_row_height = scaled(PORT_ROW_HEIGHT, zoom);
        let is_cycle_node = matches!(
            node_snapshot.node_type,
            NodeType::Model
                | NodeType::Temperature
                | NodeType::MaxTokens
                | NodeType::ToolChoice
                | NodeType::DefaultMaxTurns
        );
        let is_model_node = matches!(node_snapshot.node_type, NodeType::Model);
        let is_agent_node = matches!(node_snapshot.node_type, NodeType::Agent);
        let is_output_node = matches!(node_snapshot.node_type, NodeType::TextOutput);

        let root = commands
            .spawn((
                NodeView { id: node_id },
                node_root_style(&node_snapshot, session.pan, session.zoom),
                BackgroundColor(Color::srgb_u8(58, 58, 61)),
                BorderColor::all(Color::srgb_u8(74, 76, 82)),
                GlobalZIndex(4),
                Pickable {
                    should_block_lower: true,
                    is_hoverable: true,
                },
            ))
            .observe(
                move |mut click: On<Pointer<Click>>,
                      mut document: ResMut<GraphDocument>,
                      mut session: ResMut<EditorSession>,
                      mut editing: ResMut<TextEditingState>| {
                    if click.button == PointerButton::Primary {
                        click.propagate(false);
                        select_node_for_editing(&mut document, &mut session, &mut editing, node_id);
                    }
                },
            )
            .observe(
                move |mut drag: On<Pointer<Drag>>,
                      mut document: ResMut<GraphDocument>,
                      session: Res<EditorSession>| {
                    if drag.button == PointerButton::Primary {
                        drag.propagate(false);
                        let zoom = session.zoom.max(0.001);
                        document.move_node(node_id, drag.delta / zoom);
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
                        height: Val::Px(header_height),
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(Val::Px(scaled(12.0, zoom)), Val::Px(scaled(8.0, zoom))),
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
                          mut document: ResMut<GraphDocument>,
                          mut session: ResMut<EditorSession>,
                          mut editing: ResMut<TextEditingState>| {
                        if click.button == PointerButton::Primary {
                            click.propagate(false);
                            select_node_for_editing(&mut document, &mut session, &mut editing, node_id);
                        }
                    },
                )
                .observe(
                    move |mut drag: On<Pointer<Drag>>, mut document: ResMut<GraphDocument>, session: Res<EditorSession>| {
                        if drag.button == PointerButton::Primary {
                            drag.propagate(false);
                            let zoom = session.zoom.max(0.001);
                            document.move_node(node_id, drag.delta / zoom);
                        }
                    },
                )
                .with_child((
                    Text::new(node_snapshot.title.clone()),
                    TextFont {
                        font_size: scaled_font(15.0, zoom),
                        ..default()
                    },
                    TextColor(Color::srgb_u8(238, 240, 243)),
                    Pickable::IGNORE,
                ));

            let inputs = node_snapshot.inputs();
            let outputs = node_snapshot.outputs();
            let row_count = inputs.len().max(outputs.len()).max(1);

            parent
                .spawn(Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(node_padding), Val::Px(scaled(10.0, zoom))),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(scaled(4.0, zoom)),
                    ..default()
                })
                .insert(Pickable::IGNORE)
                .with_children(|ports| {
                    for row in 0..row_count {
                        ports
                            .spawn(Node {
                                width: Val::Percent(100.0),
                                min_height: Val::Px(port_row_height),
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
                                            column_gap: Val::Px(scaled(8.0, zoom)),
                                            ..default()
                                        })
                                        .insert(Pickable::IGNORE)
                                        .with_children(|group| {
                                            let connected =
                                                !document.input_sources(node_id, spec.ty).is_empty();
                                            spawn_port_button(
                                                group,
                                                PortAddress {
                                                    node: node_id,
                                                    direction: PortDirection::Input,
                                                    index: row,
                                                },
                                                spec.ty,
                                                zoom,
                                            );
                                            group.spawn((
                                                Text::new(format!(
                                                    "{}{}",
                                                    spec.name,
                                                    if spec.required { " *" } else { "" }
                                                )),
                                                TextFont {
                                                    font_size: scaled_font(13.0, zoom),
                                                    ..default()
                                                },
                                                TextColor(if spec.required {
                                                    Color::srgb_u8(244, 230, 165)
                                                } else if connected {
                                                    Color::srgb_u8(218, 221, 227)
                                                } else {
                                                    Color::srgb_u8(182, 186, 195)
                                                }),
                                                Pickable::IGNORE,
                                            ));
                                        });
                                } else {
                                    row_parent.spawn((
                                        Node {
                                            width: Val::Px(scaled(124.0, zoom)),
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
                                            column_gap: Val::Px(scaled(8.0, zoom)),
                                            ..default()
                                        })
                                        .insert(Pickable::IGNORE)
                                        .with_children(|group| {
                                            let connected =
                                                !document.output_targets(node_id, spec.ty).is_empty();
                                            group.spawn((
                                                Text::new(spec.name),
                                                TextFont {
                                                    font_size: scaled_font(13.0, zoom),
                                                    ..default()
                                                },
                                                TextColor(if connected {
                                                    Color::srgb_u8(218, 221, 227)
                                                } else {
                                                    Color::srgb_u8(182, 186, 195)
                                                }),
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
                                                zoom,
                                            );
                                        });
                                } else {
                                    row_parent.spawn((
                                        Node {
                                            width: Val::Px(scaled(124.0, zoom)),
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
                        padding: UiRect::all(Val::Px(node_padding)),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(scaled(6.0, zoom)),
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
                                padding: UiRect::all(Val::Px(scaled(8.0, zoom))),
                                margin: UiRect::bottom(Val::Px(scaled(4.0, zoom))),
                                min_height: Val::Px(scaled(72.0, zoom)),
                                border_radius: BorderRadius::all(Val::Px(scaled(6.0, zoom))),
                                ..default()
                            },
                            BackgroundColor(Color::srgb_u8(58, 60, 67)),
                            Pickable::IGNORE,
                        ))
                        .with_children(|panel| {
                            panel.spawn((
                                Text::new(inline_editor_value_text(&editing)),
                                TextFont {
                                    font_size: scaled_font(16.0, zoom),
                                    ..default()
                                },
                                TextColor(Color::srgb_u8(229, 232, 238)),
                                Pickable::IGNORE,
                            ));
                        });

                        body.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                padding: UiRect::axes(Val::Px(scaled(8.0, zoom)), Val::Px(scaled(6.0, zoom))),
                                margin: UiRect::bottom(Val::Px(scaled(2.0, zoom))),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(scaled(2.0, zoom)),
                                border_radius: BorderRadius::all(Val::Px(scaled(6.0, zoom))),
                                ..default()
                            },
                            BackgroundColor(Color::srgb_u8(43, 45, 50)),
                            Pickable::IGNORE,
                        ))
                        .with_children(|panel| {
                            panel.spawn((
                                Text::new("Editor"),
                                TextFont {
                                    font_size: scaled_font(10.0, zoom),
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
                                    font_size: scaled_font(11.0, zoom),
                                    ..default()
                                },
                                TextColor(Color::srgb_u8(150, 156, 167)),
                                Pickable::IGNORE,
                            ));
                        });
                    } else {
                        for line in
                            node_body_lines(&document, node_id, &node_snapshot, is_text_editing, &editing)
                        {
                            body.spawn((
                                Text::new(line),
                                TextFont {
                                    font_size: scaled_font(13.0, zoom),
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
                            column_gap: Val::Px(scaled(8.0, zoom)),
                            margin: UiRect::top(Val::Px(scaled(4.0, zoom))),
                            ..default()
                        })
                        .with_children(|row| {
                            spawn_scaled_node_action_button(
                                row,
                                "Save",
                                NodeAction::SaveText(node_id),
                                zoom,
                            );
                            spawn_scaled_node_action_button(
                                row,
                                "Cancel",
                                NodeAction::CancelText(node_id),
                                zoom,
                            );
                        });
                    } else if is_selected && is_cycle_node {
                        body.spawn(Node {
                            width: Val::Percent(100.0),
                            display: Display::Flex,
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(scaled(8.0, zoom)),
                            row_gap: Val::Px(scaled(8.0, zoom)),
                            margin: UiRect::top(Val::Px(scaled(4.0, zoom))),
                            ..default()
                        })
                        .with_children(|row| {
                            if is_model_node {
                                spawn_scaled_node_action_button(
                                    row,
                                    "Refresh",
                                    NodeAction::RefreshModels(node_id),
                                    zoom,
                                );
                            }
                            spawn_scaled_node_action_button(
                                row,
                                "Prev",
                                NodeAction::PreviousSetting(node_id),
                                zoom,
                            );
                            spawn_scaled_node_action_button(
                                row,
                                "Next",
                                NodeAction::NextSetting(node_id),
                                zoom,
                            );
                        });
                    }

                    if is_selected {
                        body.spawn(Node {
                            width: Val::Percent(100.0),
                            display: Display::Flex,
                            flex_wrap: FlexWrap::Wrap,
                            column_gap: Val::Px(scaled(8.0, zoom)),
                            row_gap: Val::Px(scaled(8.0, zoom)),
                            margin: UiRect::top(Val::Px(scaled(4.0, zoom))),
                            ..default()
                        })
                        .with_children(|row| {
                            if is_agent_node {
                                spawn_scaled_node_action_button(
                                    row,
                                    "Run",
                                    NodeAction::RunAgent(node_id),
                                    zoom,
                                );
                            }
                            if is_output_node {
                                spawn_scaled_node_action_button(
                                    row,
                                    "Clear",
                                    NodeAction::ClearOutput(node_id),
                                    zoom,
                                );
                            }
                            spawn_scaled_node_action_button(
                                row,
                                "Duplicate",
                                NodeAction::DuplicateNode(node_id),
                                zoom,
                            );
                            spawn_scaled_node_action_button(
                                row,
                                "Delete",
                                NodeAction::DeleteNode(node_id),
                                zoom,
                            );
                        });
                    }
                });
        });

        registry.node_views.insert(node_id, root);
    }

    registry.last_graph_revision = document.revision;
    registry.last_edit_revision = editing.revision;
    registry.last_zoom = session.zoom;
}

fn spawn_port_button(
    parent: &mut ChildSpawnerCommands,
    address: PortAddress,
    ty: PortType,
    zoom: f32,
) {
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(scaled(PORT_DOT_SIZE, zoom)),
                height: Val::Px(scaled(PORT_DOT_SIZE, zoom)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(scaled(PORT_DOT_SIZE, zoom))),
                ..default()
            },
            BackgroundColor(port_color(ty)),
            BorderColor::all(Color::BLACK),
        ))
        .observe(
            move |mut click: On<Pointer<Click>>,
                  mut document: ResMut<GraphDocument>,
                  mut session: ResMut<EditorSession>,
                  mut editing: ResMut<TextEditingState>| {
                if click.button != PointerButton::Primary {
                    return;
                }

                click.propagate(false);
                select_node_for_editing(&mut document, &mut session, &mut editing, address.node);

                if let Some(dragging) = session.dragging_wire {
                    if dragging.from == address {
                        session.dragging_wire = None;
                        session.touch();
                        return;
                    }

                    if document.connect(dragging.from, address) {
                        session.dragging_wire = None;
                        session.touch();
                    } else if let Some(spec) = document.port_spec(address) {
                        session.dragging_wire = Some(DraggingWire {
                            from: address,
                            ty: spec.ty,
                        });
                        session.touch();
                    }
                } else if let Some(spec) = document.port_spec(address) {
                    session.dragging_wire = Some(DraggingWire {
                        from: address,
                        ty: spec.ty,
                    });
                    session.touch();
                }
            },
        );
}

fn update_node_view_state(
    document: Res<GraphDocument>,
    session: Res<EditorSession>,
    mut nodes: Query<(&NodeView, &mut Node, &mut BorderColor, &mut GlobalZIndex)>,
    mut headers: Query<(&NodeHeaderView, &mut BackgroundColor)>,
) {
    for (view, mut node_style, mut border, mut z_index) in &mut nodes {
        let Some(node) = document.node(view.id) else {
            continue;
        };

        *node_style = node_root_style(node, session.pan, session.zoom);
        let selected = session.selected_node == Some(view.id);
        border.set_all(if selected {
            Color::srgb_u8(248, 205, 88)
        } else {
            Color::srgb_u8(74, 76, 82)
        });
        z_index.0 = if selected { 16 } else { 4 };
    }

    for (header, mut color) in &mut headers {
        color.0 = if session.selected_node == Some(header.id) {
            Color::srgb_u8(90, 90, 96)
        } else {
            Color::srgb_u8(74, 74, 78)
        };
    }
}

fn update_chrome_text(
    document: Res<GraphDocument>,
    session: Res<EditorSession>,
    runtime: Res<RigEditorRuntime>,
    editing: Res<TextEditingState>,
    palette: Res<NodePaletteState>,
    registry: Res<GraphUiRegistry>,
    mut texts: Query<&mut Text>,
) {
    if let Some(entity) = registry.status_text {
        if let Ok(mut text) = texts.get_mut(entity) {
            let run_status = runtime
                .pending_request
                .map(|request_id| format!("running #{request_id}"))
                .unwrap_or_else(|| "idle".into());
            let selection_status = session
                .selected_node
                .and_then(|node_id| document.node(node_id))
                .map(|node| node.title.clone())
                .unwrap_or_else(|| "no selection".into());
            let palette_status = if palette.visible {
                "palette open"
            } else {
                "palette closed"
            };
            text.0 = format!(
                "Ollama {}  •  {} local model(s)  •  {}  •  {}  •  zoom {:.0}%  •  {}",
                if runtime.ollama_ready {
                    "ready"
                } else {
                    "offline"
                },
                runtime.ollama_models.len(),
                run_status,
                selection_status,
                session.zoom * 100.0,
                if editing.target.is_some() {
                    "editing"
                } else {
                    palette_status
                }
            );
        }
    }

    if let Some(entity) = registry.inspector_title_text {
        if let Ok(mut text) = texts.get_mut(entity) {
            let selected_title = session
                .selected_node
                .and_then(|node_id| document.node(node_id))
                .map(|node| node.title.clone())
                .unwrap_or_else(|| "Inspector".into());
            text.0 = if session.selected_node.is_some() {
                format!("{selected_title} Inspector")
            } else {
                "Inspector".into()
            };
        }
    }

    if let Some(entity) = registry.inspector_body_text {
        if let Ok(mut text) = texts.get_mut(entity) {
            text.0 = inspector_text(&document, &session, &runtime);
        }
    }
}

fn rebuild_canvas_overlay(
    mut commands: Commands,
    document: Res<GraphDocument>,
    session: Res<EditorSession>,
    mut registry: ResMut<GraphUiRegistry>,
    canvas_query: Query<(&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition), With<CanvasSurface>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let (Some(overlay_layer), Some(canvas)) = (registry.overlay_layer, registry.canvas) else {
        return;
    };

    let Ok((canvas_node, _canvas_transform, _relative_cursor)) = canvas_query.get(canvas) else {
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
        session.pan,
        session.zoom,
    );

    for edge in document.iter_edges() {
        let Some(start) = port_center(&document, &session, edge.from) else {
            continue;
        };
        let Some(end) = port_center(&document, &session, edge.to) else {
            continue;
        };
        let color = document
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
            (WIRE_THICKNESS * session.zoom).clamp(1.5, 6.0),
        );
    }

    if let Some(dragging) = session.dragging_wire {
        if let Some(pointer) = current_canvas_cursor_local_position(&windows, &canvas_query) {
            if let Some(start) = port_center(&document, &session, dragging.from) {
                spawn_wire_segment(
                    &mut commands,
                    overlay_layer,
                    &mut registry.overlay_entities,
                    start,
                    pointer,
                    port_color(dragging.ty),
                    (WIRE_THICKNESS * session.zoom).clamp(1.5, 6.0),
                );
            }
        }
    }
}

fn current_canvas_cursor_local_position(
    windows: &Query<&Window, With<PrimaryWindow>>,
    canvas_query: &Query<(&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition), With<CanvasSurface>>,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    canvas_local_from_window_position(canvas_query, cursor)
}

fn canvas_local_from_window_position(
    canvas_query: &Query<(&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition), With<CanvasSurface>>,
    window_position: Vec2,
) -> Option<Vec2> {
    let (canvas_node, canvas_transform, relative_cursor) = canvas_query.single().ok()?;
    if !relative_cursor.cursor_over() {
        return None;
    }
    let normalized = canvas_node.normalize_point(*canvas_transform, window_position)?;
    if normalized.x < -0.5 || normalized.x > 0.5 || normalized.y < -0.5 || normalized.y > 0.5 {
        return None;
    }
    Some((normalized + Vec2::splat(0.5)) * canvas_node.size())
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

fn scaled(value: f32, zoom: f32) -> f32 {
    value * zoom.max(0.001)
}

fn scaled_font(value: f32, zoom: f32) -> f32 {
    scaled(value, zoom).max(6.0)
}

fn world_to_screen(point: Vec2, pan: Vec2, zoom: f32) -> Vec2 {
    pan + point * zoom.max(0.001)
}

fn screen_to_world(point: Vec2, pan: Vec2, zoom: f32) -> Vec2 {
    (point - pan) / zoom.max(0.001)
}

fn node_root_style(node: &GraphNode, pan: Vec2, zoom: f32) -> Node {
    let size = node_dimensions(node);
    let visual_position = world_to_screen(node.position, pan, zoom);
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(visual_position.x),
        top: Val::Px(visual_position.y),
        width: Val::Px(size.x * zoom),
        height: Val::Px(size.y * zoom),
        flex_direction: FlexDirection::Column,
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(scaled(12.0, zoom))),
        ..default()
    }
}

fn node_dimensions(node: &GraphNode) -> Vec2 {
    let row_count = node.inputs().len().max(node.outputs().len()).max(1) as f32;
    let body_rows = match node.node_type {
        NodeType::Agent => 18.0,
        _ => node.summary_lines().len() as f32,
    };
    let height = NODE_HEADER_HEIGHT
        + 16.0
        + (row_count * (PORT_ROW_HEIGHT + 4.0))
        + 12.0
        + (body_rows * 20.0);
    Vec2::new(NODE_WIDTH, height.max(170.0))
}

fn port_center(
    document: &GraphDocument,
    session: &EditorSession,
    address: PortAddress,
) -> Option<Vec2> {
    let node = document.node(address.node)?;
    let row = address.index as f32;
    let zoom = session.zoom.max(0.001);
    let visual_position = world_to_screen(node.position, session.pan, zoom);
    let y = visual_position.y
        + scaled(NODE_HEADER_HEIGHT, zoom)
        + scaled(10.0, zoom)
        + scaled(PORT_ROW_HEIGHT * 0.5, zoom)
        + row * scaled(PORT_ROW_HEIGHT + 4.0, zoom);
    let x = match address.direction {
        PortDirection::Input => visual_position.x + scaled(PORT_X_OFFSET, zoom),
        PortDirection::Output => visual_position.x + scaled(NODE_WIDTH - PORT_X_OFFSET, zoom),
    };
    Some(Vec2::new(x, y))
}

fn node_body_lines(
    document: &GraphDocument,
    node_id: NodeId,
    node: &GraphNode,
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

    if matches!(node.node_type, NodeType::Agent) {
        return agent_body_lines(document, node_id);
    }

    node.summary_lines()
}

fn inline_editor_value_text(editing: &TextEditingState) -> String {
    if editing.buffer.trim().is_empty() {
        "(empty)".into()
    } else {
        preview_multiline(&editing.buffer, 10).join("\n")
    }
}

fn filtered_node_templates(search: &str) -> Vec<NodeTemplate> {
    let needle = search.trim().to_ascii_lowercase();
    NodeTemplate::ALL
        .into_iter()
        .filter(|template| {
            needle.is_empty() || template.label().to_ascii_lowercase().contains(&needle)
        })
        .collect()
}

fn agent_body_lines(document: &GraphDocument, node_id: NodeId) -> Vec<String> {
    let mut lines = Vec::new();
    push_agent_group(
        &mut lines,
        "Core",
        [
            (
                "model",
                port_status(document, node_id, PortType::Model, true),
            ),
            (
                "prompt",
                port_status(document, node_id, PortType::Prompt, true),
            ),
            (
                "text output",
                if document
                    .output_targets(node_id, PortType::TextResponse)
                    .is_empty()
                {
                    "missing".into()
                } else {
                    format!(
                        "{} sink",
                        document
                            .output_targets(node_id, PortType::TextResponse)
                            .len()
                    )
                },
            ),
        ],
    );
    push_agent_group(
        &mut lines,
        "Identity",
        [
            (
                "name",
                port_status(document, node_id, PortType::AgentName, false),
            ),
            (
                "description",
                port_status(document, node_id, PortType::AgentDescription, false),
            ),
            (
                "preamble",
                port_status(document, node_id, PortType::Preamble, false),
            ),
        ],
    );
    push_agent_group(
        &mut lines,
        "Context",
        [
            (
                "static_context",
                multi_port_status(document, node_id, PortType::StaticContext),
            ),
            (
                "dynamic_context",
                multi_port_status(document, node_id, PortType::DynamicContext),
            ),
        ],
    );
    push_agent_group(
        &mut lines,
        "Sampling",
        [
            (
                "temperature",
                port_status(document, node_id, PortType::Temperature, false),
            ),
            (
                "max_tokens",
                port_status(document, node_id, PortType::MaxTokens, false),
            ),
            (
                "default_max_turns",
                port_status(document, node_id, PortType::DefaultMaxTurns, false),
            ),
        ],
    );
    push_agent_group(
        &mut lines,
        "Advanced",
        [
            (
                "additional_params",
                port_status(document, node_id, PortType::AdditionalParams, false),
            ),
            (
                "tool_server_handle",
                port_status(document, node_id, PortType::ToolServerHandle, false),
            ),
            (
                "hook",
                port_status(document, node_id, PortType::Hook, false),
            ),
            (
                "output_schema",
                port_status(document, node_id, PortType::OutputSchema, false),
            ),
        ],
    );
    lines
}

fn push_agent_group<const N: usize>(
    lines: &mut Vec<String>,
    title: &str,
    entries: [(&str, String); N],
) {
    lines.push(format!("[{title}]"));
    for (label, status) in entries {
        lines.push(format!("{label}: {status}"));
    }
}

fn port_status(document: &GraphDocument, node_id: NodeId, ty: PortType, required: bool) -> String {
    let count = document.input_sources(node_id, ty).len();
    if count == 0 {
        if required {
            "missing".into()
        } else {
            "optional".into()
        }
    } else {
        format!("{count} connected")
    }
}

fn multi_port_status(document: &GraphDocument, node_id: NodeId, ty: PortType) -> String {
    let count = document.input_sources(node_id, ty).len();
    if count == 0 {
        "optional".into()
    } else {
        format!("{count} connected")
    }
}

fn inspector_text(
    document: &GraphDocument,
    session: &EditorSession,
    runtime: &RigEditorRuntime,
) -> String {
    let Some(selected) = session.selected_node else {
        return format!(
            "No node selected.\n\nGraph\n• nodes: {}\n• edges: {}\n• zoom: {:.0}%\n\nRuntime\n• Ollama: {}\n• endpoint: {}\n• {}\n\nUse Space or right-click to add nodes.",
            document.node_count(),
            document.edge_count(),
            session.zoom * 100.0,
            if runtime.ollama_ready {
                "ready"
            } else {
                "offline"
            },
            runtime.ollama_endpoint,
            runtime.last_status
        );
    };

    let Some(node) = document.node(selected) else {
        return "Selection no longer exists.".into();
    };

    let incoming = node.inputs().len();
    let outgoing = node.outputs().len();
    let mut lines = vec![
        format!("Node\n• id: {}", node.id),
        format!("• kind: {}", node.title),
        format!("• position: {:.0}, {:.0}", node.position.x, node.position.y),
        format!("• input ports: {incoming}"),
        format!("• output ports: {outgoing}"),
    ];

    match &node.value {
        NodeValue::None if matches!(node.node_type, NodeType::Agent) => {
            lines.push(String::new());
            lines.push("Validation".into());
            match compile_agent_run(document, runtime, node.id) {
                Ok(run) => {
                    lines.push("• ready to run".into());
                    lines.push(format!("• model: {}", run.model));
                    if !run.warnings.is_empty() {
                        for warning in run.warnings {
                            lines.push(format!("• warning: {warning}"));
                        }
                    }
                }
                Err(error) => lines.push(format!("• {error}")),
            }
        }
        NodeValue::Model { model_name, .. } => {
            lines.push(String::new());
            lines.push("Provider".into());
            lines.push(format!(
                "• Ollama: {}",
                if runtime.ollama_ready {
                    "ready"
                } else {
                    "offline"
                }
            ));
            lines.push(format!("• endpoint: {}", runtime.ollama_endpoint));
            lines.push(format!(
                "• selected: {}",
                model_name.as_deref().unwrap_or("(none)")
            ));
            lines.push(format!("• discovered: {}", runtime.ollama_models.len()));
        }
        NodeValue::TextOutput { status, text } => {
            lines.push(String::new());
            lines.push("Output".into());
            lines.push(format!("• status: {status}"));
            lines.push("• preview:".into());
            lines.extend(preview_multiline(text, 8));
        }
        _ => {
            if let Some(text) = node.editable_text() {
                lines.push(String::new());
                lines.push("Content".into());
                lines.extend(preview_multiline(text, 8));
            }
        }
    }

    lines.push(String::new());
    lines.push("Runtime".into());
    lines.push(format!("• {}", runtime.last_status));
    lines.join("\n")
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
        PortType::Prompt | PortType::TextValue => Color::srgb_u8(245, 211, 61),
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
