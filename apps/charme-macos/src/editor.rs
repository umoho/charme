mod docking;
mod hierarchy;
mod inspector;
mod viewport;

pub(crate) use hierarchy::HierarchyItemId;

use std::{
    cell::RefCell,
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    ptr,
    sync::OnceLock,
};

use cacao::objc::declare::ClassDecl;
use cacao::objc::runtime::{Class, Object, Sel};
use cacao::{
    appkit::{
        App,
        toolbar::{ItemIdentifier, Toolbar, ToolbarDelegate, ToolbarItem},
        window::{TitleVisibility, Window, WindowDelegate},
    },
    color::{Color, Theme},
    filesystem::FileSelectPanel,
    foundation::{BOOL, NO, YES, id},
    image::{Image, ImageView},
    layout::{Layout, LayoutConstraint},
    objc::{class, msg_send, sel, sel_impl},
    text::Label,
    view::View,
};
use charme_core::{
    CharacterSource, EditorCommand, EditorSession, MaterialId, MaterialInstance, ParameterValue,
    ResourcePath, ResourcePathError, ShaderSource as DocumentShaderSource,
};
use charme_renderer::{Frame, OutputSize, PmxSceneInfo, RendererNotification};
use core_graphics::geometry::{CGPoint, CGRect};

use self::{
    docking::{
        Axis, DockNode, DockTree, DockTreeBuilder, LayoutOptions, NodeId, PanelId, Rect,
        compute_geometry,
    },
    hierarchy::HierarchyView,
    inspector::ParameterControl,
    viewport::{BrightnessSlider, OrbitInputView, make_image},
};
use crate::{
    app::{CharmeApp, MenuContext, Message},
    localization::{self, Key},
    preview::RenderBridge,
    shader_inspection::{self, ParameterControlKind, ShaderInspection},
    ui::{label, panel},
};

const DOCK_DIVIDER_THICKNESS: f64 = 2.0;
const EDITOR_CONTENT_TOP_INSET: f64 = 52.0;
const EDITOR_TOOLBAR_SEPARATOR_THICKNESS: f64 = 2.0;
const DOCK_DIVIDER_HIT_SLOP: f64 = 4.0;
const DOCK_DIVIDER_TARGET_IVAR: &str = "charmeDockDividerTarget";
const DOCK_DIVIDER_AXIS_IVAR: &str = "charmeDockDividerAxis";

struct DockDividerTarget {
    owner: *mut EditorWindow,
    node: NodeId,
}

struct DockDivider {
    visual: View,
    input: id,
    target: Box<DockDividerTarget>,
    axis: Axis,
}

impl DockDivider {
    pub(crate) fn new(node: NodeId, axis: Axis) -> Self {
        let visual = panel(editor_separator_color());
        visual.set_translates_autoresizing_mask_into_constraints(true);
        let input = unsafe {
            let input: id = msg_send![dock_divider_input_class(), new];
            let _: () = msg_send![input, setTranslatesAutoresizingMaskIntoConstraints: YES];
            input
        };
        let mut target = Box::new(DockDividerTarget {
            owner: ptr::null_mut(),
            node,
        });

        unsafe {
            let target_ptr = (&mut *target as *mut DockDividerTarget) as usize;
            (&mut *input).set_ivar(DOCK_DIVIDER_TARGET_IVAR, target_ptr);
            (&mut *input).set_ivar(
                DOCK_DIVIDER_AXIS_IVAR,
                match axis {
                    Axis::Horizontal => 0usize,
                    Axis::Vertical => 1usize,
                },
            );
        }

        Self {
            visual,
            input,
            target,
            axis,
        }
    }

    fn set_owner(&mut self, owner: *mut EditorWindow) {
        self.target.owner = owner;
    }

    fn install(&self, parent: &View) {
        parent.add_subview(&self.visual);
        parent.objc.with_mut(|parent| unsafe {
            let _: () = msg_send![parent, addSubview: self.input];
        });
    }

    fn set_frame(&self, rect: Rect) {
        self.visual.set_frame(to_cacao_rect(rect));
        let input_rect = match self.axis {
            Axis::Horizontal => Rect::new(
                rect.x - DOCK_DIVIDER_HIT_SLOP,
                rect.y,
                rect.width + DOCK_DIVIDER_HIT_SLOP * 2.0,
                rect.height,
            ),
            Axis::Vertical => Rect::new(
                rect.x,
                rect.y - DOCK_DIVIDER_HIT_SLOP,
                rect.width,
                rect.height + DOCK_DIVIDER_HIT_SLOP * 2.0,
            ),
        };
        let frame: CGRect = to_cacao_rect(input_rect).into();
        unsafe {
            let _: () = msg_send![self.input, setFrame: frame];
            let window: id = msg_send![self.input, window];
            if !window.is_null() {
                let _: () = msg_send![window, invalidateCursorRectsForView: self.input];
            }
        }
    }
}

impl Drop for DockDivider {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.input, removeFromSuperview];
            let _: () = msg_send![self.input, release];
        }
    }
}

#[derive(Clone, Copy)]
struct DividerDrag {
    node: NodeId,
    axis: Axis,
    start_coordinate: f64,
    start_first_extent: f64,
    available_extent: f64,
}

struct EditorToolbar;

impl ToolbarDelegate for EditorToolbar {
    const NAME: &'static str = "CharmeEditorToolbar";

    fn did_load(&mut self, toolbar: Toolbar) {
        toolbar.set_shows_baseline_separator(false);
    }

    fn allowed_item_identifiers(&self) -> Vec<ItemIdentifier> {
        vec![ItemIdentifier::Space]
    }

    fn default_item_identifiers(&self) -> Vec<ItemIdentifier> {
        vec![ItemIdentifier::Space]
    }

    fn item_for(&self, _: &str) -> &ToolbarItem {
        unreachable!("the empty Charme toolbar has no items")
    }
}

pub(crate) struct EditorWindow {
    toolbar: Toolbar<EditorToolbar>,
    toolbar_divider: View,
    content: View,
    tree: DockTree,
    dividers: BTreeMap<NodeId, DockDivider>,
    drag: Option<DividerDrag>,
    sidebar: View,
    viewport: View,
    inspector: View,
    image_view: ImageView,
    orbit_input: OrbitInputView,
    status: Label,
    hierarchy: HierarchyView,
    loaded_scene: RefCell<Option<PmxSceneInfo>>,
    inspector_heading: Label,
    inspector_body: Label,
    parameter_panel: View,
    parameter_controls: RefCell<Vec<ParameterControl>>,
    pub(crate) session: RefCell<EditorSession>,
    active_material: RefCell<Option<MaterialId>>,
    brightness_label: Label,
    brightness: BrightnessSlider,
    current_image: RefCell<Option<Image>>,
    bridge: RefCell<Option<RenderBridge>>,
}

impl EditorWindow {
    pub(crate) fn new() -> Self {
        let toolbar = Toolbar::new("com.umoho.charme.editor", EditorToolbar);
        let toolbar_divider = panel(editor_separator_color());
        toolbar_divider.set_translates_autoresizing_mask_into_constraints(true);
        let content = panel(Color::MacOSWindowBackgroundColor);
        let sidebar = panel(editor_panel_color());
        let viewport = panel(Color::SystemBlack);
        let inspector = panel(editor_panel_color());
        let (tree, dividers) = default_dock_layout();
        let image_view = ImageView::new();
        image_view.set_background_color(Color::SystemBlack);
        let orbit_input = OrbitInputView::new();
        let hierarchy = HierarchyView::new();

        let inspector_heading = label(
            localization::text(Key::Inspector),
            11.0,
            true,
            Color::LabelSecondary,
        );
        let inspector_body = label(
            localization::text(Key::InspectorBody),
            13.0,
            false,
            Color::Label,
        );
        inspector_body.set_max_number_of_lines(0);
        let parameter_panel = View::new();
        let brightness_label = label(
            localization::text(Key::Brightness),
            12.0,
            false,
            Color::Label,
        );
        let brightness = BrightnessSlider::new(0.3);
        let status = label(
            localization::text(Key::RendererStarting),
            11.0,
            false,
            Color::SystemWhite,
        );

        Self {
            toolbar,
            toolbar_divider,
            content,
            tree,
            dividers,
            drag: None,
            sidebar,
            viewport,
            inspector,
            image_view,
            orbit_input,
            status,
            hierarchy,
            loaded_scene: RefCell::new(None),
            inspector_heading,
            inspector_body,
            parameter_panel,
            parameter_controls: RefCell::new(Vec::new()),
            session: RefCell::new(EditorSession::new(localization::text(
                Key::UntitledCharacter,
            ))),
            active_material: RefCell::new(None),
            brightness_label,
            brightness,
            current_image: RefCell::new(None),
            bridge: RefCell::new(None),
        }
    }

    pub(crate) fn install_session(&self, session: EditorSession) {
        self.session.replace(session);
        self.active_material.replace(None);
        self.parameter_controls.borrow_mut().clear();
        self.loaded_scene.replace(None);
        self.hierarchy.clear();
        self.inspector_heading
            .set_text(localization::text(Key::Inspector));
        self.inspector_body
            .set_text(localization::text(Key::InspectorBody));
        App::<CharmeApp, Message>::dispatch_main(Message::RefreshMenus);
    }

    pub(crate) fn reset_session(&self) {
        self.session
            .replace(EditorSession::new(localization::text(Key::UntitledProject)));
        self.active_material.replace(None);
        self.parameter_controls.borrow_mut().clear();
        self.loaded_scene.replace(None);
        self.hierarchy.clear();
        self.inspector_heading
            .set_text(localization::text(Key::Inspector));
        self.inspector_body
            .set_text(localization::text(Key::InspectorBody));
        App::<CharmeApp, Message>::dispatch_main(Message::RefreshMenus);
    }

    pub(crate) fn save_project(&self) -> Result<(), charme_core::SessionPersistenceError> {
        self.session.borrow_mut().save()
    }

    pub(crate) fn save_project_as(
        &self,
        path: PathBuf,
    ) -> Result<(), charme_core::SessionPersistenceError> {
        self.session.borrow_mut().save_as(path)
    }

    pub(crate) fn start_renderer(&self) {
        if self.bridge.borrow().is_some() {
            return;
        }
        let (size, scale) = self.viewport_metrics();
        *self.bridge.borrow_mut() = Some(RenderBridge::start(size, scale));
    }

    fn sync_size(&self) {
        let bridge = self.bridge.borrow();
        let Some(bridge) = bridge.as_ref() else {
            return;
        };
        let (size, scale) = self.viewport_metrics();
        bridge.resize(size, scale);
    }

    fn viewport_metrics(&self) -> (OutputSize, f64) {
        self.viewport.objc.get(|view| unsafe {
            let bounds: CGRect = msg_send![view, bounds];
            let window: id = msg_send![view, window];
            let scale: f64 = if window.is_null() {
                1.0
            } else {
                msg_send![window, backingScaleFactor]
            };
            let width = (bounds.size.width.max(1.0) * scale).round() as u32;
            let height = (bounds.size.height.max(1.0) * scale).round() as u32;
            (OutputSize::new(width, height), scale)
        })
    }

    pub(crate) fn display(&self, frame: Frame, scale: f64) {
        let sequence = frame.sequence();
        let width = frame.width();
        let height = frame.height();
        match make_image(frame, scale) {
            Ok(image) => {
                self.image_view.set_image(&image);
                *self.current_image.borrow_mut() = Some(image);
                self.status.set_text(localization::format(
                    Key::FrameStatus,
                    &[
                        ("sequence", &sequence),
                        ("width", &width),
                        ("height", &height),
                    ],
                ));
            }
            Err(error) => {
                eprintln!("Failed to create the rendered frame image: {error}");
                self.show_error(&error);
            }
        }
    }

    pub(crate) fn import_pmx(&self, path: PathBuf) {
        let resource = {
            let session = self.session.borrow();
            pmx_resource_path(session.project_path(), &path)
        };
        let resource = match resource {
            Ok(resource) => resource,
            Err(error) => {
                eprintln!("Failed to store PMX path {}: {error}", path.display());
                self.show_error(&localization::format(
                    Key::PmxLoadFailed,
                    &[("path", &path.display())],
                ));
                return;
            }
        };
        if let Err(error) = self
            .session
            .borrow_mut()
            .apply(EditorCommand::SetCharacter(Some(CharacterSource::pmx(
                resource,
            ))))
        {
            eprintln!("Failed to update the project character: {error}");
            self.show_error(&localization::format(
                Key::PmxLoadFailed,
                &[("path", &path.display())],
            ));
            return;
        }
        App::<CharmeApp, Message>::dispatch_main(Message::RefreshMenus);
        self.load_pmx(path);
    }

    pub(crate) fn load_pmx(&self, path: PathBuf) {
        self.status.set_text(localization::format(
            Key::LoadingPmx,
            &[("path", &path.display())],
        ));
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.load_pmx(path);
        }
    }

    pub(crate) fn choose_shader(&self) {
        let mut panel = FileSelectPanel::new();
        panel.set_can_choose_files(true);
        panel.set_can_choose_directories(false);
        panel.set_allows_multiple_selection(false);
        panel.set_message(localization::text(Key::ChooseShaderMessage));
        panel.show(|urls| {
            if let Some(url) = urls.first() {
                App::<CharmeApp, Message>::dispatch_main(Message::InspectShader(url.pathbuf()));
            }
        });
    }

    pub(crate) fn inspect_shader(&self, path: PathBuf) {
        self.inspector_heading
            .set_text(localization::text(Key::InspectingShader));
        self.inspector_body.set_text(path.display().to_string());
        self.parameter_controls.borrow_mut().clear();
        shader_inspection::inspect_shader(path);
    }

    pub(crate) fn show_shader_result(
        &self,
        path: PathBuf,
        result: Result<ShaderInspection, String>,
    ) {
        let inspection = match result {
            Ok(inspection) => inspection,
            Err(error) => {
                self.inspector_heading
                    .set_text(localization::text(Key::ShaderError));
                self.inspector_body.set_text(localization::format(
                    Key::ShaderErrorDetails,
                    &[("path", &path.display()), ("error", &error)],
                ));
                self.status
                    .set_text(localization::text(Key::ReflectionFailed));
                return;
            }
        };

        self.inspector_heading
            .set_text(localization::text(Key::MaterialInspector));
        let file_name = inspection
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(localization::text(Key::WgslShader));
        self.install_document_material(&inspection.path, file_name);
        let summary_key = if inspection.non_scalar_field_count == 0 {
            Key::ShaderSummary
        } else {
            Key::ShaderSummaryWithNonScalar
        };
        self.inspector_body.set_text(localization::format(
            summary_key,
            &[
                ("file_name", &file_name),
                ("parameter_blocks", &inspection.parameter_block_count),
                ("diagnostics", &inspection.diagnostics.len()),
                ("non_scalar_fields", &inspection.non_scalar_field_count),
            ],
        ));

        let mut controls = self.parameter_controls.borrow_mut();
        controls.clear();
        for (index, spec) in inspection.controls.iter().take(8).enumerate() {
            let control = ParameterControl::new(spec);
            self.parameter_panel.add_subview(&control.view);
            LayoutConstraint::activate(&[
                control
                    .view
                    .top
                    .constraint_equal_to(&self.parameter_panel.top)
                    .offset(index as f64 * 58.0),
                control
                    .view
                    .leading
                    .constraint_equal_to(&self.parameter_panel.leading),
                control
                    .view
                    .trailing
                    .constraint_equal_to(&self.parameter_panel.trailing),
                control.view.height.constraint_equal_to_constant(48.0),
            ]);
            controls.push(control);
        }
        self.status.set_text(localization::format(
            Key::ShaderReflected,
            &[("file_name", &file_name), ("controls", &controls.len())],
        ));
    }

    fn install_document_material(&self, path: &std::path::Path, name: &str) {
        let resource = if path.is_absolute() {
            ResourcePath::absolute(path.to_path_buf())
        } else {
            ResourcePath::project_relative("assets/shaders/preview_material.wgsl")
        };
        let Ok(resource) = resource else {
            return;
        };
        let shader = DocumentShaderSource::new(name, resource);
        let material = MaterialInstance::new(name, shader.id());
        let material_id = material.id();
        let mut session = self.session.borrow_mut();
        if session
            .apply(EditorCommand::UpsertShader(shader))
            .and_then(|_| session.apply(EditorCommand::UpsertMaterial(material)))
            .is_ok()
        {
            *self.active_material.borrow_mut() = Some(material_id);
        }
    }

    pub(crate) fn set_parameter_value(&self, key: &str, value: f64, kind: ParameterControlKind) {
        if let Some(control) = self
            .parameter_controls
            .borrow()
            .iter()
            .find(|control| control.key() == key)
        {
            control.set_value(value, kind);
        }
        let parameter = match kind {
            ParameterControlKind::Float => ParameterValue::F32(value as f32),
            ParameterControlKind::SignedInteger => ParameterValue::I32(value as i32),
            ParameterControlKind::UnsignedInteger => ParameterValue::U32(value.max(0.0) as u32),
        };
        let active_material = *self.active_material.borrow();
        let updated = active_material.and_then(|material| {
            self.session
                .borrow_mut()
                .apply(EditorCommand::SetMaterialParameter {
                    material,
                    path: key.to_owned(),
                    value: Some(parameter.clone()),
                })
                .ok()
        });
        if updated.is_some()
            && let Some(bridge) = self.bridge.borrow().as_ref()
        {
            bridge.set_material_parameter(key.to_owned(), parameter);
        }
        let formatted_value = format!("{value:.3}");
        self.status.set_text(localization::format(
            if updated.is_some() {
                Key::ParameterUpdated
            } else {
                Key::ParameterWaiting
            },
            &[("key", &key), ("value", &formatted_value)],
        ));
        App::<CharmeApp, Message>::dispatch_main(Message::RefreshMenus);
    }

    pub(crate) fn handle_renderer_notification(&self, notification: RendererNotification) {
        match notification {
            RendererNotification::PmxLoaded(info) => self.show_scene_info(&info),
            RendererNotification::PmxLoadFailed { path, message } => {
                eprintln!("Failed to load PMX {}: {message}", path.display());
                self.show_error(&localization::format(
                    Key::PmxLoadFailed,
                    &[("path", &path.display())],
                ));
            }
            RendererNotification::MaterialThumbnailReady {
                path,
                slot_index,
                frame,
            } => {
                let scene_matches = self
                    .loaded_scene
                    .borrow()
                    .as_ref()
                    .is_some_and(|scene| scene.path() == path);
                if !scene_matches {
                    return;
                }
                match make_image(frame, 1.0) {
                    Ok(image) => self.hierarchy.set_material_thumbnail(slot_index, &image),
                    Err(error) => eprintln!("Failed to create material thumbnail: {error}"),
                }
            }
            RendererNotification::MaterialParameterRejected { path, message } => {
                eprintln!("Renderer rejected parameter {path}: {message}");
                self.show_error(&localization::format(
                    Key::ParameterRejected,
                    &[("path", &path)],
                ));
            }
            _ => {}
        }
    }

    fn show_scene_info(&self, info: &PmxSceneInfo) {
        self.hierarchy.set_scene(info);
        self.loaded_scene.replace(Some(info.clone()));
        self.select_hierarchy_item(HierarchyItemId::Model);
        self.status.set_text(localization::format(
            if info.warnings().is_empty() {
                Key::SceneLoaded
            } else {
                Key::SceneLoadedWithWarnings
            },
            &[
                ("name", &info.name()),
                ("slots", &info.material_slots().len()),
                ("warnings", &info.warnings().len()),
            ],
        ));
    }

    pub(crate) fn select_hierarchy_item(&self, item: HierarchyItemId) {
        let scene = self.loaded_scene.borrow();
        let Some(info) = scene.as_ref() else {
            self.inspector_heading
                .set_text(localization::text(Key::Inspector));
            self.inspector_body
                .set_text(localization::text(Key::EmptyScene));
            return;
        };

        match item {
            HierarchyItemId::Scene | HierarchyItemId::Model => {
                self.inspector_heading.set_text(info.name());
                self.inspector_body.set_text(localization::format(
                    Key::SceneSummary,
                    &[
                        ("name", &info.name()),
                        ("vertices", &info.vertex_count()),
                        ("indices", &info.index_count()),
                    ],
                ));
            }
            HierarchyItemId::Materials => {
                self.inspector_heading
                    .set_text(localization::text(Key::Materials));
                self.inspector_body.set_text(localization::format(
                    Key::SceneLoaded,
                    &[
                        ("name", &info.name()),
                        ("slots", &info.material_slots().len()),
                    ],
                ));
            }
            HierarchyItemId::MaterialSlot(index) => {
                let Some(slot) = info.material_slots().get(index) else {
                    return;
                };
                self.inspector_heading.set_text(slot.name());
                let missing = localization::text(Key::MissingValue);
                self.inspector_body.set_text(localization::format(
                    Key::MaterialDetails,
                    &[
                        ("name", &slot.name()),
                        ("index", &slot.index()),
                        ("diffuse", &slot.diffuse_texture().unwrap_or(missing)),
                        ("sphere", &slot.sphere_texture().unwrap_or(missing)),
                        ("toon", &slot.toon_texture().unwrap_or(missing)),
                    ],
                ));
            }
        }
    }

    pub(crate) fn set_brightness(&self, value: f32) {
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.set_brightness(value);
        }
    }

    pub(crate) fn orbit(&self, delta_x: f32, delta_y: f32) {
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.orbit(delta_x, delta_y);
        }
    }

    pub(crate) fn zoom(&self, delta: f32) {
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.zoom(delta);
        }
    }

    pub(crate) fn show_error(&self, error: &str) {
        self.status
            .set_text(format!("{}{error}", localization::text(Key::ErrorPrefix)));
    }

    fn content_bounds(&self) -> Rect {
        let bounds: CGRect = self
            .content
            .objc
            .get(|view| unsafe { msg_send![view, bounds] });
        Rect::new(
            0.0,
            EDITOR_CONTENT_TOP_INSET,
            bounds.size.width,
            (bounds.size.height - EDITOR_CONTENT_TOP_INSET).max(0.0),
        )
    }

    fn layout_dock(&self) {
        let bounds: CGRect = self
            .content
            .objc
            .get(|view| unsafe { msg_send![view, bounds] });
        self.toolbar_divider.set_frame(cacao::geometry::Rect::new(
            EDITOR_CONTENT_TOP_INSET - EDITOR_TOOLBAR_SEPARATOR_THICKNESS,
            0.0,
            bounds.size.width,
            EDITOR_TOOLBAR_SEPARATOR_THICKNESS,
        ));

        let geometry = compute_geometry(
            &self.tree,
            self.content_bounds(),
            LayoutOptions {
                divider_thickness: DOCK_DIVIDER_THICKNESS,
            },
        )
        .expect("the default dock tree should produce valid geometry");

        for pane in geometry.panes {
            let view = match self.tree.node(pane.node) {
                Some(DockNode::Tabs { panels, .. })
                    if panels.iter().any(|id| id.as_str() == "hierarchy") =>
                {
                    &self.sidebar
                }
                Some(DockNode::Tabs { panels, .. })
                    if panels.iter().any(|id| id.as_str() == "viewport") =>
                {
                    &self.viewport
                }
                Some(DockNode::Tabs { panels, .. })
                    if panels.iter().any(|id| id.as_str() == "inspector") =>
                {
                    &self.inspector
                }
                _ => continue,
            };
            view.set_frame(to_cacao_rect(pane.rect));
        }

        for divider in geometry.dividers {
            if let Some(view) = self.dividers.get(&divider.node) {
                view.set_frame(divider.rect);
            }
        }
    }

    fn event_position(&self, event: id) -> CGPoint {
        let window_position: CGPoint = unsafe { msg_send![event, locationInWindow] };
        let no_view: id = ptr::null_mut();
        self.content
            .objc
            .get(|view| unsafe { msg_send![view, convertPoint: window_position fromView: no_view] })
    }

    fn begin_divider_drag(&mut self, node: NodeId, event: id) {
        let (axis, ratio) = match self.tree.node(node) {
            Some(DockNode::Split { axis, ratio, .. }) => (*axis, *ratio),
            _ => return,
        };
        let Ok(geometry) = compute_geometry(
            &self.tree,
            self.content_bounds(),
            LayoutOptions {
                divider_thickness: DOCK_DIVIDER_THICKNESS,
            },
        ) else {
            return;
        };
        let Some(divider) = geometry.dividers.iter().find(|item| item.node == node) else {
            return;
        };
        let split_extent = axis_extent(axis, divider.split_rect);
        let divider_extent = axis_extent(axis, divider.rect);
        let available_extent = split_extent - divider_extent;
        if available_extent <= 0.0 {
            return;
        }
        self.drag = Some(DividerDrag {
            node,
            axis,
            start_coordinate: axis_coordinate(axis, self.event_position(event)),
            start_first_extent: available_extent * ratio.get(),
            available_extent,
        });
    }

    fn update_divider_drag(&mut self, _node: NodeId, event: id) {
        let Some(drag) = self.drag else {
            return;
        };
        let delta = axis_coordinate(drag.axis, self.event_position(event)) - drag.start_coordinate;
        let ratio = (drag.start_first_extent + delta) / drag.available_extent;
        if self.tree.set_split_ratio(drag.node, ratio).is_ok() {
            self.layout_dock();
        }
    }

    fn end_divider_drag(&mut self, node: NodeId, event: id) {
        self.update_divider_drag(node, event);
        self.drag = None;
    }
}

impl WindowDelegate for EditorWindow {
    const NAME: &'static str = "CharmeEditorWindow";

    fn did_become_key(&self) {
        App::<CharmeApp, Message>::dispatch_main(Message::MenuContextChanged(MenuContext::Editor));
    }

    fn did_load(&mut self, window: Window) {
        window.set_title(localization::text(Key::AppName));
        window.set_title_visibility(TitleVisibility::Hidden);
        window.set_titlebar_appears_transparent(true);
        window.set_titlebar_separator_style(0);
        window.set_toolbar(&self.toolbar);
        window.set_shows_toolbar_button(false);
        window.set_content_view(&self.content);
        window.set_minimum_content_size(900.0, 560.0);

        self.content
            .set_translates_autoresizing_mask_into_constraints(true);
        for view in [&self.sidebar, &self.viewport, &self.inspector] {
            view.set_translates_autoresizing_mask_into_constraints(true);
            self.content.add_subview(view);
        }
        self.content.add_subview(&self.toolbar_divider);
        for divider in self.dividers.values() {
            divider.install(&self.content);
        }
        let owner = self as *mut EditorWindow;
        for divider in self.dividers.values_mut() {
            divider.set_owner(owner);
        }
        self.viewport.add_subview(&self.image_view);
        self.viewport.add_subview(&self.orbit_input.view);
        self.viewport.add_subview(&self.status);
        self.sidebar.add_subview(self.hierarchy.view());
        self.inspector.add_subview(&self.inspector_heading);
        self.inspector.add_subview(&self.inspector_body);
        self.inspector.add_subview(&self.parameter_panel);
        self.inspector.add_subview(&self.brightness_label);
        self.inspector.add_subview(&self.brightness.view);

        LayoutConstraint::activate(&[
            self.image_view.top.constraint_equal_to(&self.viewport.top),
            self.image_view
                .bottom
                .constraint_equal_to(&self.viewport.bottom),
            self.image_view
                .leading
                .constraint_equal_to(&self.viewport.leading),
            self.image_view
                .trailing
                .constraint_equal_to(&self.viewport.trailing),
            self.orbit_input
                .view
                .top
                .constraint_equal_to(&self.viewport.top),
            self.orbit_input
                .view
                .bottom
                .constraint_equal_to(&self.viewport.bottom),
            self.orbit_input
                .view
                .leading
                .constraint_equal_to(&self.viewport.leading),
            self.orbit_input
                .view
                .trailing
                .constraint_equal_to(&self.viewport.trailing),
            self.status
                .leading
                .constraint_equal_to(&self.viewport.leading)
                .offset(14.0),
            self.status
                .bottom
                .constraint_equal_to(&self.viewport.bottom)
                .offset(-12.0),
            self.hierarchy
                .view()
                .top
                .constraint_equal_to(&self.sidebar.top),
            self.hierarchy
                .view()
                .bottom
                .constraint_equal_to(&self.sidebar.bottom),
            self.hierarchy
                .view()
                .leading
                .constraint_equal_to(&self.sidebar.leading),
            self.hierarchy
                .view()
                .trailing
                .constraint_equal_to(&self.sidebar.trailing),
            self.inspector_heading
                .top
                .constraint_equal_to(&self.inspector.top)
                .offset(22.0),
            self.inspector_heading
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.inspector_body
                .top
                .constraint_equal_to(&self.inspector_heading.bottom)
                .offset(14.0),
            self.inspector_body
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.inspector_body
                .trailing
                .constraint_equal_to(&self.inspector.trailing)
                .offset(-18.0),
            self.parameter_panel
                .top
                .constraint_equal_to(&self.inspector.top)
                .offset(132.0),
            self.parameter_panel
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.parameter_panel
                .trailing
                .constraint_equal_to(&self.inspector.trailing)
                .offset(-18.0),
            self.parameter_panel
                .bottom
                .constraint_equal_to(&self.brightness_label.top)
                .offset(-12.0),
            self.brightness
                .view
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.brightness
                .view
                .trailing
                .constraint_equal_to(&self.inspector.trailing)
                .offset(-18.0),
            self.brightness
                .view
                .bottom
                .constraint_equal_to(&self.inspector.bottom)
                .offset(-18.0),
            self.brightness
                .view
                .height
                .constraint_equal_to_constant(28.0),
            self.brightness_label
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.brightness_label
                .bottom
                .constraint_equal_to(&self.brightness.view.top)
                .offset(-4.0),
        ]);

        self.layout_dock();

        self.image_view.objc.with_mut(|image_view| unsafe {
            let _: () = msg_send![image_view, setImageScaling: 1usize];
            let low_priority: f32 = 1.0;
            for orientation in [0isize, 1isize] {
                let _: () = msg_send![image_view,
                    setContentCompressionResistancePriority: low_priority
                    forOrientation: orientation
                ];
                let _: () = msg_send![image_view,
                    setContentHuggingPriority: low_priority
                    forOrientation: orientation
                ];
            }
        });
    }

    fn did_resize(&self) {
        self.layout_dock();
        self.sync_size();
    }

    fn did_change_backing_properties(&self) {
        self.sync_size();
    }

    fn did_deminiaturize(&self) {
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.request_redraw();
        }
    }

    fn will_close(&self) {
        self.bridge.borrow_mut().take();
    }
}

fn pmx_resource_path(
    project_path: Option<&Path>,
    path: &Path,
) -> Result<ResourcePath, ResourcePathError> {
    match project_path.and_then(Path::parent) {
        Some(project_directory) => ResourcePath::from_path(project_directory, path),
        None => ResourcePath::absolute(path.to_path_buf()),
    }
}

fn default_dock_layout() -> (DockTree, BTreeMap<NodeId, DockDivider>) {
    let mut builder = DockTreeBuilder::new();
    let hierarchy = builder
        .tabs(vec![PanelId::from("hierarchy")], PanelId::from("hierarchy"))
        .expect("hierarchy tab is valid");
    let viewport = builder
        .tabs(vec![PanelId::from("viewport")], PanelId::from("viewport"))
        .expect("viewport tab is valid");
    let inspector = builder
        .tabs(vec![PanelId::from("inspector")], PanelId::from("inspector"))
        .expect("inspector tab is valid");
    let center = builder
        .split(Axis::Horizontal, 0.72, viewport, inspector)
        .expect("center dock split is valid");
    let root = builder
        .split(Axis::Horizontal, 0.22, hierarchy, center)
        .expect("root dock split is valid");
    let tree = builder.build(root).expect("default dock tree is valid");
    let dividers = BTreeMap::from([
        (root, DockDivider::new(root, Axis::Horizontal)),
        (center, DockDivider::new(center, Axis::Horizontal)),
    ]);
    (tree, dividers)
}

fn to_cacao_rect(rect: Rect) -> cacao::geometry::Rect {
    cacao::geometry::Rect::new(rect.y, rect.x, rect.width, rect.height)
}

fn axis_coordinate(axis: Axis, point: CGPoint) -> f64 {
    match axis {
        Axis::Horizontal => point.x,
        Axis::Vertical => point.y,
    }
}

fn axis_extent(axis: Axis, rect: Rect) -> f64 {
    match axis {
        Axis::Horizontal => rect.width,
        Axis::Vertical => rect.height,
    }
}

fn dispatch_dock_divider_event(
    view: &Object,
    event: id,
    handler: fn(&mut EditorWindow, NodeId, id),
) {
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let target_ptr = *view.get_ivar::<usize>(DOCK_DIVIDER_TARGET_IVAR);
        let Some(target) = (target_ptr as *mut DockDividerTarget).as_mut() else {
            return;
        };
        let Some(owner) = target.owner.as_mut() else {
            return;
        };
        handler(owner, target.node, event);
    }));
}

extern "C" fn dock_divider_mouse_down(view: &Object, _: Sel, event: id) {
    dispatch_dock_divider_event(view, event, EditorWindow::begin_divider_drag);
}

extern "C" fn dock_divider_mouse_dragged(view: &Object, _: Sel, event: id) {
    dispatch_dock_divider_event(view, event, EditorWindow::update_divider_drag);
}

extern "C" fn dock_divider_mouse_up(view: &Object, _: Sel, event: id) {
    dispatch_dock_divider_event(view, event, EditorWindow::end_divider_drag);
}

extern "C" fn dock_divider_accepts_first_mouse(_: &Object, _: Sel, _: id) -> BOOL {
    YES
}

extern "C" fn dock_divider_does_not_move_window(_: &Object, _: Sel) -> BOOL {
    NO
}

extern "C" fn dock_divider_reset_cursor_rects(view: &Object, _: Sel) {
    unsafe {
        let bounds: CGRect = msg_send![view, bounds];
        let axis = *view.get_ivar::<usize>(DOCK_DIVIDER_AXIS_IVAR);
        let cursor: id = match axis {
            0 => msg_send![class!(NSCursor), resizeLeftRightCursor],
            _ => msg_send![class!(NSCursor), resizeUpDownCursor],
        };
        let _: () = msg_send![view, addCursorRect: bounds cursor: cursor];
    }
}

fn dock_divider_input_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let mut declaration = ClassDecl::new("CharmeDockDividerInput", class!(NSView))
            .expect("dock divider input class is registered only once");
        declaration.add_ivar::<usize>(DOCK_DIVIDER_TARGET_IVAR);
        declaration.add_ivar::<usize>(DOCK_DIVIDER_AXIS_IVAR);
        declaration.add_method(
            sel!(mouseDown:),
            dock_divider_mouse_down as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(mouseDragged:),
            dock_divider_mouse_dragged as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(mouseUp:),
            dock_divider_mouse_up as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(acceptsFirstMouse:),
            dock_divider_accepts_first_mouse as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        declaration.add_method(
            sel!(mouseDownCanMoveWindow),
            dock_divider_does_not_move_window as extern "C" fn(&Object, Sel) -> BOOL,
        );
        declaration.add_method(
            sel!(resetCursorRects),
            dock_divider_reset_cursor_rects as extern "C" fn(&Object, Sel),
        );
        declaration.register()
    })
}

fn editor_panel_color() -> Color {
    Color::MacOSWindowBackgroundColor
}

fn editor_separator_color() -> Color {
    Color::dynamic(|style| match style.theme {
        Theme::Light => Color::rgb(170, 170, 170),
        Theme::Dark => Color::rgb(8, 8, 8),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_imported_pmx_relative_to_an_existing_project() {
        let resource = pmx_resource_path(
            Some(Path::new("/projects/hero/hero.charme")),
            Path::new("/projects/hero/models/hero.pmx"),
        )
        .unwrap();

        assert_eq!(
            resource,
            ResourcePath::ProjectRelative(PathBuf::from("models/hero.pmx"))
        );
    }

    #[test]
    fn stores_external_or_unsaved_pmx_as_an_absolute_path() {
        let external = pmx_resource_path(
            Some(Path::new("/projects/hero/hero.charme")),
            Path::new("/models/hero.pmx"),
        )
        .unwrap();
        let unsaved = pmx_resource_path(None, Path::new("/models/hero.pmx")).unwrap();

        assert_eq!(
            external,
            ResourcePath::Absolute(PathBuf::from("/models/hero.pmx"))
        );
        assert_eq!(external, unsaved);
    }
}
