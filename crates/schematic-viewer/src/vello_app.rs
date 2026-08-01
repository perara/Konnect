//! Native, dependency-gated Vello schematic viewer.

use crate::change_timeline::{ChangeKind, ChangeOrigin, ChangeTimeline};
use crate::edit_session::EditSession;
use crate::editor_history::{HistoryCommand, HistoryCreation, HistoryEntry};
use crate::editor_model::{
    box_selects_bounds, drag_delta_mm, kicad_lock_path, notification_matches_revision, snap_point,
    summarize_external_change,
};
use crate::native_scene::{
    connected_wire_moves, discover_hierarchy, duplicate_symbol_block, load_hierarchy,
    move_items_with_connected_wires, rotate_item_source, Bounds, ColorRole, ConnectedWireMove,
    ConnectivityDiagnosticKind, HierarchyEntry, HierarchyScene, ObjectKind, Point as SchPoint,
    Primitive, SchematicScene, TextAlign,
};
use crate::vello_render::{
    encode_primitives, encode_scene, encode_scene_without_ranges, polyline_path, round_stroke,
};
#[cfg(test)]
use crate::vello_render::{kicad_svg_arc, svg_arc_path};
use crate::viewer_settings::ViewerSettings;
use anyhow::{anyhow, Context, Result};
use fontdb::{Database, Family, Query};
use konnect_sexp::schematic::{BusEntryDirection, HierarchicalSheetSpec, SheetPinType};
use konnect_sexp::{
    commit_file_transaction, parse_sexp, prepare_command, read_consistent,
    recover_file_transactions, DocumentRevision, ItemAnchor, ItemId, SchematicCommand, SexpError,
};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rustybuzz::{shape, Face as ShapingFace, UnicodeBuffer};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};
use vello::kurbo::{
    Affine, Arc as KurboArc, BezPath, Circle, Line, Point as KurboPoint, Rect, RoundedRect, Stroke,
};
use vello::peniko::{Blob, Color, Fill, FontData};
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu;
use vello::{AaConfig, Glyph, Renderer, RendererOptions, Scene};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Icon, Window, WindowAttributes, WindowId};

#[path = "vello_frame.rs"]
mod vello_frame;
#[path = "vello_interaction.rs"]
mod vello_interaction;
#[path = "vello_panels.rs"]
mod vello_panels;
#[path = "vello_runtime.rs"]
mod vello_runtime;
#[path = "vello_ui.rs"]
mod vello_ui;

use vello_frame::*;
use vello_runtime::*;
pub(crate) use vello_ui::Palette;
use vello_ui::{
    change_icon, draw_selectable_text, draw_ui_icon, NativeFont, ScreenRect, SelectableText,
    TextDrag, TextRun, TextSelection, Theme, DIAGNOSTICS_HEADER_HEIGHT, DIAGNOSTICS_PANEL_WIDTH,
    FILMSTRIP_HEIGHT, HISTORY_LIMIT, PAGE_PADDING, SIDEBAR_WIDTH, STATUS_HEIGHT, THUMBNAIL_GAP,
    THUMBNAIL_WIDTH, TIMELINE_HEIGHT,
};

struct NativeSheet {
    name: String,
    depth: usize,
    file: PathBuf,
    semantic: SchematicScene,
    rendered: Scene,
    compatibility: Option<Scene>,
    compatibility_error: Option<String>,
}

impl NativeSheet {
    fn from_hierarchy(entry: HierarchyScene, palette: Palette) -> Self {
        let rendered = encode_scene(&entry.scene, palette);
        let compatibility = (!entry.scene.coverage.is_complete())
            .then(|| crate::svg_order_cache::load_fresh(&entry.file))
            .flatten()
            .and_then(|svg| compatibility_scene(&svg).ok());
        Self {
            name: entry.name,
            depth: entry.depth,
            file: entry.file,
            semantic: entry.scene,
            rendered,
            compatibility,
            compatibility_error: None,
        }
    }

    fn rebuild(&mut self, palette: Palette) {
        self.rendered = encode_scene(&self.semantic, palette);
    }
}

struct RenderState {
    surface: RenderSurface<'static>,
    window: Arc<Window>,
    valid_surface: bool,
}

enum UserEvent {
    FilesChanged(Vec<PathBuf>),
    Reloaded(ReloadBatch),
}

#[derive(Debug)]
struct ReloadRequest {
    generation: u64,
    root: PathBuf,
    changed: HashSet<PathBuf>,
    known: HashSet<PathBuf>,
    external: bool,
}

struct ReloadBatch {
    generation: u64,
    entries: std::result::Result<Vec<HierarchyEntry>, String>,
    loaded: HashMap<PathBuf, std::result::Result<LoadedScene, String>>,
    external: bool,
}

struct LoadedScene {
    semantic: SchematicScene,
    compatibility: Option<Scene>,
    compatibility_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarAction {
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    Fit,
    Grid,
    Snap,
    UiScale,
    HighlightChanges,
    FollowChanges,
    TextSelect,
    Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditControl {
    EditMode,
    Commit,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiIcon {
    Add,
    Commit,
    Delete,
    Discard,
    Duplicate,
    Edit,
    External,
    Fit,
    Follow,
    Grid,
    Highlight,
    Move,
    Redo,
    Scale,
    Snap,
    Theme,
    TextSelect,
    Transform,
    Undo,
    Wire,
    ZoomIn,
    ZoomOut,
}

#[derive(Debug, Clone, Copy)]
struct SelectionBox {
    start: PhysicalPosition<f64>,
    current: PhysicalPosition<f64>,
    additive: bool,
}

struct ItemDrag {
    start: PhysicalPosition<f64>,
    current: PhysicalPosition<f64>,
    base_scene: Scene,
    connected_wires: Vec<ConnectedWireMove>,
}

#[derive(Debug, Clone, Copy)]
struct DiagnosticsPanelDrag {
    pointer_offset: (f64, f64),
}

#[derive(Debug, Clone)]
struct SearchHit {
    sheet: usize,
    uuid: Option<String>,
    description: String,
}

#[derive(Debug, Clone, Default)]
struct SearchState {
    query: String,
    hits: Vec<SearchHit>,
    current: usize,
}

#[derive(Debug, Clone)]
struct PropertyEdit {
    file: PathBuf,
    uuid: String,
    name: String,
    value: String,
}

#[derive(Debug, Clone, Copy)]
struct WireDraft {
    start: SchPoint,
    current: SchPoint,
    is_bus: bool,
}

#[derive(Debug, Clone)]
struct LabelEdit {
    point: SchPoint,
    value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SheetEditField {
    Name,
    File,
}

#[derive(Debug, Clone)]
struct SheetEdit {
    point: SchPoint,
    name: String,
    file: String,
    field: SheetEditField,
}

#[derive(Debug, Clone)]
struct SheetPinEdit {
    sheet_uuid: String,
    point: SchPoint,
    rotation: f64,
    name: String,
    pin_type: SheetPinType,
}

#[derive(Debug, Clone)]
struct ExternalChangePreview {
    lines: Vec<String>,
}

struct VelloViewer {
    root: PathBuf,
    font: NativeFont,
    theme: Theme,
    sheets: Vec<NativeSheet>,
    active: usize,
    selected_uuids: HashSet<String>,
    status: String,
    settings: ViewerSettings,
    timeline: ChangeTimeline,
    highlighted_change: Option<u64>,
    pending_follow: Option<u64>,
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    local_revisions: HashMap<PathBuf, DocumentRevision>,
    edit_session: EditSession,

    context: RenderContext,
    renderers: Vec<Option<Renderer>>,
    state: Option<RenderState>,
    cached_window: Option<Arc<Window>>,
    frame: Scene,

    watcher: RecommendedWatcher,
    watched_dirs: HashSet<PathBuf>,
    reload_tx: mpsc::Sender<ReloadRequest>,
    reload_generation: Arc<AtomicU64>,

    cursor: Option<PhysicalPosition<f64>>,
    modifiers: ModifiersState,
    panning: bool,
    selection_box: Option<SelectionBox>,
    item_drag: Option<ItemDrag>,
    diagnostics_drag: Option<DiagnosticsPanelDrag>,
    search: Option<SearchState>,
    property_edit: Option<PropertyEdit>,
    wire_draft: Option<WireDraft>,
    label_edit: Option<LabelEdit>,
    sheet_edit: Option<SheetEdit>,
    sheet_pin_edit: Option<SheetPinEdit>,
    external_preview: Option<ExternalChangePreview>,
    text_targets: Vec<SelectableText>,
    text_select_mode: bool,
    text_drag: Option<TextDrag>,
    text_selection: Option<TextSelection>,
    clipboard: Option<arboard::Clipboard>,
    pan: (f64, f64),
    zoom: f64,
    grid_mm: f64,
    bus_entry_direction: BusEntryDirection,
    snap_enabled: bool,
    film_scroll: f64,
    timeline_scroll: usize,
}

impl VelloViewer {
    fn new(
        root: PathBuf,
        mut font: NativeFont,
        sheets: Vec<NativeSheet>,
        watcher: RecommendedWatcher,
        reload_tx: mpsc::Sender<ReloadRequest>,
        reload_generation: Arc<AtomicU64>,
    ) -> Self {
        let settings = ViewerSettings::load();
        font.ui_scale = settings.ui_scale;
        let theme = if settings.dark_theme {
            Theme::Dark
        } else {
            Theme::Light
        };
        let mut viewer = Self {
            root,
            font,
            theme,
            sheets,
            active: 0,
            selected_uuids: HashSet::new(),
            status: "Live".to_owned(),
            settings,
            timeline: ChangeTimeline::default(),
            highlighted_change: None,
            pending_follow: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            local_revisions: HashMap::new(),
            edit_session: EditSession::default(),
            context: RenderContext::new(),
            renderers: Vec::new(),
            state: None,
            cached_window: None,
            frame: Scene::new(),
            watcher,
            watched_dirs: HashSet::new(),
            reload_tx,
            reload_generation,
            cursor: None,
            modifiers: ModifiersState::empty(),
            panning: false,
            selection_box: None,
            item_drag: None,
            diagnostics_drag: None,
            search: None,
            property_edit: None,
            wire_draft: None,
            label_edit: None,
            sheet_edit: None,
            sheet_pin_edit: None,
            external_preview: None,
            text_targets: Vec::new(),
            text_select_mode: false,
            text_drag: None,
            text_selection: None,
            clipboard: arboard::Clipboard::new().ok(),
            pan: (0.0, 0.0),
            zoom: 1.0,
            grid_mm: 1.27,
            bus_entry_direction: BusEntryDirection::DownRight,
            snap_enabled: true,
            film_scroll: 0.0,
            timeline_scroll: 0,
        };
        viewer.reconcile_watch_dirs();
        viewer.status = viewer.live_status();
        viewer
    }
}

fn can_exit_without_losing_staged_edits(edit_session: &EditSession) -> bool {
    !edit_session.has_pending()
}

pub(crate) fn run() -> Result<()> {
    run_native()
}

impl ApplicationHandler<UserEvent> for VelloViewer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = match self.cached_window.take() {
            Some(window) => window,
            None => match event_loop.create_window(window_attributes()) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    eprintln!("failed to create Vello window: {error}");
                    event_loop.exit();
                    return;
                }
            },
        };
        let size = window.inner_size();
        let surface = match pollster::block_on(self.context.create_surface(
            window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        )) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("failed to create Vello GPU surface: {error}");
                event_loop.exit();
                return;
            }
        };
        self.renderers
            .resize_with(self.context.devices.len(), || None);
        let device_id = surface.dev_id;
        if self.renderers[device_id].is_none() {
            let options = RendererOptions {
                use_cpu: false,
                antialiasing_support: [AaConfig::Area].into_iter().collect(),
                num_init_threads: None,
                pipeline_cache: None,
            };
            match Renderer::new(&self.context.devices[device_id].device, options) {
                Ok(renderer) => self.renderers[device_id] = Some(renderer),
                Err(error) => {
                    eprintln!("failed to initialize Vello renderer: {error}");
                    event_loop.exit();
                    return;
                }
            }
        }
        window.set_title("Konnect — Schematic Studio");
        window.request_redraw();
        self.state = Some(RenderState {
            surface,
            window,
            valid_surface: size.width > 0 && size.height > 0,
        });
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.take() {
            self.cached_window = Some(state.window);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &self.state else {
            return;
        };
        if state.window.id() != window_id {
            return;
        }
        let width = f64::from(state.surface.config.width);
        let height = f64::from(state.surface.config.height);

        match event {
            WindowEvent::CloseRequested => {
                if can_exit_without_losing_staged_edits(&self.edit_session) {
                    event_loop.exit();
                } else {
                    self.status = "Close blocked · Commit or Discard staged changes before exiting"
                        .to_owned();
                    self.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.valid_surface = size.width > 0 && size.height > 0;
                    if state.valid_surface {
                        self.context
                            .resize_surface(&mut state.surface, size.width, size.height);
                        state.window.request_redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(drag) = self.diagnostics_drag {
                    self.move_diagnostics_panel(width, height, position, drag);
                    self.request_redraw();
                } else if self.text_drag.is_some() {
                    self.update_text_selection(position);
                    self.request_redraw();
                } else if self.wire_draft.is_some() {
                    self.update_wire_draft(width, height, position);
                    self.request_redraw();
                } else if let Some(drag) = &mut self.item_drag {
                    drag.current = position;
                    self.status = "Dragging · release to stage this edit".to_owned();
                    self.request_redraw();
                } else if let Some(selection) = &mut self.selection_box {
                    selection.current = position;
                    self.request_redraw();
                } else if self.panning {
                    if let Some(previous) = self.cursor {
                        self.pan.0 += position.x - previous.x;
                        self.pan.1 += position.y - previous.y;
                    }
                    self.request_redraw();
                }
                self.cursor = Some(position);
                self.request_redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.finish_diagnostics_drag();
                self.cursor = None;
                self.panning = false;
                self.selection_box = None;
                self.item_drag = None;
                self.text_drag = None;
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Released {
                    if button == MouseButton::Left {
                        if self.diagnostics_drag.is_some() {
                            self.finish_diagnostics_drag();
                            self.request_redraw();
                        } else if self.text_drag.is_some() {
                            self.finish_text_selection();
                            self.request_redraw();
                        } else if let Some(drag) = self.item_drag.take() {
                            self.finish_item_drag(width, height, drag);
                            self.request_redraw();
                        } else if let Some(selection) = self.selection_box.take() {
                            self.finish_box_selection(width, height, selection);
                            self.request_redraw();
                        }
                    }
                    self.panning = false;
                    return;
                }
                let Some(cursor) = self.cursor else {
                    return;
                };
                if button == MouseButton::Left {
                    if self.handle_edit_controls(width, cursor.x, cursor.y)
                        || self.handle_toolbar(width, height, cursor.x, cursor.y)
                        || self.handle_diagnostics(width, height, cursor.x, cursor.y)
                    {
                        self.request_redraw();
                    } else if self.text_select_mode {
                        self.start_text_selection(cursor);
                        self.request_redraw();
                    } else if self.wire_draft.is_some() {
                        self.update_wire_draft(width, height, cursor);
                        self.commit_wire();
                        self.request_redraw();
                    } else if self.handle_filmstrip(width, height, cursor.x, cursor.y) {
                        self.request_redraw();
                    } else if self.main_rect(width, height).contains(cursor.x, cursor.y) {
                        let additive = self.modifiers.shift_key()
                            || self.modifiers.control_key()
                            || self.modifiers.super_key();
                        if self.select_at(width, height, cursor.x, cursor.y, additive) {
                            if self.edit_session.enabled
                                && !additive
                                && self.selected_items_are_movable()
                            {
                                self.start_item_drag(cursor);
                            }
                        } else {
                            if self.modifiers.alt_key() {
                                self.panning = true;
                            } else {
                                self.selection_box = Some(SelectionBox {
                                    start: cursor,
                                    current: cursor,
                                    additive,
                                });
                            }
                        }
                        self.request_redraw();
                    }
                } else if matches!(button, MouseButton::Middle | MouseButton::Right) {
                    self.panning = true;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(cursor) = self.cursor else {
                    return;
                };
                let amount = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        if self
                            .filmstrip_rect(width, height)
                            .contains(cursor.x, cursor.y)
                        {
                            f64::from(if x.abs() > y.abs() { x } else { y }) * 40.0
                        } else {
                            f64::from(y) * 40.0
                        }
                    }
                    MouseScrollDelta::PixelDelta(position) => {
                        if position.x.abs() > position.y.abs() {
                            position.x
                        } else {
                            position.y
                        }
                    }
                };
                if cursor.y >= height - TIMELINE_HEIGHT
                    && self
                        .filmstrip_rect(width, height)
                        .contains(cursor.x, cursor.y)
                {
                    const LABEL_WIDTH: f64 = 96.0;
                    const CARD_WIDTH: f64 = 138.0;
                    const CARD_GAP: f64 = 6.0;
                    let available = (width - SIDEBAR_WIDTH - LABEL_WIDTH - 20.0).max(CARD_WIDTH);
                    let visible = ((available + CARD_GAP) / (CARD_WIDTH + CARD_GAP))
                        .floor()
                        .max(1.0) as usize;
                    let maximum = self.timeline.len().saturating_sub(visible);
                    if amount > 0.0 {
                        self.timeline_scroll = (self.timeline_scroll + 1).min(maximum);
                    } else if amount < 0.0 {
                        self.timeline_scroll = self.timeline_scroll.saturating_sub(1);
                    }
                } else if self
                    .filmstrip_rect(width, height)
                    .contains(cursor.x, cursor.y)
                {
                    let content_width = self.sheets.len() as f64
                        * (THUMBNAIL_WIDTH + THUMBNAIL_GAP)
                        + THUMBNAIL_GAP;
                    self.film_scroll =
                        (self.film_scroll - amount).clamp(0.0, (content_width - width).max(0.0));
                } else {
                    self.set_zoom_about(
                        width,
                        height,
                        self.zoom * 1.0015_f64.powf(amount),
                        cursor.x,
                        cursor.y,
                    );
                }
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let editing = self.modifiers.control_key() || self.modifiers.super_key();
                if editing
                    && matches!(event.logical_key.as_ref(), Key::Character(value) if value.eq_ignore_ascii_case("c"))
                    && self.text_selection.is_some()
                {
                    self.copy_selected_text();
                    self.request_redraw();
                    return;
                }
                if self.handle_sheet_pin_key(event.logical_key.as_ref()) {
                    self.request_redraw();
                    return;
                }
                if self.handle_sheet_key(event.logical_key.as_ref()) {
                    self.request_redraw();
                    return;
                }
                if self.handle_label_key(event.logical_key.as_ref()) {
                    self.request_redraw();
                    return;
                }
                if self.handle_property_key(event.logical_key.as_ref()) {
                    self.request_redraw();
                    return;
                }
                if self.handle_search_key(event.logical_key.as_ref()) {
                    self.request_redraw();
                    return;
                }
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Enter) if editing => self.commit_edit_session(),
                    Key::Named(NamedKey::Escape) => {
                        if self.text_select_mode || self.text_selection.is_some() {
                            self.text_select_mode = false;
                            self.text_drag = None;
                            self.text_selection = None;
                            self.status = self.live_status();
                        } else if self.external_preview.take().is_some() {
                            self.status = self.live_status();
                        } else {
                            let cancelled_wire = self.wire_draft.take().is_some();
                            self.selection_box = None;
                            self.item_drag = None;
                            self.panning = false;
                            if cancelled_wire {
                                self.status = "Wire cancelled".to_owned();
                            } else {
                                self.selected_uuids.clear();
                                self.status = self.live_status();
                            }
                        }
                    }
                    Key::Named(NamedKey::Delete) => self.delete_selected(),
                    Key::Named(NamedKey::ArrowLeft) if editing => {
                        self.nudge_selected(-self.grid_mm, 0.0);
                    }
                    Key::Named(NamedKey::ArrowRight) if editing => {
                        self.nudge_selected(self.grid_mm, 0.0);
                    }
                    Key::Named(NamedKey::ArrowUp) if editing => {
                        self.nudge_selected(0.0, -self.grid_mm);
                    }
                    Key::Named(NamedKey::ArrowDown) if editing => {
                        self.nudge_selected(0.0, self.grid_mm);
                    }
                    Key::Character(value)
                        if editing
                            && value.eq_ignore_ascii_case("z")
                            && self.modifiers.shift_key() =>
                    {
                        self.redo();
                    }
                    Key::Character(value) if editing && (value.eq_ignore_ascii_case("y")) => {
                        self.redo();
                    }
                    Key::Character(value) if editing && value.eq_ignore_ascii_case("z") => {
                        self.undo();
                    }
                    Key::Character(value) if editing && value.eq_ignore_ascii_case("f") => {
                        self.start_search();
                    }
                    Key::Character(value) if editing && value.eq_ignore_ascii_case("d") => {
                        self.duplicate_selected();
                    }
                    Key::Named(NamedKey::ArrowLeft) => {
                        self.switch_sheet(self.active.saturating_sub(1));
                    }
                    Key::Named(NamedKey::ArrowRight) => {
                        self.switch_sheet(
                            (self.active + 1).min(self.sheets.len().saturating_sub(1)),
                        );
                    }
                    Key::Character("+" | "=") => {
                        let (x, y) = self.main_rect(width, height).center();
                        self.set_zoom_about(width, height, self.zoom * 1.25, x, y);
                    }
                    Key::Character("-") => {
                        let (x, y) = self.main_rect(width, height).center();
                        self.set_zoom_about(width, height, self.zoom / 1.25, x, y);
                    }
                    Key::Character("0") => self.fit(),
                    Key::Character(value) if value.eq_ignore_ascii_case("t") => {
                        self.toggle_theme();
                    }
                    Key::Character(value) if value.eq_ignore_ascii_case("g") => {
                        self.cycle_grid();
                    }
                    Key::Character(value) if value.eq_ignore_ascii_case("s") => {
                        self.toggle_snap();
                    }
                    Key::Character(value) if value.eq_ignore_ascii_case("e") => {
                        self.start_property_edit();
                    }
                    Key::Character(value) if !editing && value.eq_ignore_ascii_case("w") => {
                        self.start_wire(width, height, self.modifiers.shift_key());
                    }
                    Key::Character(value)
                        if !editing
                            && self.wire_draft.is_none()
                            && value.eq_ignore_ascii_case("j") =>
                    {
                        self.insert_at_cursor(width, height, "junction", |point| {
                            konnect_sexp::schematic::format_junction(point.x, point.y)
                        });
                    }
                    Key::Character(value)
                        if !editing
                            && self.wire_draft.is_none()
                            && value.eq_ignore_ascii_case("q") =>
                    {
                        self.insert_at_cursor(width, height, "no-connect marker", |point| {
                            konnect_sexp::schematic::format_no_connect(point.x, point.y)
                        });
                    }
                    Key::Character(value)
                        if !editing
                            && self.wire_draft.is_none()
                            && value.eq_ignore_ascii_case("l") =>
                    {
                        self.start_label_edit(width, height);
                    }
                    Key::Character(value)
                        if !editing
                            && self.wire_draft.is_none()
                            && value.eq_ignore_ascii_case("b") =>
                    {
                        if self.modifiers.shift_key() {
                            self.bus_entry_direction = self.bus_entry_direction.rotated_clockwise();
                            self.status =
                                format!("Bus-entry direction · {:?}", self.bus_entry_direction);
                        } else {
                            let direction = self.bus_entry_direction;
                            self.insert_at_cursor(width, height, "bus entry", |point| {
                                konnect_sexp::schematic::format_bus_entry(
                                    point.x, point.y, direction,
                                )
                            });
                        }
                    }
                    Key::Character(value)
                        if !editing
                            && self.wire_draft.is_none()
                            && value.eq_ignore_ascii_case("h") =>
                    {
                        self.start_sheet_edit(width, height);
                    }
                    Key::Character(value)
                        if !editing
                            && self.wire_draft.is_none()
                            && value.eq_ignore_ascii_case("p") =>
                    {
                        self.start_sheet_pin_edit(width, height);
                    }
                    Key::Character(value) if editing && value.eq_ignore_ascii_case("r") => {
                        if self.edit_session.has_pending() {
                            self.status = "Reload blocked · Commit or Discard staged changes first"
                                .to_owned();
                        } else {
                            let files = self
                                .sheets
                                .iter()
                                .map(|sheet| sheet.file.clone())
                                .collect::<Vec<_>>();
                            self.schedule_reload(&files);
                        }
                    }
                    Key::Character(value) if value.eq_ignore_ascii_case("r") => {
                        self.rotate_selected();
                    }
                    Key::Character(value) if value.eq_ignore_ascii_case("x") => {
                        self.mirror_selected("x");
                    }
                    Key::Character(value) if value.eq_ignore_ascii_case("y") => {
                        self.mirror_selected("y");
                    }
                    _ => {}
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let Some((width, height, device_id, valid_surface)) =
                    self.state.as_ref().map(|state| {
                        (
                            state.surface.config.width,
                            state.surface.config.height,
                            state.surface.dev_id,
                            state.valid_surface,
                        )
                    })
                else {
                    return;
                };
                if !valid_surface {
                    return;
                }
                self.draw_frame(width, height);
                let Some(state) = &self.state else {
                    return;
                };
                let params = vello::RenderParams {
                    base_color: self.palette().app,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                };
                let Some(renderer) = self.renderers[device_id].as_mut() else {
                    return;
                };
                let device = &self.context.devices[device_id];
                if let Err(error) = renderer.render_to_texture(
                    &device.device,
                    &device.queue,
                    &self.frame,
                    &state.surface.target_view,
                    &params,
                ) {
                    self.status = format!("GPU render error: {error}");
                    return;
                }
                let texture = match state.surface.surface.get_current_texture() {
                    Ok(texture) => texture,
                    Err(error) => {
                        self.status = format!("Surface error: {error}");
                        return;
                    }
                };
                let mut encoder =
                    device
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Konnect Vello blit"),
                        });
                state.surface.blitter.copy(
                    &device.device,
                    &mut encoder,
                    &state.surface.target_view,
                    &texture
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default()),
                );
                device.queue.submit([encoder.finish()]);
                texture.present();
                let _ = device.device.poll(wgpu::PollType::Poll);
                if self.timeline.is_animating(Instant::now()) {
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::FilesChanged(paths) => {
                let paths = self.external_paths(paths);
                if paths.is_empty() {
                    return;
                }
                let keys = paths.iter().map(|path| path_key(path)).collect::<Vec<_>>();
                let conflicts = self.edit_session.mark_external_change(keys.clone());
                let paths = paths
                    .into_iter()
                    .zip(keys)
                    .filter_map(|(path, key)| {
                        self.edit_session
                            .staged_source(&key)
                            .is_none()
                            .then_some(path)
                    })
                    .collect::<Vec<_>>();
                if conflicts > 0 {
                    self.status = format!(
                        "External change overlaps {conflicts} staged file(s) · Commit blocked; Discard to load it"
                    );
                }
                if conflicts == 0 {
                    self.status = "Change detected · rendering directly…".to_owned();
                }
                if !paths.is_empty() {
                    self.schedule_external_reload(&paths);
                }
                self.request_redraw();
            }
            UserEvent::Reloaded(batch) => {
                self.apply_reload_batch(batch);
                self.request_redraw();
            }
        }
    }
}

fn relative_time(recorded_at: SystemTime) -> String {
    let elapsed = SystemTime::now()
        .duration_since(recorded_at)
        .unwrap_or(Duration::ZERO);
    match elapsed.as_secs() {
        0..=4 => "now".to_owned(),
        seconds @ 5..=59 => format!("{seconds}s"),
        seconds @ 60..=3_599 => format!("{}m", seconds / 60),
        seconds => format!("{}h", seconds / 3_600),
    }
}

fn push_history(stack: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(color: Color) -> f32 {
        let linear = |component: f32| {
            if component <= 0.040_45 {
                component / 12.92
            } else {
                ((component + 0.055) / 1.055).powf(2.4)
            }
        };
        let [red, green, blue, _] = color.components;
        0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
    }

    fn contrast_ratio(left: Color, right: Color) -> f32 {
        let left = relative_luminance(left);
        let right = relative_luminance(right);
        (left.max(right) + 0.05) / (left.min(right) + 0.05)
    }

    #[test]
    fn close_is_blocked_until_the_staged_session_is_resolved() {
        let mut edit_session = EditSession::default();
        let file = PathBuf::from("child.kicad_sch");
        let source = "(kicad_sch (uuid \"child\"))".to_owned();
        assert!(can_exit_without_losing_staged_edits(&edit_session));

        edit_session
            .stage_creation(file.clone(), &file, source.clone())
            .expect("stage creation");
        assert!(!can_exit_without_losing_staged_edits(&edit_session));

        edit_session
            .cancel_creation(&file, &file, &source)
            .expect("discard staged creation");
        assert!(can_exit_without_losing_staged_edits(&edit_session));
    }

    #[test]
    fn arc_interpolation_contains_endpoints() {
        let start = SchPoint { x: 1.0, y: 0.0 };
        let mid = SchPoint { x: 0.0, y: 1.0 };
        let end = SchPoint { x: -1.0, y: 0.0 };
        let points = arc_points(start, mid, end, 8);
        assert_eq!(points.first(), Some(&start));
        let last = points.last().unwrap();
        assert!((last.x - end.x).abs() < 1e-9);
        assert!((last.y - end.y).abs() < 1e-9);
    }

    #[test]
    fn matches_kicad_inductor_arc_serialization() {
        let arc = kicad_svg_arc(
            SchPoint { x: 98.30, y: 96.53 },
            SchPoint { x: 97.29, y: 95.50 },
            SchPoint { x: 96.27, y: 96.51 },
        )
        .unwrap();

        assert_eq!(arc.from.x, 98.31);
        assert_eq!(arc.from.y, 96.5301);
        assert_eq!(arc.to.x, 96.27);
        assert_eq!(arc.to.y, 96.51);
        assert_eq!(arc.radii.x, 1.02);
        assert!(arc.large_arc);
        assert!(!arc.sweep);

        let path = svg_arc_path(
            SchPoint { x: 98.30, y: 96.53 },
            SchPoint { x: 97.29, y: 95.50 },
            SchPoint { x: 96.27, y: 96.51 },
        )
        .unwrap();
        assert_eq!(
            path.elements()[1],
            vello::kurbo::PathEl::CurveTo(
                vello::kurbo::Point::new(98.315_551_757_812_5, 95.966_766_357_421_88),
                vello::kurbo::Point::new(97.863_380_432_128_9, 95.505_599_975_585_94),
                vello::kurbo::Point::new(97.300_048_828_125, 95.500_053_405_761_72),
            )
        );
    }

    #[test]
    fn toolbar_buttons_remain_inside_window() {
        for (_, rect) in VelloViewer::toolbar_buttons(1280.0) {
            assert!(rect.x0 >= 0.0);
            assert!(rect.x1 <= 1280.0);
            assert!(rect.y0 >= STATUS_HEIGHT);
            assert!(rect.y1 <= 480.0);
        }
    }

    #[test]
    fn every_toolbar_and_change_icon_encodes_without_panicking() {
        let mut scene = Scene::new();
        for icon in [
            UiIcon::Add,
            UiIcon::Commit,
            UiIcon::Delete,
            UiIcon::Discard,
            UiIcon::Duplicate,
            UiIcon::Edit,
            UiIcon::External,
            UiIcon::Fit,
            UiIcon::Follow,
            UiIcon::Grid,
            UiIcon::Highlight,
            UiIcon::Move,
            UiIcon::Redo,
            UiIcon::Scale,
            UiIcon::Snap,
            UiIcon::TextSelect,
            UiIcon::Theme,
            UiIcon::Transform,
            UiIcon::Undo,
            UiIcon::Wire,
            UiIcon::ZoomIn,
            UiIcon::ZoomOut,
        ] {
            draw_ui_icon(
                &mut scene,
                icon,
                ScreenRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 32.0,
                    y1: 32.0,
                },
                Theme::Dark.palette().text,
            );
        }
    }

    #[test]
    fn edit_controls_are_compact_and_inside_the_minimum_window() {
        let controls = VelloViewer::edit_controls(720.0);

        assert_eq!(controls.len(), 3);
        assert!(controls.iter().all(|(_, rect)| {
            rect.x0 >= SIDEBAR_WIDTH
                && rect.x1 <= 720.0
                && rect.y0 >= 0.0
                && rect.y1 <= STATUS_HEIGHT
        }));
        assert!(controls.windows(2).all(|pair| pair[0].1.x1 < pair[1].1.x0));
    }

    #[test]
    fn selectable_text_extracts_unicode_ranges_without_byte_slicing() {
        let target = SelectableText {
            text: "AΩB".to_owned(),
            rect: ScreenRect {
                x0: 0.0,
                y0: 0.0,
                x1: 30.0,
                y1: 10.0,
            },
            character_x: vec![0.0, 10.0, 20.0, 30.0],
            select_whole: false,
        };

        assert_eq!(target.character_at(16.0), 2);
        assert_eq!(target.selected_text(1, 2), "Ω");
        assert_eq!(target.selected_text(3, 1), "ΩB");
        assert_eq!(target.selection_rect(1, 3).x0, 10.0);
        assert_eq!(target.selection_rect(1, 3).x1, 30.0);
    }

    #[test]
    fn sheet_names_are_humanized_without_corrupting_part_numbers() {
        assert_eq!(display_sheet_name("01_primary_bms"), "Primary BMS");
        assert_eq!(
            display_sheet_name("02_secondary_protection"),
            "Secondary Protection"
        );
        assert_eq!(display_sheet_name("6s-bms-bq40z80"), "6S BMS BQ40Z80");
    }

    #[test]
    fn dark_palette_has_accessible_ui_and_schematic_contrast() {
        let palette = Theme::Dark.palette();
        for surface in [
            palette.app,
            palette.toolbar,
            palette.filmstrip,
            palette.card,
        ] {
            assert!(
                contrast_ratio(palette.text, surface) >= 7.0,
                "primary UI text must retain enhanced contrast"
            );
        }
        assert!(contrast_ratio(palette.card_border, palette.card) >= 3.0);

        for foreground in [
            palette.accent,
            palette.border,
            palette.bus,
            palette.junction,
            palette.label,
            palette.no_connect,
            palette.pin,
            palette.sheet_file,
            palette.symbol,
            palette.text,
            palette.wire,
            palette.selection,
        ] {
            assert!(
                contrast_ratio(foreground, palette.page) >= 4.5,
                "schematic foreground must remain legible on the dark page"
            );
        }
    }

    #[test]
    fn object_kinds_remain_editor_visible() {
        assert_ne!(
            crate::native_scene::ObjectKind::Symbol,
            crate::native_scene::ObjectKind::Wire
        );
    }

    #[test]
    fn latency_summary_uses_nearest_rank_p95() {
        let mut samples = (1..=20).map(f64::from).collect::<Vec<_>>();

        let summary = latency_summary(&mut samples);

        assert_eq!(summary.mean_ms, 10.5);
        assert_eq!(summary.p95_ms, 19.0);
        assert_eq!(summary.max_ms, 20.0);
    }

    #[test]
    fn next_sheet_instance_uses_root_uuid_and_next_numeric_page() {
        let source = r#"(kicad_sch
          (uuid "root-a")
          (sheet (at 1 1) (size 10 10) (uuid "sheet-a")
            (instances (project "demo" (path "/root-a" (page "4")))))
          (sheet (at 20 1) (size 10 10) (uuid "sheet-b")
            (instances (project "demo" (path "/root-a" (page "2"))))))"#;

        assert_eq!(
            next_sheet_instance(source).unwrap(),
            ("/root-a".to_owned(), "5".to_owned())
        );
    }

    #[test]
    fn sheet_pin_projection_uses_the_nearest_border_and_snaps_along_it() {
        let rectangle = (10.0, 20.0, 90.0, 70.0);

        let (right, right_rotation) =
            nearest_sheet_edge(rectangle, SchPoint { x: 88.0, y: 33.1 }, 2.54, true);
        let (top, top_rotation) =
            nearest_sheet_edge(rectangle, SchPoint { x: 44.2, y: 20.4 }, 2.54, true);

        assert_eq!(right.x, 90.0);
        assert!((right.y / 2.54 - (right.y / 2.54).round()).abs() < 1e-9);
        assert_eq!(right_rotation, 0.0);
        assert_eq!(top.y, 20.0);
        assert!((top.x / 2.54 - (top.x / 2.54).round()).abs() < 1e-9);
        assert_eq!(top_rotation, 90.0);
    }

    #[test]
    fn sheet_rectangle_reads_exact_at_and_size_geometry() {
        let source = r#"(kicad_sch
            (sheet (at 12.7 25.4) (size 80 50) (uuid "sheet-a")))"#;

        assert_eq!(
            sheet_rectangle(source, "sheet-a"),
            Some((12.7, 25.4, 92.7, 75.4))
        );
    }

    #[test]
    fn background_reload_loads_changed_but_not_known_unchanged_sheet() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("root.kicad_sch");
        std::fs::write(
            &path,
            r#"(kicad_sch (version 20250101) (uuid "root") (paper "A4"))"#,
        )
        .expect("write schematic");
        let key = path_key(&path);
        let generation = AtomicU64::new(1);
        let unchanged = ReloadRequest {
            generation: 1,
            root: path.clone(),
            changed: HashSet::new(),
            known: HashSet::from([key.clone()]),
            external: false,
        };
        let changed = ReloadRequest {
            generation: 1,
            root: path,
            changed: HashSet::from([key.clone()]),
            known: HashSet::from([key.clone()]),
            external: false,
        };

        let unchanged_batch =
            build_reload_batch(&unchanged, &generation).expect("current batch completes");
        let changed_batch =
            build_reload_batch(&changed, &generation).expect("current batch completes");

        assert!(unchanged_batch.loaded.is_empty());
        assert!(matches!(changed_batch.loaded.get(&key), Some(Ok(_))));
    }

    #[test]
    fn superseded_background_reload_is_cancelled() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("root.kicad_sch");
        std::fs::write(
            &path,
            r#"(kicad_sch (version 20250101) (uuid "root") (paper "A4"))"#,
        )
        .expect("write schematic");
        let request = ReloadRequest {
            generation: 1,
            root: path.clone(),
            changed: HashSet::from([path_key(&path)]),
            known: HashSet::new(),
            external: false,
        };
        let generation = AtomicU64::new(2);

        assert!(build_reload_batch(&request, &generation).is_none());
    }

    #[cfg(feature = "golden-svg-reference")]
    #[test]
    fn same_vello_oracle_flattens_kicad_style_groups() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"
            viewBox="0 0 10 10"><g style="fill:#f5f4ef;stroke:#840000;stroke-width:0.1524">
            <rect x="0" y="0" width="10" height="10"/></g></svg>"##;
        let tree =
            vello_svg::usvg::Tree::from_str(svg, &vello_svg::usvg::Options::default()).unwrap();
        let mut scene = Scene::new();
        append_svg_group_flat(&mut scene, tree.root());
    }
}
