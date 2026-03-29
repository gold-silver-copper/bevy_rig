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
    text::LineBreak,
    ui::{ComputedNode, RelativeCursorPosition, ui_transform::UiGlobalTransform},
    window::PrimaryWindow,
};

use crate::{
    catalog::{PortSpec, node_inputs, node_outputs},
    compile::compile_agent_run,
    graph::{
        DraggingWire, EditorSession, GraphDocument, GraphNode, NodeId, NodeTemplate, NodeType,
        NodeValue, PortAddress, PortDirection, PortType, ResizeCorner,
    },
    runtime::RigEditorRuntime,
};

const NODE_HEADER_HEIGHT: f32 = 34.0;
const NODE_PADDING: f32 = 12.0;
const PORT_ROW_HEIGHT: f32 = 28.0;
const PORT_DOT_SIZE: f32 = 12.0;
const PORT_ROW_GAP: f32 = 2.0;
const PORT_GROUP_PADDING_X: f32 = 6.0;
const PORTS_PADDING_Y: f32 = 8.0;
const AGENT_SECTION_ROW_HEIGHT: f32 = 24.0;
const AGENT_WIRE_ROW_HEIGHT: f32 = 24.0;
const PORT_CORE_SCALE_IDLE: f32 = 0.58;
const PORT_CORE_SCALE_HOVER: f32 = 0.66;
const PORT_CORE_SCALE_ACTIVE: f32 = 0.74;
const PORT_RING_SCALE: f32 = 1.34;
const PORT_GLOW_SCALE: f32 = 1.92;
const PORT_ANIMATION_SPEED: f32 = 18.0;
const RESIZE_HANDLE_HIT_SIZE: f32 = 18.0;
const WIRE_THICKNESS: f32 = 3.0;
const WIRE_SEGMENT_LENGTH: f32 = 30.0;
const WIRE_MIN_SEGMENTS: usize = 10;
const WIRE_MAX_SEGMENTS: usize = 22;
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
            .init_resource::<NodeContextMenuState>()
            .init_resource::<HoverHintState>()
            .add_systems(Startup, (setup_editor_ui, initialize_editor_session))
            .add_systems(
                Update,
                (
                    handle_palette_shortcuts,
                    handle_palette_buttons,
                    handle_context_menu_buttons,
                    handle_node_buttons,
                    handle_text_edit_input,
                    handle_canvas_zoom,
                    handle_hover_hints,
                    sync_node_views,
                    sync_palette_view,
                    sync_context_menu_view,
                    sync_hover_hint_view,
                    update_port_view_state,
                    update_node_view_state,
                    update_chrome_text,
                    rebuild_canvas_overlay,
                ),
            );
    }
}

#[derive(Resource, Clone)]
struct EditorFont(Handle<Font>);

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
    palette_parent: Option<Entity>,
    palette_view: Option<Entity>,
    context_menu_parent: Option<Entity>,
    context_menu_view: Option<Entity>,
    hover_hint_parent: Option<Entity>,
    hover_hint_view: Option<Entity>,
    node_views: HashMap<NodeId, Entity>,
    overlay_entities: Vec<Entity>,
    last_graph_revision: u64,
    last_palette_revision: u64,
    last_context_menu_revision: u64,
    last_hover_hint_revision: u64,
    last_zoom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditField {
    Title,
    Value,
}

#[derive(Resource, Default)]
struct TextEditingState {
    target: Option<(NodeId, EditField)>,
    buffer: String,
    revision: u64,
}

impl TextEditingState {
    fn clear(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.target = None;
        self.buffer.clear();
    }

    fn begin_if_needed(&mut self, target: NodeId, field: EditField, value: &str) {
        if self.target == Some((target, field)) {
            return;
        }
        self.target = Some((target, field));
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

#[derive(Resource, Default)]
struct NodeContextMenuState {
    visible: bool,
    screen_position: Vec2,
    node_id: Option<NodeId>,
    revision: u64,
}

impl NodeContextMenuState {
    fn open(&mut self, screen_position: Vec2, node_id: NodeId) {
        self.visible = true;
        self.screen_position = screen_position;
        self.node_id = Some(node_id);
        self.revision = self.revision.wrapping_add(1);
    }

    fn close(&mut self) {
        if self.visible || self.node_id.is_some() {
            self.visible = false;
            self.node_id = None;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

#[derive(Resource, Default)]
struct HoverHintState {
    visible: bool,
    text: String,
    screen_position: Vec2,
    revision: u64,
}

impl HoverHintState {
    fn show(&mut self, text: impl Into<String>, screen_position: Vec2) {
        self.visible = true;
        self.text = text.into();
        self.screen_position = screen_position;
        self.revision = self.revision.wrapping_add(1);
    }

    fn hide(&mut self) {
        if self.visible || !self.text.is_empty() {
            self.visible = false;
            self.text.clear();
            self.revision = self.revision.wrapping_add(1);
        }
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
struct NodeTitleText {
    id: NodeId,
}

#[derive(Component)]
struct NodeValueText {
    id: NodeId,
}

#[derive(Component, Clone, Copy)]
enum NodeAction {
    PreviousSetting(NodeId),
    NextSetting(NodeId),
    RefreshModels(NodeId),
    RunAgent(NodeId),
    StopRun(NodeId),
    ClearOutput(NodeId),
}

#[derive(Component)]
struct NodeActionButton(NodeAction);

#[derive(Component, Clone, Copy)]
struct ButtonHintText(&'static str);

#[derive(Component, Clone, Copy)]
struct PaletteButton(NodeTemplate);

#[derive(Component, Clone, Copy)]
enum ContextMenuAction {
    RenameNode(NodeId),
    DuplicateNode(NodeId),
    DeleteNode(NodeId),
}

#[derive(Component)]
struct ContextMenuButton(ContextMenuAction);

#[derive(Component, Clone, Copy)]
struct PortView {
    address: PortAddress,
    ty: PortType,
}

#[derive(Component, Clone, Copy)]
struct PortDecor {
    halo: Entity,
    ring: Entity,
    core: Entity,
}

#[derive(Component, Clone, Copy, Debug)]
struct PortVisualState {
    scale: f32,
    core_scale: f32,
    ring_alpha: f32,
    halo_alpha: f32,
    core_alpha: f32,
    brightness: f32,
}

impl Default for PortVisualState {
    fn default() -> Self {
        Self {
            scale: 1.0,
            core_scale: PORT_CORE_SCALE_IDLE,
            ring_alpha: 0.0,
            halo_alpha: 0.0,
            core_alpha: 1.0,
            brightness: 0.0,
        }
    }
}

fn select_node_for_editing(
    document: &mut GraphDocument,
    session: &mut EditorSession,
    editing: &mut TextEditingState,
    node_id: NodeId,
) {
    let selection_changed = session.selected_node != Some(node_id);
    session.select_node(Some(node_id));
    if let Some(value) = document
        .node(node_id)
        .and_then(GraphNode::editable_value_text)
    {
        editing.begin_if_needed(node_id, EditField::Value, &value);
    } else if selection_changed || editing.target.is_some() {
        editing.clear();
    }
}

fn begin_title_edit(document: &GraphDocument, editing: &mut TextEditingState, node_id: NodeId) {
    if let Some(node) = document.node(node_id) {
        editing.begin_if_needed(node_id, EditField::Title, &node.title);
    }
}

fn begin_wire_drag(document: &GraphDocument, session: &mut EditorSession, address: PortAddress) {
    let Some(spec) = document.port_spec(address) else {
        return;
    };
    let next = Some(DraggingWire {
        from: address,
        ty: spec.ty,
    });
    if session.dragging_wire != next {
        session.dragging_wire = next;
        session.touch();
    } else {
        session.dragging_wire = next;
    }
}

fn clear_wire_drag(session: &mut EditorSession) {
    if session.dragging_wire.take().is_some() {
        session.touch();
    }
}

fn try_connect_ports(
    document: &mut GraphDocument,
    session: &mut EditorSession,
    from: PortAddress,
    to: PortAddress,
) -> bool {
    if document.connect(from, to) {
        clear_wire_drag(session);
        session.hovered_port = Some(to);
        true
    } else {
        false
    }
}

fn apply_inline_edit(
    document: &mut GraphDocument,
    editing: &mut TextEditingState,
    node_id: NodeId,
    field: EditField,
) {
    match field {
        EditField::Title => {
            document.set_node_title_live(node_id, editing.buffer.clone());
        }
        EditField::Value => {
            document.set_node_inline_value_live(node_id, &editing.buffer);
        }
    }
    editing.mark_changed();
}

fn is_numeric_edit_target(document: &GraphDocument, node_id: NodeId) -> bool {
    matches!(
        document.node(node_id).map(|node| &node.value),
        Some(NodeValue::Temperature(_) | NodeValue::U64(_))
    )
}

fn setup_editor_ui(
    mut commands: Commands,
    mut registry: ResMut<GraphUiRegistry>,
    asset_server: Res<AssetServer>,
) {
    commands.insert_resource(EditorFont(
        asset_server.load("fonts/JetBrainsMono-Regular.ttf"),
    ));
    commands.spawn(Camera2d);

    let root = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgb_u8(20, 21, 24)),
        ))
        .id();

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
             mut context_menu: ResMut<NodeContextMenuState>,
             canvas_query: Query<
                (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
                With<CanvasSurface>,
            >| {
                if click.button == PointerButton::Primary {
                    click.propagate(false);
                    session.select_node(None);
                    session.hovered_port = None;
                    session.dragging_wire = None;
                    session.touch();
                    editing.clear();
                    palette.close();
                    context_menu.close();
                } else if click.button == PointerButton::Secondary {
                    click.propagate(false);
                    if let Some(screen_position) = canvas_local_from_window_position(
                        &canvas_query,
                        click.pointer_location.position,
                    ) {
                        let spawn_world =
                            screen_to_world(screen_position, session.pan, session.zoom);
                        palette.open(screen_position, spawn_world);
                        context_menu.close();
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
    commands.entity(root).add_child(canvas);

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
    registry.palette_parent = Some(canvas);
    registry.context_menu_parent = Some(canvas);
    registry.hover_hint_parent = Some(canvas);
}

fn editor_text_font(fonts: &EditorFont, size: f32) -> TextFont {
    TextFont {
        font: fonts.0.clone(),
        font_size: size,
        ..default()
    }
}

fn node_corner_radius(zoom: f32) -> Val {
    Val::Px(scaled(12.0, zoom))
}

fn spawn_scaled_node_action_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: NodeAction,
    fonts: &EditorFont,
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
            ButtonHintText(node_action_hint(action)),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .with_child((
            Text::new(label),
            editor_text_font(fonts, scaled_font(12.0, zoom)),
            TextColor(Color::srgb_u8(232, 234, 238)),
        ));
}

fn spawn_compact_node_action_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: NodeAction,
    fonts: &EditorFont,
    zoom: f32,
) {
    parent
        .spawn((
            Button,
            Node {
                min_width: Val::Px(scaled(28.0, zoom)),
                min_height: Val::Px(scaled(28.0, zoom)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Val::Px(scaled(6.0, zoom)), Val::Px(scaled(4.0, zoom))),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(scaled(8.0, zoom))),
                ..default()
            },
            NodeActionButton(action),
            ButtonHintText(node_action_hint(action)),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .with_child((
            Text::new(label),
            editor_text_font(fonts, scaled_font(12.0, zoom)),
            TextColor(Color::srgb_u8(232, 234, 238)),
        ));
}

fn spawn_palette_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    template: NodeTemplate,
    fonts: &EditorFont,
) {
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
            ButtonHintText("Add this node to the canvas"),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .with_child((
            Text::new(label),
            editor_text_font(fonts, 13.0),
            TextColor(Color::srgb_u8(232, 234, 238)),
            Pickable::IGNORE,
        ));
}

fn spawn_context_menu_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: ContextMenuAction,
    fonts: &EditorFont,
) {
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
            ContextMenuButton(action),
            ButtonHintText(context_menu_hint(action)),
            BackgroundColor(button_background(Interaction::None)),
            BorderColor::all(Color::srgb_u8(62, 64, 70)),
        ))
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .with_child((
            Text::new(label),
            editor_text_font(fonts, 13.0),
            TextColor(Color::srgb_u8(232, 234, 238)),
            Pickable::IGNORE,
        ));
}

fn node_action_hint(action: NodeAction) -> &'static str {
    match action {
        NodeAction::PreviousSetting(_) => "Decrease or go to previous value",
        NodeAction::NextSetting(_) => "Increase or go to next value",
        NodeAction::RefreshModels(_) => "Refresh local Ollama models",
        NodeAction::RunAgent(_) => "Run this agent graph",
        NodeAction::StopRun(_) => "Stop the current run",
        NodeAction::ClearOutput(_) => "Clear this output node",
    }
}

fn context_menu_hint(action: ContextMenuAction) -> &'static str {
    match action {
        ContextMenuAction::RenameNode(_) => "Rename this node",
        ContextMenuAction::DuplicateNode(_) => "Duplicate this node",
        ContextMenuAction::DeleteNode(_) => "Delete this node",
    }
}

fn handle_node_buttons(
    mut document: ResMut<GraphDocument>,
    mut session: ResMut<EditorSession>,
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
            NodeAction::PreviousSetting(node_id) => {
                session.select_node(Some(node_id));
                if !document.cycle_setting(node_id, -1, &runtime.ollama_models) {
                    runtime.last_status = "This node has no previous setting to cycle.".into();
                } else if editing.target == Some((node_id, EditField::Value))
                    && let Some(value) = document
                        .node(node_id)
                        .and_then(GraphNode::editable_value_text)
                {
                    editing.buffer = value;
                    editing.mark_changed();
                }
            }
            NodeAction::NextSetting(node_id) => {
                session.select_node(Some(node_id));
                if !document.cycle_setting(node_id, 1, &runtime.ollama_models) {
                    runtime.last_status = "This node has no next setting to cycle.".into();
                } else if editing.target == Some((node_id, EditField::Value))
                    && let Some(value) = document
                        .node(node_id)
                        .and_then(GraphNode::editable_value_text)
                {
                    editing.buffer = value;
                    editing.mark_changed();
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
            NodeAction::StopRun(node_id) => {
                session.select_node(Some(node_id));
                if let Some(output_node) = runtime.stop_run() {
                    document.set_output_result(
                        output_node,
                        "Run stopped.".into(),
                        "stopped".into(),
                    );
                }
            }
            NodeAction::ClearOutput(node_id) => {
                if document.clear_output(node_id) {
                    runtime.last_status = "Cleared text output.".into();
                }
            }
        }
    }
}

fn handle_text_edit_input(
    mut key_events: MessageReader<KeyboardInput>,
    mut editing: ResMut<TextEditingState>,
    mut document: ResMut<GraphDocument>,
) {
    let Some((target, field)) = editing.target else {
        return;
    };

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
                apply_inline_edit(&mut document, &mut editing, target, field);
                continue;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if matches!(field, EditField::Title) || is_numeric_edit_target(&document, target) {
                    editing.clear();
                    return;
                }
                editing.buffer.push('\n');
                apply_inline_edit(&mut document, &mut editing, target, field);
                continue;
            }
            KeyCode::Tab => continue,
            _ => {}
        }

        if let Some(text) = &event.text {
            editing.buffer.push_str(text);
            apply_inline_edit(&mut document, &mut editing, target, field);
        }
    }
}

fn handle_canvas_zoom(
    mut wheel_events: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<EditorSession>,
    canvas_query: Query<
        (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
        With<CanvasSurface>,
    >,
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
    mut context_menu: ResMut<NodeContextMenuState>,
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
                        context_menu.close();
                    }
                    continue;
                }
                _ => continue,
            }
        }

        match event.key_code {
            KeyCode::Escape => {
                palette.close();
                context_menu.close();
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
                    context_menu.close();
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
    mut context_menu: ResMut<NodeContextMenuState>,
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
        context_menu.close();
    }
}

fn handle_context_menu_buttons(
    mut document: ResMut<GraphDocument>,
    mut session: ResMut<EditorSession>,
    mut editing: ResMut<TextEditingState>,
    mut palette: ResMut<NodePaletteState>,
    mut context_menu: ResMut<NodeContextMenuState>,
    mut runtime: ResMut<RigEditorRuntime>,
    mut interactions: Query<
        (&Interaction, &ContextMenuButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut background) in &mut interactions {
        background.0 = button_background(*interaction);
        if *interaction != Interaction::Pressed {
            continue;
        }

        match button.0 {
            ContextMenuAction::RenameNode(node_id) => {
                session.select_node(Some(node_id));
                begin_title_edit(&document, &mut editing, node_id);
            }
            ContextMenuAction::DuplicateNode(node_id) => {
                session.select_node(Some(node_id));
                if let Some(duplicate) = document.duplicate_node(node_id, Vec2::new(36.0, 36.0)) {
                    select_node_for_editing(&mut document, &mut session, &mut editing, duplicate);
                    runtime.last_status = "Duplicated node.".into();
                }
            }
            ContextMenuAction::DeleteNode(node_id) => {
                let was_editing = matches!(editing.target, Some((target, _)) if target == node_id);
                if document.remove_node(node_id) {
                    if session.selected_node == Some(node_id) {
                        session.select_node(None);
                    }
                    if matches!(session.dragging_wire, Some(DraggingWire { from, .. }) if from.node == node_id)
                    {
                        session.dragging_wire = None;
                        session.touch();
                    }
                    if matches!(session.hovered_port, Some(address) if address.node == node_id) {
                        session.hovered_port = None;
                    }
                    if was_editing {
                        editing.clear();
                    }
                    runtime.last_status = "Deleted node.".into();
                }
            }
        }

        palette.close();
        context_menu.close();
    }
}

fn handle_hover_hints(
    mut hint: ResMut<HoverHintState>,
    hint_buttons: Query<(&Interaction, &ButtonHintText), With<Button>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    canvas_query: Query<
        (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
        With<CanvasSurface>,
    >,
) {
    let hovered = hint_buttons
        .iter()
        .find_map(|(interaction, text)| (*interaction == Interaction::Hovered).then_some(text.0));

    if let Some(text) = hovered {
        if let Some(position) = current_canvas_cursor_local_position(&windows, &canvas_query) {
            hint.show(text, position + Vec2::new(14.0, 14.0));
            return;
        }
    }

    hint.hide();
}

fn sync_palette_view(
    mut commands: Commands,
    palette: Res<NodePaletteState>,
    mut registry: ResMut<GraphUiRegistry>,
    canvas_query: Query<&ComputedNode, With<CanvasSurface>>,
    fonts: Res<EditorFont>,
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
                    editor_text_font(&fonts, 16.0),
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
                            editor_text_font(&fonts, 13.0),
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
                        editor_text_font(&fonts, 12.0),
                        TextColor(Color::srgb_u8(150, 156, 167)),
                        Pickable::IGNORE,
                    ));
                } else {
                    for template in matches.into_iter().take(10) {
                        spawn_palette_button(parent, template.label(), template, &fonts);
                    }
                }

                parent.spawn((
                    Text::new("Enter adds the first result • Esc closes"),
                    editor_text_font(&fonts, 11.0),
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

fn sync_context_menu_view(
    mut commands: Commands,
    context_menu: Res<NodeContextMenuState>,
    document: Res<GraphDocument>,
    mut registry: ResMut<GraphUiRegistry>,
    canvas_query: Query<&ComputedNode, With<CanvasSurface>>,
    fonts: Res<EditorFont>,
) {
    if registry.last_context_menu_revision == context_menu.revision {
        return;
    }

    if let Some(entity) = registry.context_menu_view.take() {
        commands.entity(entity).despawn();
    }

    if context_menu.visible {
        let Some(parent) = registry.context_menu_parent else {
            return;
        };
        let canvas_size = canvas_query
            .get(parent)
            .map(ComputedNode::size)
            .unwrap_or(Vec2::new(1200.0, 900.0));
        let max_left = (canvas_size.x - 220.0 - 18.0).max(18.0);
        let max_top = (canvas_size.y - 160.0).max(18.0);
        let node_label = context_menu
            .node_id
            .and_then(|node_id| document.node(node_id))
            .map(|node| node.title.clone())
            .unwrap_or_else(|| "Node".into());

        let panel = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(context_menu.screen_position.x.clamp(18.0, max_left)),
                    top: Val::Px(context_menu.screen_position.y.clamp(18.0, max_top)),
                    width: Val::Px(220.0),
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
                    Text::new(node_label),
                    editor_text_font(&fonts, 15.0),
                    TextColor(Color::srgb_u8(236, 238, 241)),
                    Pickable::IGNORE,
                ));

                if let Some(node_id) = context_menu.node_id {
                    spawn_context_menu_button(
                        parent,
                        "Rename",
                        ContextMenuAction::RenameNode(node_id),
                        &fonts,
                    );
                    spawn_context_menu_button(
                        parent,
                        "Duplicate",
                        ContextMenuAction::DuplicateNode(node_id),
                        &fonts,
                    );
                    spawn_context_menu_button(
                        parent,
                        "Delete",
                        ContextMenuAction::DeleteNode(node_id),
                        &fonts,
                    );
                }
            })
            .id();
        commands.entity(parent).add_child(panel);
        registry.context_menu_view = Some(panel);
    }

    registry.last_context_menu_revision = context_menu.revision;
}

fn sync_hover_hint_view(
    mut commands: Commands,
    hint: Res<HoverHintState>,
    mut registry: ResMut<GraphUiRegistry>,
    canvas_query: Query<&ComputedNode, With<CanvasSurface>>,
    fonts: Res<EditorFont>,
) {
    if registry.last_hover_hint_revision == hint.revision {
        return;
    }

    if let Some(entity) = registry.hover_hint_view.take() {
        commands.entity(entity).despawn();
    }

    if hint.visible {
        let Some(parent) = registry.hover_hint_parent else {
            return;
        };
        let canvas_size = canvas_query
            .get(parent)
            .map(ComputedNode::size)
            .unwrap_or(Vec2::new(1200.0, 900.0));
        let estimated_width = (hint.text.chars().count() as f32 * 7.6 + 20.0).clamp(120.0, 320.0);
        let left = hint
            .screen_position
            .x
            .clamp(12.0, (canvas_size.x - estimated_width - 12.0).max(12.0));
        let top = hint
            .screen_position
            .y
            .clamp(12.0, (canvas_size.y - 34.0).max(12.0));

        let panel = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(left),
                    top: Val::Px(top),
                    max_width: Val::Px(320.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.82)),
                GlobalZIndex(60),
                Pickable::IGNORE,
            ))
            .with_child((
                Text::new(hint.text.clone()),
                editor_text_font(&fonts, 12.0),
                TextColor(Color::srgb_u8(236, 238, 241)),
                Pickable::IGNORE,
            ))
            .id();
        commands.entity(parent).add_child(panel);
        registry.hover_hint_view = Some(panel);
    }

    registry.last_hover_hint_revision = hint.revision;
}

fn sync_node_views(
    mut commands: Commands,
    document: Res<GraphDocument>,
    session: Res<EditorSession>,
    editing: Res<TextEditingState>,
    mut registry: ResMut<GraphUiRegistry>,
    fonts: Res<EditorFont>,
) {
    let Some(node_layer) = registry.node_layer else {
        return;
    };

    if registry.last_graph_revision == document.revision
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
        let is_title_editing = editing.target == Some((node_id, EditField::Title));
        let is_value_editing = editing.target == Some((node_id, EditField::Value))
            && node_snapshot.editable_value_text().is_some();
        let zoom = session.zoom.max(0.001);
        let header_height = scaled(NODE_HEADER_HEIGHT, zoom);
        let node_padding = scaled(NODE_PADDING, zoom);
        let port_row_height = scaled(PORT_ROW_HEIGHT, zoom);
        let is_model_node = matches!(node_snapshot.node_type, NodeType::Model);
        let is_agent_node = matches!(node_snapshot.node_type, NodeType::Agent);
        let is_output_node = matches!(node_snapshot.node_type, NodeType::TextOutput);
        let is_tool_choice_node = matches!(node_snapshot.node_type, NodeType::ToolChoice);
        let is_numeric_node = matches!(
            node_snapshot.node_type,
            NodeType::Temperature | NodeType::U64
        );

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
                      mut editing: ResMut<TextEditingState>,
                      mut palette: ResMut<NodePaletteState>,
                      mut context_menu: ResMut<NodeContextMenuState>,
                      canvas_query: Query<
                    (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
                    With<CanvasSurface>,
                >| {
                    if click.button == PointerButton::Primary {
                        click.propagate(false);
                        select_node_for_editing(&mut document, &mut session, &mut editing, node_id);
                        palette.close();
                        context_menu.close();
                    } else if click.button == PointerButton::Secondary {
                        click.propagate(false);
                        session.select_node(Some(node_id));
                        palette.close();
                        if let Some(screen_position) = canvas_local_from_window_position(
                            &canvas_query,
                            click.pointer_location.position,
                        ) {
                            context_menu.open(screen_position, node_id);
                        }
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
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(
                            Val::Px(scaled(12.0, zoom)),
                            Val::Px(scaled(8.0, zoom)),
                        ),
                        border: UiRect::bottom(Val::Px(1.0)),
                        border_radius: BorderRadius::top(node_corner_radius(zoom)),
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
                          mut editing: ResMut<TextEditingState>,
                          mut palette: ResMut<NodePaletteState>,
                          mut context_menu: ResMut<NodeContextMenuState>,
                          canvas_query: Query<
                        (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
                        With<CanvasSurface>,
                    >| {
                        if click.button == PointerButton::Primary {
                            click.propagate(false);
                            select_node_for_editing(
                                &mut document,
                                &mut session,
                                &mut editing,
                                node_id,
                            );
                            palette.close();
                            context_menu.close();
                        } else if click.button == PointerButton::Secondary {
                            click.propagate(false);
                            session.select_node(Some(node_id));
                            palette.close();
                            if let Some(screen_position) = canvas_local_from_window_position(
                                &canvas_query,
                                click.pointer_location.position,
                            ) {
                                context_menu.open(screen_position, node_id);
                            }
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
                .with_children(|header| {
                    header
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                min_width: Val::Px(0.0),
                                padding: UiRect::right(Val::Px(scaled(10.0, zoom))),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_child((
                            Text::new(if is_title_editing {
                                editing_text_with_cursor(&editing.buffer)
                            } else {
                                node_snapshot.title.clone()
                            }),
                            NodeTitleText { id: node_id },
                            editor_text_font(&fonts, scaled_font(15.0, zoom)),
                            TextColor(Color::srgb_u8(238, 240, 243)),
                            Pickable::IGNORE,
                        ));

                    header
                        .spawn((
                            Node {
                                display: Display::Flex,
                                align_items: AlignItems::Center,
                                flex_shrink: 0.0,
                                column_gap: Val::Px(scaled(6.0, zoom)),
                                ..default()
                            },
                            Pickable::IGNORE,
                        ))
                        .with_children(|actions| {
                            if is_model_node {
                                spawn_scaled_node_action_button(
                                    actions,
                                    "Ref",
                                    NodeAction::RefreshModels(node_id),
                                    &fonts,
                                    zoom,
                                );
                                spawn_scaled_node_action_button(
                                    actions,
                                    "‹",
                                    NodeAction::PreviousSetting(node_id),
                                    &fonts,
                                    zoom,
                                );
                                spawn_scaled_node_action_button(
                                    actions,
                                    "›",
                                    NodeAction::NextSetting(node_id),
                                    &fonts,
                                    zoom,
                                );
                            } else if is_tool_choice_node {
                                spawn_scaled_node_action_button(
                                    actions,
                                    "‹",
                                    NodeAction::PreviousSetting(node_id),
                                    &fonts,
                                    zoom,
                                );
                                spawn_scaled_node_action_button(
                                    actions,
                                    "›",
                                    NodeAction::NextSetting(node_id),
                                    &fonts,
                                    zoom,
                                );
                            } else if is_agent_node {
                                spawn_scaled_node_action_button(
                                    actions,
                                    "Run",
                                    NodeAction::RunAgent(node_id),
                                    &fonts,
                                    zoom,
                                );
                                spawn_scaled_node_action_button(
                                    actions,
                                    "Stop",
                                    NodeAction::StopRun(node_id),
                                    &fonts,
                                    zoom,
                                );
                            } else if is_output_node {
                                spawn_scaled_node_action_button(
                                    actions,
                                    "Clear",
                                    NodeAction::ClearOutput(node_id),
                                    &fonts,
                                    zoom,
                                );
                            }
                        });
                });

            let inputs = node_snapshot.inputs();
            let outputs = node_snapshot.outputs();
            let agent_rows = if is_agent_node {
                Some(agent_wire_rows(&document, node_id, &node_snapshot))
            } else {
                None
            };
            let row_count = agent_rows
                .as_ref()
                .map(|rows| rows.len())
                .unwrap_or_else(|| inputs.len().max(outputs.len()).max(1));

            parent
                .spawn(Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(node_padding), Val::Px(scaled(8.0, zoom))),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(scaled(2.0, zoom)),
                    border_radius: if is_agent_node {
                        BorderRadius::bottom(node_corner_radius(zoom))
                    } else {
                        BorderRadius::ZERO
                    },
                    overflow: if is_agent_node {
                        Overflow::clip()
                    } else {
                        Overflow::DEFAULT
                    },
                    ..default()
                })
                .insert(BackgroundColor(Color::srgb_u8(46, 47, 52)))
                .insert(Pickable::IGNORE)
                .with_children(|ports| {
                    if let Some(rows) = &agent_rows {
                        for row in rows {
                            spawn_agent_wire_row(ports, row, node_id, &fonts, zoom);
                        }
                    } else {
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
                                                padding: UiRect::axes(
                                                    Val::Px(scaled(6.0, zoom)),
                                                    Val::Px(scaled(3.0, zoom)),
                                                ),
                                                ..default()
                                            })
                                            .insert(Pickable::IGNORE)
                                            .with_children(|group| {
                                                let address = PortAddress {
                                                    node: node_id,
                                                    direction: PortDirection::Input,
                                                    index: row,
                                                };
                                                spawn_port_button(group, address, spec.ty, zoom);
                                                group.spawn((
                                                    Text::new(format!(
                                                        "{}{}",
                                                        spec.name,
                                                        if spec.required { " *" } else { "" }
                                                    )),
                                                    editor_text_font(
                                                        &fonts,
                                                        scaled_font(13.0, zoom),
                                                    ),
                                                    TextColor(Color::srgb_u8(236, 238, 241)),
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
                                                padding: UiRect::axes(
                                                    Val::Px(scaled(6.0, zoom)),
                                                    Val::Px(scaled(3.0, zoom)),
                                                ),
                                                ..default()
                                            })
                                            .insert(Pickable::IGNORE)
                                            .with_children(|group| {
                                                group.spawn((
                                                    Text::new(spec.name),
                                                    editor_text_font(
                                                        &fonts,
                                                        scaled_font(13.0, zoom),
                                                    ),
                                                    TextColor(Color::srgb_u8(236, 238, 241)),
                                                    Pickable::IGNORE,
                                                ));
                                                let address = PortAddress {
                                                    node: node_id,
                                                    direction: PortDirection::Output,
                                                    index: row,
                                                };
                                                spawn_port_button(group, address, spec.ty, zoom);
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
                    }
                });

            if !is_agent_node {
                parent
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_grow: 1.0,
                            padding: UiRect::all(Val::Px(scaled(10.0, zoom))),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(scaled(4.0, zoom)),
                            border: UiRect::top(Val::Px(1.0)),
                            border_radius: BorderRadius::bottom(node_corner_radius(zoom)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BorderColor {
                            top: Color::srgb_u8(77, 79, 86),
                            ..BorderColor::DEFAULT
                        },
                        BackgroundColor(Color::srgb_u8(46, 47, 52)),
                        Pickable::IGNORE,
                    ))
                    .with_children(|body| {
                        if is_numeric_node {
                            spawn_numeric_value_surface(
                                body,
                                node_id,
                                &node_snapshot,
                                is_value_editing,
                                &editing,
                                &fonts,
                                zoom,
                            );
                        } else if node_snapshot.editable_value_text().is_some() {
                            spawn_editable_value_surface(
                                body,
                                node_id,
                                &node_snapshot,
                                is_value_editing,
                                &editing,
                                &fonts,
                                zoom,
                            );
                        } else {
                            for line in node_body_lines(&node_snapshot, is_value_editing, &editing)
                            {
                                body.spawn((
                                    Text::new(line),
                                    editor_text_font(&fonts, scaled_font(13.0, zoom)),
                                    TextColor(Color::srgb_u8(240, 242, 245)),
                                    Pickable::IGNORE,
                                ));
                            }
                        }
                    });
            }

            for corner in [
                ResizeCorner::NorthWest,
                ResizeCorner::NorthEast,
                ResizeCorner::SouthWest,
                ResizeCorner::SouthEast,
            ] {
                spawn_resize_handle(parent, node_id, corner, zoom);
            }
        });

        registry.node_views.insert(node_id, root);
    }

    registry.last_graph_revision = document.revision;
    registry.last_zoom = session.zoom;
}

fn spawn_port_button(
    parent: &mut ChildSpawnerCommands,
    address: PortAddress,
    ty: PortType,
    zoom: f32,
) {
    let size = scaled(PORT_DOT_SIZE, zoom);
    let mut halo = None;
    let mut ring = None;
    let mut core = None;

    let mut port = parent.spawn((
        Button,
        PortView { address, ty },
        PortVisualState::default(),
        Node {
            width: Val::Px(size),
            height: Val::Px(size),
            overflow: Overflow::visible(),
            ..default()
        },
        UiTransform::default(),
        BackgroundColor(Color::NONE),
        BorderColor::all(Color::NONE),
        ZIndex(2),
    ));

    port.with_children(|children| {
        halo = Some(
            children
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Px(size),
                        height: Val::Px(size),
                        border_radius: BorderRadius::all(Val::Px(size * 0.5)),
                        ..default()
                    },
                    UiTransform::default(),
                    BackgroundColor(Color::NONE),
                    Pickable::IGNORE,
                ))
                .id(),
        );
        ring = Some(
            children
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Px(size),
                        height: Val::Px(size),
                        border: UiRect::all(Val::Px(scaled(1.4, zoom))),
                        border_radius: BorderRadius::all(Val::Px(size * 0.5)),
                        ..default()
                    },
                    UiTransform::default(),
                    BackgroundColor(Color::NONE),
                    BorderColor::all(Color::NONE),
                    Pickable::IGNORE,
                ))
                .id(),
        );
        core = Some(
            children
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Px(size),
                        height: Val::Px(size),
                        border: UiRect::all(Val::Px(scaled(0.9, zoom))),
                        border_radius: BorderRadius::all(Val::Px(size * 0.5)),
                        ..default()
                    },
                    UiTransform {
                        scale: Vec2::splat(PORT_CORE_SCALE_IDLE),
                        ..default()
                    },
                    BackgroundColor(port_color(ty)),
                    BorderColor::all(Color::BLACK.with_alpha(0.7)),
                    Pickable::IGNORE,
                ))
                .id(),
        );
    });

    port.insert(PortDecor {
        halo: halo.expect("port halo child"),
        ring: ring.expect("port ring child"),
        core: core.expect("port core child"),
    });

    port.observe(
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
                    clear_wire_drag(&mut session);
                    return;
                }

                if try_connect_ports(&mut document, &mut session, dragging.from, address) {
                    return;
                }
            }

            begin_wire_drag(&document, &mut session, address);
        },
    )
    .observe(
        move |mut drag_start: On<Pointer<DragStart>>,
              mut document: ResMut<GraphDocument>,
              mut session: ResMut<EditorSession>,
              mut editing: ResMut<TextEditingState>| {
            if drag_start.button != PointerButton::Primary {
                return;
            }

            drag_start.propagate(false);
            select_node_for_editing(&mut document, &mut session, &mut editing, address.node);
            begin_wire_drag(&document, &mut session, address);
        },
    )
    .observe(
        move |mut drag_drop: On<Pointer<DragDrop>>,
              mut document: ResMut<GraphDocument>,
              mut session: ResMut<EditorSession>| {
            if drag_drop.button != PointerButton::Primary {
                return;
            }

            drag_drop.propagate(false);
            let Some(dragging) = session.dragging_wire else {
                return;
            };

            try_connect_ports(&mut document, &mut session, dragging.from, address);
        },
    )
    .observe(
        move |mut drag_end: On<Pointer<DragEnd>>, mut session: ResMut<EditorSession>| {
            if drag_end.button != PointerButton::Primary {
                return;
            }

            drag_end.propagate(false);
            if matches!(session.dragging_wire, Some(DraggingWire { from, .. }) if from == address) {
                clear_wire_drag(&mut session);
            }
        },
    )
    .observe(
        move |_: On<Pointer<Over>>, mut session: ResMut<EditorSession>| {
            session.hovered_port = Some(address);
        },
    )
    .observe(
        move |_: On<Pointer<Out>>, mut session: ResMut<EditorSession>| {
            if session.hovered_port == Some(address) {
                session.hovered_port = None;
            }
        },
    )
    .observe(
        move |drag_enter: On<Pointer<DragEnter>>, mut session: ResMut<EditorSession>| {
            if drag_enter.button == PointerButton::Primary {
                session.hovered_port = Some(address);
            }
        },
    )
    .observe(
        move |drag_leave: On<Pointer<DragLeave>>, mut session: ResMut<EditorSession>| {
            if drag_leave.button == PointerButton::Primary && session.hovered_port == Some(address)
            {
                session.hovered_port = None;
            }
        },
    );
}

fn update_port_view_state(
    time: Res<Time>,
    document: Res<GraphDocument>,
    session: Res<EditorSession>,
    mut ports: Query<
        (
            &PortView,
            &Interaction,
            &PortDecor,
            &mut PortVisualState,
            &mut UiTransform,
            &mut ZIndex,
        ),
        With<Button>,
    >,
    mut child_backgrounds: Query<&mut BackgroundColor, Without<PortView>>,
    mut child_borders: Query<&mut BorderColor, Without<PortView>>,
    mut child_transforms: Query<&mut UiTransform, Without<PortView>>,
) {
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 8.0).sin();
    let blend = 1.0 - (-PORT_ANIMATION_SPEED * time.delta_secs()).exp();

    for (port, interaction, decor, mut visual, mut transform, mut z_index) in &mut ports {
        let hovered =
            session.hovered_port == Some(port.address) || *interaction == Interaction::Hovered;
        let connected = port_is_connected(&document, port.address);
        let selected = session.selected_node == Some(port.address.node);
        let dragging = session.dragging_wire;
        let is_source = dragging.is_some_and(|wire| wire.from == port.address);
        let compatible =
            dragging.is_some_and(|wire| port_can_connect(&document, wire.from, port.address));
        let hovered_compatible = hovered && compatible;
        let incompatible = dragging.is_some() && !is_source && !compatible;

        let target_scale = if is_source {
            1.36 + pulse * 0.12
        } else if hovered_compatible {
            1.28 + pulse * 0.05
        } else if hovered {
            1.22
        } else if compatible {
            1.08
        } else if incompatible {
            0.92
        } else {
            1.0
        };
        let target_core_scale = if is_source {
            PORT_CORE_SCALE_ACTIVE
        } else if hovered || hovered_compatible {
            PORT_CORE_SCALE_HOVER
        } else {
            PORT_CORE_SCALE_IDLE
        };
        let target_ring_alpha = if is_source {
            0.82
        } else if hovered_compatible {
            0.58
        } else if hovered {
            0.42
        } else if compatible {
            0.16
        } else if connected && selected {
            0.18
        } else {
            0.0
        };
        let target_halo_alpha = if is_source {
            0.32 + pulse * 0.08
        } else if hovered_compatible {
            0.20 + pulse * 0.05
        } else if compatible {
            0.08
        } else {
            0.0
        };
        let target_core_alpha = if incompatible { 0.3 } else { 1.0 };
        let target_brightness = if is_source {
            0.22 + pulse * 0.08
        } else if hovered_compatible {
            0.18
        } else if hovered {
            0.12
        } else if compatible {
            0.08
        } else {
            0.0
        };

        visual.scale = visual.scale.lerp(target_scale, blend);
        visual.core_scale = visual.core_scale.lerp(target_core_scale, blend);
        visual.ring_alpha = visual.ring_alpha.lerp(target_ring_alpha, blend);
        visual.halo_alpha = visual.halo_alpha.lerp(target_halo_alpha, blend);
        visual.core_alpha = visual.core_alpha.lerp(target_core_alpha, blend);
        visual.brightness = visual.brightness.lerp(target_brightness, blend);

        transform.scale = Vec2::splat(visual.scale);
        z_index.0 = if is_source {
            5
        } else if hovered_compatible {
            4
        } else if hovered {
            3
        } else {
            2
        };

        let base = port_color(port.ty);
        let ring_color = if incompatible {
            base.darker(0.28).with_alpha(0.05)
        } else {
            base.lighter(0.2 + visual.brightness * 0.35)
                .with_alpha(visual.ring_alpha)
        };
        let halo_color = if incompatible {
            Color::NONE
        } else {
            base.lighter(0.24 + visual.brightness * 0.45)
                .with_alpha(visual.halo_alpha)
        };
        let core_color = if incompatible {
            base.darker(0.16).with_alpha(visual.core_alpha)
        } else {
            base.lighter(visual.brightness)
                .with_alpha(visual.core_alpha)
        };

        if let Ok(mut halo_transform) = child_transforms.get_mut(decor.halo) {
            halo_transform.scale = Vec2::splat(PORT_GLOW_SCALE + pulse * 0.06);
        }
        if let Ok(mut ring_transform) = child_transforms.get_mut(decor.ring) {
            ring_transform.scale = Vec2::splat(
                PORT_RING_SCALE
                    + if hovered_compatible || is_source {
                        pulse * 0.04
                    } else {
                        0.0
                    },
            );
        }
        if let Ok(mut core_transform) = child_transforms.get_mut(decor.core) {
            core_transform.scale = Vec2::splat(visual.core_scale);
        }

        if let Ok(mut halo_bg) = child_backgrounds.get_mut(decor.halo) {
            halo_bg.0 = halo_color;
        }
        if let Ok(mut core_bg) = child_backgrounds.get_mut(decor.core) {
            core_bg.0 = core_color;
        }
        if let Ok(mut ring_border) = child_borders.get_mut(decor.ring) {
            ring_border.set_all(ring_color);
        }
        if let Ok(mut core_border) = child_borders.get_mut(decor.core) {
            core_border.set_all(if incompatible {
                base.darker(0.25).with_alpha(0.45)
            } else {
                core_color.darker(0.22).with_alpha(0.92)
            });
        }
    }
}

fn port_can_connect(document: &GraphDocument, source: PortAddress, candidate: PortAddress) -> bool {
    if source == candidate {
        return false;
    }

    match source.direction {
        PortDirection::Output => document.can_connect(source, candidate),
        PortDirection::Input => document.can_connect(candidate, source),
    }
}

fn port_is_connected(document: &GraphDocument, address: PortAddress) -> bool {
    document
        .iter_edges()
        .any(|edge| edge.from == address || edge.to == address)
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
    editing: Res<TextEditingState>,
    mut titles: Query<(&NodeTitleText, &mut Text)>,
    mut values: Query<(&NodeValueText, &mut Text)>,
) {
    for (label, mut text) in &mut titles {
        let Some(node) = document.node(label.id) else {
            continue;
        };

        let next = if editing.target == Some((label.id, EditField::Title)) {
            editing_text_with_cursor(&editing.buffer)
        } else {
            node.title.clone()
        };

        if *text != Text::new(next.clone()) {
            *text = Text::new(next);
        }
    }

    for (label, mut text) in &mut values {
        let Some(node) = document.node(label.id) else {
            continue;
        };

        let next = if editing.target == Some((label.id, EditField::Value)) {
            editing_text_with_cursor(&editing.buffer)
        } else {
            node.editable_value_text().unwrap_or_default()
        };

        if *text != Text::new(next.clone()) {
            *text = Text::new(next);
        }
    }
}

fn rebuild_canvas_overlay(
    mut commands: Commands,
    document: Res<GraphDocument>,
    session: Res<EditorSession>,
    time: Res<Time>,
    mut registry: ResMut<GraphUiRegistry>,
    canvas_query: Query<
        (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
        With<CanvasSurface>,
    >,
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
        spawn_wire_curve(
            &mut commands,
            overlay_layer,
            &mut registry.overlay_entities,
            start,
            end,
            edge.from.direction,
            edge.to.direction,
            color,
            WireStyle {
                thickness: (WIRE_THICKNESS * session.zoom).clamp(1.5, 6.0),
                alpha: 0.76,
                end_brighten: 0.14,
                pulse: 0.0,
                arrowhead: false,
            },
        );
    }

    if let Some(dragging) = session.dragging_wire {
        if let Some(pointer) = current_canvas_cursor_local_position(&windows, &canvas_query) {
            if let Some(start) = port_center(&document, &session, dragging.from) {
                let pulse = 0.5 + 0.5 * (time.elapsed_secs() * 8.0).sin();
                let hovered_target = session
                    .hovered_port
                    .filter(|candidate| port_can_connect(&document, dragging.from, *candidate));
                spawn_wire_curve(
                    &mut commands,
                    overlay_layer,
                    &mut registry.overlay_entities,
                    start,
                    pointer,
                    dragging.from.direction,
                    opposite_port_direction(dragging.from.direction),
                    port_color(dragging.ty),
                    WireStyle {
                        thickness: (WIRE_THICKNESS * session.zoom).clamp(1.8, 6.8)
                            * (1.08 + pulse * 0.1),
                        alpha: if hovered_target.is_some() {
                            0.98
                        } else {
                            0.88 + pulse * 0.08
                        },
                        end_brighten: if hovered_target.is_some() { 0.38 } else { 0.28 },
                        pulse,
                        arrowhead: true,
                    },
                );
            }
        }
    }
}

fn current_canvas_cursor_local_position(
    windows: &Query<&Window, With<PrimaryWindow>>,
    canvas_query: &Query<
        (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
        With<CanvasSurface>,
    >,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    canvas_local_from_window_position(canvas_query, cursor)
}

fn canvas_local_from_window_position(
    canvas_query: &Query<
        (&ComputedNode, &UiGlobalTransform, &RelativeCursorPosition),
        With<CanvasSurface>,
    >,
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

#[derive(Clone, Copy, Debug)]
struct WireStyle {
    thickness: f32,
    alpha: f32,
    end_brighten: f32,
    pulse: f32,
    arrowhead: bool,
}

fn spawn_wire_curve(
    commands: &mut Commands,
    overlay_layer: Entity,
    overlay_entities: &mut Vec<Entity>,
    start: Vec2,
    end: Vec2,
    start_direction: PortDirection,
    end_direction: PortDirection,
    color: Color,
    style: WireStyle,
) {
    let control_points =
        wire_control_points(start, end, start_direction, end_direction, style.thickness);
    let segments = wire_segment_count(start, end);
    let mut previous = cubic_bezier_point(control_points, 0.0);

    spawn_wire_cap(
        commands,
        overlay_layer,
        overlay_entities,
        previous,
        style.thickness * 0.92,
        color.with_alpha(style.alpha * 0.92),
    );

    for step in 1..=segments {
        let t = step as f32 / segments as f32;
        let current = cubic_bezier_point(control_points, t);
        let mid_t = (t + (step - 1) as f32 / segments as f32) * 0.5;
        let thickness = style.thickness * (0.92 + mid_t * 0.16 + style.pulse * 0.05);
        let segment_color = color
            .lighter(style.end_brighten * mid_t.powf(1.35))
            .with_alpha((style.alpha * (0.88 + mid_t * 0.12)).clamp(0.0, 1.0));
        spawn_wire_segment(
            commands,
            overlay_layer,
            overlay_entities,
            previous,
            current,
            segment_color,
            thickness,
        );
        spawn_wire_cap(
            commands,
            overlay_layer,
            overlay_entities,
            current,
            thickness,
            segment_color,
        );
        previous = current;
    }

    if style.arrowhead {
        let mut tangent = cubic_bezier_tangent(control_points, 0.985).normalize_or_zero();
        if tangent == Vec2::ZERO {
            tangent = (end - start).normalize_or_zero();
        }
        if tangent != Vec2::ZERO {
            let normal = Vec2::new(-tangent.y, tangent.x);
            let arrow_len = (style.thickness * 3.2).clamp(10.0, 16.0);
            let arrow_width = arrow_len * 0.4;
            let base = end - tangent * arrow_len;
            let left = base + normal * arrow_width;
            let right = base - normal * arrow_width;
            let arrow_color = color
                .lighter(style.end_brighten + 0.18)
                .with_alpha((style.alpha + 0.04).clamp(0.0, 1.0));
            spawn_wire_segment(
                commands,
                overlay_layer,
                overlay_entities,
                left,
                end,
                arrow_color,
                (style.thickness * 0.58).max(1.4),
            );
            spawn_wire_segment(
                commands,
                overlay_layer,
                overlay_entities,
                right,
                end,
                arrow_color,
                (style.thickness * 0.58).max(1.4),
            );
            spawn_wire_cap(
                commands,
                overlay_layer,
                overlay_entities,
                end,
                (style.thickness * 0.64).max(1.4),
                arrow_color,
            );
        }
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

fn spawn_wire_cap(
    commands: &mut Commands,
    overlay_layer: Entity,
    overlay_entities: &mut Vec<Entity>,
    center: Vec2,
    diameter: f32,
    color: Color,
) {
    let entity = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(center.x - diameter * 0.5),
                top: Val::Px(center.y - diameter * 0.5),
                width: Val::Px(diameter),
                height: Val::Px(diameter),
                border_radius: BorderRadius::all(Val::Px(diameter * 0.5)),
                ..default()
            },
            BackgroundColor(color),
            GlobalZIndex(3),
            Pickable::IGNORE,
        ))
        .id();
    commands.entity(overlay_layer).add_child(entity);
    overlay_entities.push(entity);
}

fn wire_control_points(
    start: Vec2,
    end: Vec2,
    start_direction: PortDirection,
    end_direction: PortDirection,
    thickness: f32,
) -> [Vec2; 4] {
    let distance = start.distance(end);
    let handle = (distance * 0.34).max(32.0 + thickness * 3.0).min(180.0);
    let start_handle = match start_direction {
        PortDirection::Input => start + Vec2::new(-handle, 0.0),
        PortDirection::Output => start + Vec2::new(handle, 0.0),
    };
    let end_handle = match end_direction {
        PortDirection::Input => end + Vec2::new(-handle, 0.0),
        PortDirection::Output => end + Vec2::new(handle, 0.0),
    };
    [start, start_handle, end_handle, end]
}

fn wire_segment_count(start: Vec2, end: Vec2) -> usize {
    ((start.distance(end) / WIRE_SEGMENT_LENGTH).ceil() as usize)
        .clamp(WIRE_MIN_SEGMENTS, WIRE_MAX_SEGMENTS)
}

fn cubic_bezier_point(points: [Vec2; 4], t: f32) -> Vec2 {
    let omt = 1.0 - t;
    points[0] * omt * omt * omt
        + points[1] * 3.0 * omt * omt * t
        + points[2] * 3.0 * omt * t * t
        + points[3] * t * t * t
}

fn cubic_bezier_tangent(points: [Vec2; 4], t: f32) -> Vec2 {
    let omt = 1.0 - t;
    (points[1] - points[0]) * 3.0 * omt * omt
        + (points[2] - points[1]) * 6.0 * omt * t
        + (points[3] - points[2]) * 3.0 * t * t
}

fn opposite_port_direction(direction: PortDirection) -> PortDirection {
    match direction {
        PortDirection::Input => PortDirection::Output,
        PortDirection::Output => PortDirection::Input,
    }
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
        overflow: Overflow::clip(),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(scaled(12.0, zoom))),
        ..default()
    }
}

fn node_dimensions(node: &GraphNode) -> Vec2 {
    node.size
}

fn port_center(
    document: &GraphDocument,
    session: &EditorSession,
    address: PortAddress,
) -> Option<Vec2> {
    let node = document.node(address.node)?;
    let zoom = session.zoom.max(0.001);
    let visual_position = world_to_screen(node.position, session.pan, zoom);
    let node_width = node.size.x;
    let port_inset = NODE_PADDING + PORT_GROUP_PADDING_X + (PORT_DOT_SIZE * 0.5);
    let x = match address.direction {
        PortDirection::Input => visual_position.x + scaled(port_inset, zoom),
        PortDirection::Output => visual_position.x + scaled(node_width - port_inset, zoom),
    };

    let y = if matches!(node.node_type, NodeType::Agent) {
        let port_ty = document.port_spec(address)?.ty;
        let rows = agent_wire_rows(document, address.node, node);
        let mut cursor_y = visual_position.y + scaled(NODE_HEADER_HEIGHT + PORTS_PADDING_Y, zoom);
        let section_height = scaled(AGENT_SECTION_ROW_HEIGHT, zoom);
        let wire_height = scaled(AGENT_WIRE_ROW_HEIGHT, zoom);
        let row_gap = scaled(PORT_ROW_GAP, zoom);

        let mut found = None;
        for row in rows {
            match row {
                AgentWireRow::Section(_) => {
                    cursor_y += section_height + row_gap;
                }
                AgentWireRow::Input(spec, _) => {
                    if address.direction == PortDirection::Input && spec.ty == port_ty {
                        found = Some(cursor_y + wire_height * 0.5);
                        break;
                    }
                    cursor_y += wire_height + row_gap;
                }
                AgentWireRow::Output(spec, _) => {
                    if address.direction == PortDirection::Output && spec.ty == port_ty {
                        found = Some(cursor_y + wire_height * 0.5);
                        break;
                    }
                    cursor_y += wire_height + row_gap;
                }
            }
        }
        found?
    } else {
        let row = address.index as f32;
        visual_position.y
            + scaled(NODE_HEADER_HEIGHT + PORTS_PADDING_Y, zoom)
            + scaled(PORT_ROW_HEIGHT * 0.5, zoom)
            + row * scaled(PORT_ROW_HEIGHT + PORT_ROW_GAP, zoom)
    };

    Some(Vec2::new(x, y))
}

fn editable_value_text(
    node: &GraphNode,
    is_value_editing: bool,
    editing: &TextEditingState,
) -> Option<String> {
    if is_value_editing {
        return Some(editing_text_with_cursor(&editing.buffer));
    }

    node.editable_value_text()
}

fn spawn_editable_value_surface(
    body: &mut ChildSpawnerCommands,
    node_id: NodeId,
    node: &GraphNode,
    is_value_editing: bool,
    editing: &TextEditingState,
    fonts: &EditorFont,
    zoom: f32,
) {
    let text = editable_value_text(node, is_value_editing, editing).unwrap_or_default();
    body.spawn((
        Node {
            width: Val::Percent(100.0),
            flex_grow: 1.0,
            min_height: Val::Px(0.0),
            padding: UiRect::all(Val::Px(scaled(10.0, zoom))),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::FlexStart,
            overflow: Overflow::clip(),
            border_radius: BorderRadius::all(Val::Px(scaled(8.0, zoom))),
            ..default()
        },
        BackgroundColor(Color::srgb_u8(9, 10, 12)),
        Pickable::IGNORE,
    ))
    .with_children(|surface| {
        surface.spawn((
            Node {
                width: Val::Percent(100.0),
                ..default()
            },
            Text::new(text),
            NodeValueText { id: node_id },
            TextLayout::new_with_linebreak(LineBreak::WordOrCharacter),
            editor_text_font(fonts, scaled_font(13.0, zoom)),
            TextColor(Color::srgb_u8(244, 246, 248)),
            Pickable::IGNORE,
        ));
    });
}

fn spawn_numeric_value_surface(
    body: &mut ChildSpawnerCommands,
    node_id: NodeId,
    node: &GraphNode,
    is_value_editing: bool,
    editing: &TextEditingState,
    fonts: &EditorFont,
    zoom: f32,
) {
    let display = if is_value_editing {
        editing_text_with_cursor(&editing.buffer)
    } else {
        node.editable_value_text().unwrap_or_default()
    };

    body.spawn((
        Node {
            width: Val::Percent(100.0),
            padding: UiRect::all(Val::Px(scaled(8.0, zoom))),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(scaled(10.0, zoom)),
            border_radius: BorderRadius::all(Val::Px(scaled(8.0, zoom))),
            ..default()
        },
        BackgroundColor(Color::srgb_u8(9, 10, 12)),
        Pickable::IGNORE,
    ))
    .with_children(|surface| {
        spawn_compact_node_action_button(
            surface,
            "‹",
            NodeAction::PreviousSetting(node_id),
            fonts,
            zoom,
        );
        surface
            .spawn((
                Node {
                    flex_grow: 1.0,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_child((
                Text::new(display),
                NodeValueText { id: node_id },
                editor_text_font(fonts, scaled_font(13.0, zoom)),
                TextColor(Color::srgb_u8(244, 246, 248)),
                Pickable::IGNORE,
            ));
        spawn_compact_node_action_button(
            surface,
            "›",
            NodeAction::NextSetting(node_id),
            fonts,
            zoom,
        );
    });
}

fn spawn_resize_handle(
    parent: &mut ChildSpawnerCommands,
    node_id: NodeId,
    corner: ResizeCorner,
    zoom: f32,
) {
    let handle_size = scaled(RESIZE_HANDLE_HIT_SIZE, zoom);
    let mut style = Node {
        position_type: PositionType::Absolute,
        width: Val::Px(handle_size),
        height: Val::Px(handle_size),
        ..default()
    };

    match corner {
        ResizeCorner::NorthWest => {
            style.left = Val::Px(scaled(1.0, zoom));
            style.top = Val::Px(scaled(1.0, zoom));
        }
        ResizeCorner::NorthEast => {
            style.right = Val::Px(scaled(1.0, zoom));
            style.top = Val::Px(scaled(1.0, zoom));
        }
        ResizeCorner::SouthWest => {
            style.left = Val::Px(scaled(1.0, zoom));
            style.bottom = Val::Px(scaled(1.0, zoom));
        }
        ResizeCorner::SouthEast => {
            style.right = Val::Px(scaled(1.0, zoom));
            style.bottom = Val::Px(scaled(1.0, zoom));
        }
    }

    parent
        .spawn((
            Button,
            style,
            BackgroundColor(Color::NONE),
            BorderColor::all(Color::NONE),
            ButtonHintText("Drag to resize this node"),
        ))
        .observe(|mut click: On<Pointer<Click>>| {
            click.propagate(false);
        })
        .observe(
            move |mut drag: On<Pointer<Drag>>,
                  mut document: ResMut<GraphDocument>,
                  session: Res<EditorSession>| {
                if drag.button == PointerButton::Primary {
                    drag.propagate(false);
                    let zoom = session.zoom.max(0.001);
                    document.resize_node(node_id, corner, drag.delta / zoom);
                }
            },
        );
}

enum AgentWireRow {
    Section(&'static str),
    Input(PortSpec, String),
    Output(PortSpec, String),
}

fn agent_wire_rows(
    document: &GraphDocument,
    node_id: NodeId,
    node: &GraphNode,
) -> Vec<AgentWireRow> {
    let input = |ty: PortType| node.inputs().iter().find(|spec| spec.ty == ty).copied();
    let output = |ty: PortType| node.outputs().iter().find(|spec| spec.ty == ty).copied();
    let mut rows = Vec::new();

    rows.push(AgentWireRow::Section("Core"));
    rows.push(AgentWireRow::Input(
        input(PortType::Model).expect("agent model port"),
        inline_port_status(document, node_id, PortType::Model, true),
    ));
    rows.push(AgentWireRow::Input(
        input(PortType::Prompt).expect("agent prompt port"),
        inline_port_status(document, node_id, PortType::Prompt, true),
    ));
    rows.push(AgentWireRow::Output(
        output(PortType::TextResponse).expect("agent text output port"),
        output_port_status(document, node_id, PortType::TextResponse),
    ));

    rows.push(AgentWireRow::Section("Identity"));
    for ty in [
        PortType::AgentName,
        PortType::AgentDescription,
        PortType::Preamble,
    ] {
        rows.push(AgentWireRow::Input(
            input(ty).expect("agent identity port"),
            inline_port_status(document, node_id, ty, false),
        ));
    }

    rows.push(AgentWireRow::Section("Context"));
    for ty in [PortType::StaticContext, PortType::DynamicContext] {
        rows.push(AgentWireRow::Input(
            input(ty).expect("agent context port"),
            inline_multi_port_status(document, node_id, ty),
        ));
    }

    rows.push(AgentWireRow::Section("Sampling"));
    for ty in [
        PortType::Temperature,
        PortType::MaxTokens,
        PortType::DefaultMaxTurns,
    ] {
        rows.push(AgentWireRow::Input(
            input(ty).expect("agent sampling port"),
            inline_port_status(document, node_id, ty, false),
        ));
    }

    rows.push(AgentWireRow::Section("Advanced"));
    for ty in [
        PortType::AdditionalParams,
        PortType::ToolServerHandle,
        PortType::ToolChoice,
        PortType::Hook,
        PortType::OutputSchema,
    ] {
        rows.push(AgentWireRow::Input(
            input(ty).expect("agent advanced port"),
            inline_port_status(document, node_id, ty, false),
        ));
    }

    rows
}

fn spawn_agent_wire_row(
    ports: &mut ChildSpawnerCommands,
    row: &AgentWireRow,
    node_id: NodeId,
    fonts: &EditorFont,
    zoom: f32,
) {
    match row {
        AgentWireRow::Section(title) => {
            ports
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(scaled(AGENT_SECTION_ROW_HEIGHT, zoom)),
                        padding: UiRect::axes(
                            Val::Px(scaled(6.0, zoom)),
                            Val::Px(scaled(6.0, zoom)),
                        ),
                        border: UiRect::top(Val::Px(1.0)),
                        ..default()
                    },
                    BorderColor {
                        top: Color::srgb_u8(77, 79, 86),
                        ..BorderColor::DEFAULT
                    },
                    Pickable::IGNORE,
                ))
                .with_child((
                    Text::new(*title),
                    editor_text_font(fonts, scaled_font(11.0, zoom)),
                    TextColor(Color::srgb_u8(162, 168, 178)),
                    Pickable::IGNORE,
                ));
        }
        AgentWireRow::Input(spec, status) => {
            let index = spec_index(node_inputs(NodeType::Agent), spec.ty);
            ports
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(scaled(AGENT_WIRE_ROW_HEIGHT, zoom)),
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(scaled(8.0, zoom)),
                        padding: UiRect::axes(
                            Val::Px(scaled(6.0, zoom)),
                            Val::Px(scaled(2.0, zoom)),
                        ),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|row_parent| {
                    let address = PortAddress {
                        node: node_id,
                        direction: PortDirection::Input,
                        index,
                    };
                    spawn_port_button(row_parent, address, spec.ty, zoom);
                    row_parent.spawn((
                        Text::new(format!("{} {}", spec.name, status)),
                        editor_text_font(fonts, scaled_font(12.0, zoom)),
                        TextColor(Color::srgb_u8(236, 238, 241)),
                        Pickable::IGNORE,
                    ));
                });
        }
        AgentWireRow::Output(spec, status) => {
            let index = spec_index(node_outputs(NodeType::Agent), spec.ty);
            ports
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(scaled(AGENT_WIRE_ROW_HEIGHT, zoom)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::FlexEnd,
                        column_gap: Val::Px(scaled(8.0, zoom)),
                        padding: UiRect::axes(
                            Val::Px(scaled(6.0, zoom)),
                            Val::Px(scaled(2.0, zoom)),
                        ),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|row_parent| {
                    row_parent.spawn((
                        Text::new(format!("{} {}", spec.name, status)),
                        editor_text_font(fonts, scaled_font(12.0, zoom)),
                        TextColor(Color::srgb_u8(236, 238, 241)),
                        Pickable::IGNORE,
                    ));
                    let address = PortAddress {
                        node: node_id,
                        direction: PortDirection::Output,
                        index,
                    };
                    spawn_port_button(row_parent, address, spec.ty, zoom);
                });
        }
    }
}

fn spec_index(specs: &[PortSpec], ty: PortType) -> usize {
    specs
        .iter()
        .position(|spec| spec.ty == ty)
        .expect("port type missing from spec list")
}

fn inline_port_status(
    document: &GraphDocument,
    node_id: NodeId,
    ty: PortType,
    required: bool,
) -> String {
    let count = document.input_sources(node_id, ty).len();
    format!(
        "({}{} connected)",
        if required { "required, " } else { "optional, " },
        count
    )
}

fn inline_multi_port_status(document: &GraphDocument, node_id: NodeId, ty: PortType) -> String {
    let count = document.input_sources(node_id, ty).len();
    format!("(optional, {} connected)", count)
}

fn output_port_status(document: &GraphDocument, node_id: NodeId, ty: PortType) -> String {
    let count = document.output_targets(node_id, ty).len();
    format!("(output, {} connected)", count)
}

fn node_body_lines(
    node: &GraphNode,
    is_value_editing: bool,
    editing: &TextEditingState,
) -> Vec<String> {
    if is_value_editing {
        return preview_multiline(&editing_text_with_cursor(&editing.buffer), 10);
    }

    if let Some(value) = node.editable_value_text() {
        return preview_multiline(&value, 10);
    }

    node.summary_lines()
}

fn editing_text_with_cursor(value: &str) -> String {
    format!("{value}|")
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
        PortType::Temperature
        | PortType::MaxTokens
        | PortType::DefaultMaxTurns
        | PortType::U64Value => Color::srgb_u8(255, 173, 96),
        PortType::AdditionalParams | PortType::OutputSchema => Color::srgb_u8(255, 126, 133),
        PortType::ToolChoice
        | PortType::ToolServerHandle
        | PortType::DynamicContext
        | PortType::Hook => Color::srgb_u8(141, 206, 255),
        PortType::AgentName | PortType::AgentDescription => Color::srgb_u8(196, 214, 112),
    }
}
