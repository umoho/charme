mod docking;
mod hierarchy;
mod inspector;
mod viewport;

pub(crate) use hierarchy::HierarchyItemId;

use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
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
    foundation::{BOOL, NO, YES, id, nil},
    image::{Image, ImageView},
    layout::{Layout, LayoutConstraint},
    objc::{class, msg_send, sel, sel_impl},
    progress::{ProgressIndicator, ProgressIndicatorStyle},
    text::Label,
    view::View,
};
use charme_application::{
    EditorAction, EditorController, InspectorProvider, MaterialSelectionContext, SelectionTarget,
    ShaderParameterProvider, controls_for_material, inspect_preview_shader,
};
use charme_core::{
    CharacterSource, EditorCommand, MaterialId, MaterialInstance, MaterialSlot, MaterialSlotId,
    ParameterValue, ResourcePath, ResourcePathError, ShaderSource as DocumentShaderSource,
};
use charme_renderer::{Frame, OutputSize, PmxSceneInfo, RendererNotification};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};

use self::{
    docking::{
        Axis, DockNode, DockTree, DockTreeBuilder, LayoutOptions, NodeId, PanelId, Rect,
        compute_geometry,
    },
    hierarchy::HierarchyView,
    inspector::{ParameterControl, PropertyRow},
    viewport::{NavigationGizmo, OrbitInputView, make_image},
};
use crate::{
    app::{CharmeApp, MenuContext, Message},
    localization::{self, Key},
    preview::RenderBridge,
    shader_inspection::{self, ShaderInspection},
    ui::{label, panel},
};

const DOCK_DIVIDER_THICKNESS: f64 = 2.0;
const EDITOR_CONTENT_TOP_INSET: f64 = 52.0;
const EDITOR_TOOLBAR_SEPARATOR_THICKNESS: f64 = 2.0;
const PROJECT_TITLEBAR_HORIZONTAL_INSET: f64 = 8.0;
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

struct ProjectTitlebar {
    _view: View,
    _stack: View,
    title: Label,
    status: Label,
    controller: id,
}

impl ProjectTitlebar {
    fn new() -> Self {
        let view = panel(Color::Clear);
        view.set_translates_autoresizing_mask_into_constraints(true);
        let origin = CGPoint::new(0.0, 0.0);
        let size = CGSize::new(220.0, 32.0);
        view.set_frame(CGRect::new(&origin, &size));

        let title = label(
            localization::text(Key::UntitledProject),
            13.0,
            true,
            Color::Label,
        );
        title.set_max_number_of_lines(1);
        let status = label(
            localization::text(Key::Unchanged),
            10.0,
            false,
            Color::LabelSecondary,
        );
        status.set_max_number_of_lines(1);
        let stack = View::new();
        view.add_subview(&stack);
        stack.add_subview(&title);
        stack.add_subview(&status);
        LayoutConstraint::activate(&[
            stack
                .leading
                .constraint_equal_to(&view.leading)
                .offset(PROJECT_TITLEBAR_HORIZONTAL_INSET),
            stack
                .trailing
                .constraint_equal_to(&view.trailing)
                .offset(-PROJECT_TITLEBAR_HORIZONTAL_INSET),
            stack.center_y.constraint_equal_to(&view.center_y),
            stack.height.constraint_equal_to_constant(27.0),
            title.top.constraint_equal_to(&stack.top),
            title.leading.constraint_equal_to(&stack.leading),
            title.trailing.constraint_equal_to(&stack.trailing),
            title.height.constraint_equal_to_constant(16.0),
            status.top.constraint_equal_to(&title.bottom),
            status.leading.constraint_equal_to(&stack.leading),
            status.trailing.constraint_equal_to(&stack.trailing),
            status.height.constraint_equal_to_constant(11.0),
            status.bottom.constraint_equal_to(&stack.bottom),
        ]);

        let controller = unsafe {
            let controller: id = msg_send![class!(NSTitlebarAccessoryViewController), new];
            view.objc.with_mut(|view| {
                let _: () = msg_send![controller, setView: view];
            });
            // NSLayoutAttributeLeft keeps the accessory adjacent to the native traffic lights.
            let _: () = msg_send![controller, setLayoutAttribute: 1isize];
            controller
        };

        Self {
            _view: view,
            _stack: stack,
            title,
            status,
            controller,
        }
    }

    fn install<T>(&self, window: &Window<T>) {
        unsafe {
            let _: () =
                msg_send![&*window.objc, addTitlebarAccessoryViewController: self.controller];
        }
    }

    fn set_document(&self, name: &str, dirty: bool) {
        self.title.set_text(name);
        self.status.set_text(localization::text(if dirty {
            Key::UnsavedChanges
        } else {
            Key::Unchanged
        }));
    }
}

impl Drop for ProjectTitlebar {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.controller, release];
        }
    }
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

struct PendingPmxLoad {
    epoch: u64,
    path: PathBuf,
    character: Option<CharacterSource>,
}

#[derive(Default)]
struct PmxLoadTracker {
    current_epoch: u64,
    pending: VecDeque<PendingPmxLoad>,
}

impl PmxLoadTracker {
    fn begin(&mut self, path: PathBuf, character: Option<CharacterSource>) {
        self.advance_epoch();
        self.pending.push_back(PendingPmxLoad {
            epoch: self.current_epoch,
            path,
            character,
        });
    }

    fn invalidate(&mut self) {
        self.advance_epoch();
    }

    fn complete(&mut self, path: &Path) -> Option<PendingPmxLoad> {
        let request = self.pending.pop_front()?;
        (request.epoch == self.current_epoch && request.path == path).then_some(request)
    }

    fn advance_epoch(&mut self) {
        self.current_epoch = self.current_epoch.wrapping_add(1);
    }
}

pub(crate) struct EditorWindow {
    toolbar: Toolbar<EditorToolbar>,
    titlebar: ProjectTitlebar,
    toolbar_divider: View,
    content: View,
    tree: DockTree,
    dividers: BTreeMap<NodeId, DockDivider>,
    drag: Option<DividerDrag>,
    sidebar: View,
    viewport: View,
    inspector: View,
    inspector_label: Label,
    image_view: ImageView,
    orbit_input: OrbitInputView,
    navigation_gizmo: NavigationGizmo,
    status: Label,
    hierarchy_label: Label,
    hierarchy: HierarchyView,
    pmx_loads: RefCell<PmxLoadTracker>,
    loaded_scene: RefCell<Option<PmxSceneInfo>>,
    active_inspector_slot: RefCell<Option<MaterialSlotId>>,
    inspector_heading: Label,
    inspector_body: Label,
    inspector_preview: ImageView,
    inspector_preview_container: View,
    inspector_spinner: ProgressIndicator,
    inspector_heading_preview_top: RefCell<Option<LayoutConstraint>>,
    inspector_heading_full_top: RefCell<Option<LayoutConstraint>>,
    parameter_section_preview_top: RefCell<Option<LayoutConstraint>>,
    parameter_section_full_top: RefCell<Option<LayoutConstraint>>,
    source_section: Label,
    source_panel: View,
    source_rows: RefCell<Vec<PropertyRow>>,
    parameter_section: Label,
    parameter_panel: View,
    parameter_controls: RefCell<Vec<ParameterControl>>,
    reflected_inspection: RefCell<Option<ShaderInspection>>,
    pub(crate) controller: RefCell<EditorController>,
    active_material: RefCell<Option<MaterialId>>,
    current_image: RefCell<Option<Image>>,
    current_inspector_preview: RefCell<Option<Image>>,
    bridge: RefCell<Option<RenderBridge>>,
}

impl EditorWindow {
    pub(crate) fn new() -> Self {
        let toolbar = Toolbar::new("com.umoho.charme.editor", EditorToolbar);
        let titlebar = ProjectTitlebar::new();
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
        let navigation_gizmo = NavigationGizmo::new();
        let hierarchy_label = label(
            localization::text(Key::Hierarchy),
            11.0,
            true,
            Color::LabelSecondary,
        );
        let hierarchy = HierarchyView::new();

        let inspector_label = label(
            localization::text(Key::Inspector),
            11.0,
            true,
            Color::LabelSecondary,
        );
        let inspector_heading = label("", 17.0, true, Color::Label);
        inspector_heading.set_max_number_of_lines(1);
        let inspector_body = label(
            localization::text(Key::InspectorBody),
            11.0,
            false,
            Color::LabelSecondary,
        );
        inspector_body.set_max_number_of_lines(0);
        let inspector_preview = ImageView::new();
        inspector_preview.set_background_color(Color::SystemGray);
        inspector_preview.set_hidden(true);
        let inspector_preview_container = panel(Color::SystemGray);
        inspector_preview_container.layer.set_corner_radius(12.0);
        let inspector_spinner = ProgressIndicator::new();
        inspector_spinner.set_style(ProgressIndicatorStyle::Spinner);
        inspector_spinner.set_indeterminate(true);
        inspector_spinner.set_hidden(true);
        inspector_preview_container.add_subview(&inspector_preview);
        inspector_preview_container.add_subview(&inspector_spinner);
        LayoutConstraint::activate(&[
            inspector_preview
                .top
                .constraint_equal_to(&inspector_preview_container.top),
            inspector_preview
                .bottom
                .constraint_equal_to(&inspector_preview_container.bottom),
            inspector_preview
                .leading
                .constraint_equal_to(&inspector_preview_container.leading),
            inspector_preview
                .trailing
                .constraint_equal_to(&inspector_preview_container.trailing),
            inspector_spinner
                .center_x
                .constraint_equal_to(&inspector_preview_container.center_x),
            inspector_spinner
                .center_y
                .constraint_equal_to(&inspector_preview_container.center_y),
            inspector_spinner.width.constraint_equal_to_constant(20.0),
            inspector_spinner.height.constraint_equal_to_constant(20.0),
        ]);
        let source_section = label(
            localization::text(Key::MaterialSource),
            11.0,
            true,
            Color::LabelSecondary,
        );
        let source_panel = panel(Color::SystemFillQuaternary);
        source_panel.layer.set_corner_radius(8.0);
        let source_rows = vec![
            PropertyRow::new(localization::text(Key::SourceSlot)),
            PropertyRow::new(localization::text(Key::DiffuseTexture)),
            PropertyRow::new(localization::text(Key::SphereTexture)),
            PropertyRow::new(localization::text(Key::ToonTexture)),
        ];
        for (index, row) in source_rows.iter().enumerate() {
            source_panel.add_subview(&row.view);
            LayoutConstraint::activate(&[
                row.view
                    .top
                    .constraint_equal_to(&source_panel.top)
                    .offset(8.0 + index as f64 * 28.0),
                row.view
                    .leading
                    .constraint_equal_to(&source_panel.leading)
                    .offset(12.0),
                row.view
                    .trailing
                    .constraint_equal_to(&source_panel.trailing)
                    .offset(-12.0),
                row.view.height.constraint_equal_to_constant(22.0),
            ]);
        }
        let parameter_section = label(
            localization::text(Key::MaterialParameters),
            11.0,
            true,
            Color::LabelSecondary,
        );
        let parameter_panel = panel(Color::SystemFillQuaternary);
        parameter_panel.layer.set_corner_radius(8.0);
        source_section.set_hidden(true);
        source_panel.set_hidden(true);
        parameter_section.set_hidden(true);
        parameter_panel.set_hidden(true);
        let status = label(
            localization::text(Key::RendererStarting),
            11.0,
            false,
            Color::SystemWhite,
        );

        Self {
            toolbar,
            titlebar,
            toolbar_divider,
            content,
            tree,
            dividers,
            drag: None,
            sidebar,
            viewport,
            inspector,
            inspector_label,
            image_view,
            orbit_input,
            navigation_gizmo,
            status,
            hierarchy_label,
            hierarchy,
            pmx_loads: RefCell::new(PmxLoadTracker::default()),
            loaded_scene: RefCell::new(None),
            active_inspector_slot: RefCell::new(None),
            inspector_heading,
            inspector_body,
            inspector_preview,
            inspector_preview_container,
            inspector_spinner,
            inspector_heading_preview_top: RefCell::new(None),
            inspector_heading_full_top: RefCell::new(None),
            parameter_section_preview_top: RefCell::new(None),
            parameter_section_full_top: RefCell::new(None),
            source_section,
            source_panel,
            source_rows: RefCell::new(source_rows),
            parameter_section,
            parameter_panel,
            parameter_controls: RefCell::new(Vec::new()),
            reflected_inspection: RefCell::new(None),
            controller: RefCell::new(EditorController::new(localization::text(
                Key::UntitledCharacter,
            ))),
            active_material: RefCell::new(None),
            current_image: RefCell::new(None),
            current_inspector_preview: RefCell::new(None),
            bridge: RefCell::new(None),
        }
    }

    pub(crate) fn install_controller(&self, controller: EditorController) {
        self.controller.replace(controller);
        self.reset_project_views();
        self.publish_view_model();
    }

    pub(crate) fn reset_controller(&self) {
        self.controller
            .replace(EditorController::new(localization::text(
                Key::UntitledProject,
            )));
        self.reset_project_views();
        self.publish_view_model();
    }

    fn reset_project_views(&self) {
        self.pmx_loads.borrow_mut().invalidate();
        App::<CharmeApp, Message>::dispatch_main(Message::PmxLoadFinished);
        self.active_material.replace(None);
        self.parameter_controls.borrow_mut().clear();
        self.reflected_inspection.replace(None);
        self.current_image.replace(None);
        self.current_inspector_preview.replace(None);
        clear_image(&self.image_view);
        clear_image(&self.inspector_preview);
        self.set_inspector_preview_visible(false);
        self.set_source_visible(false);
        self.set_parameter_section_visible(false);
        self.active_inspector_slot.replace(None);
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.set_selected_material_slot(None);
            bridge.clear_pmx();
        }
        self.loaded_scene.replace(None);
        self.hierarchy.clear();
        self.navigation_gizmo.reset();
        self.inspector_heading
            .set_text(localization::text(Key::Inspector));
        self.inspector_body
            .set_text(localization::text(Key::InspectorBody));
    }

    pub(crate) fn save_project(&self) -> Result<(), charme_application::EditorControllerError> {
        let result = self.controller.borrow_mut().save();
        if result.is_ok() {
            self.publish_view_model();
        }
        result
    }

    pub(crate) fn save_project_as(
        &self,
        path: PathBuf,
    ) -> Result<(), charme_application::EditorControllerError> {
        let result = self.controller.borrow_mut().save_as(path);
        if result.is_ok() {
            self.publish_view_model();
        }
        result
    }

    fn publish_view_model(&self) {
        let view_model = self.controller.borrow().view_model();
        self.titlebar
            .set_document(&view_model.document_name, view_model.dirty);
        let update = charme_application::EditorUpdate {
            view_model,
            event: None,
        };
        App::<CharmeApp, Message>::dispatch_main(Message::Application(
            charme_application::ApplicationEvent::EditorUpdated(update),
        ));
    }

    pub(crate) fn dispatch_action(
        &self,
        action: EditorAction,
    ) -> Result<charme_application::EditorUpdate, charme_application::EditorControllerError> {
        let update = self.controller.borrow_mut().dispatch(action)?;
        self.titlebar
            .set_document(&update.view_model.document_name, update.view_model.dirty);
        App::<CharmeApp, Message>::dispatch_main(Message::Application(
            charme_application::ApplicationEvent::EditorUpdated(update.clone()),
        ));
        Ok(update)
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
            let controller = self.controller.borrow();
            pmx_resource_path(controller.project_path(), &path)
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
        self.load_pmx(path, Some(CharacterSource::pmx(resource)));
    }

    pub(crate) fn load_pmx(&self, path: PathBuf, character: Option<CharacterSource>) {
        let existing_slot_ids = self
            .controller
            .borrow()
            .document()
            .material_slots()
            .iter()
            .map(|slot| (slot.source_index(), slot.id()))
            .collect();
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            self.pmx_loads.borrow_mut().begin(path.clone(), character);
            App::<CharmeApp, Message>::dispatch_main(Message::PmxLoadStarted(path.clone()));
            bridge.load_pmx(path, existing_slot_ids);
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
        self.set_source_visible(false);
        self.set_parameter_section_visible(false);
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
                self.set_source_visible(false);
                self.set_parameter_section_visible(false);
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
        self.reflected_inspection.replace(Some(inspection.clone()));
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

        self.set_source_visible(false);
        let control_count = self.install_parameter_controls(&inspection.controls);
        self.set_parameter_section_visible(control_count != 0);
        self.status.set_text(localization::format(
            Key::ShaderReflected,
            &[("file_name", &file_name), ("controls", &control_count)],
        ));
    }

    fn install_parameter_controls(
        &self,
        specs: &[charme_application::ParameterControlSpec],
    ) -> usize {
        let old_controls = self.parameter_controls.replace(Vec::new());
        for control in old_controls {
            control.view.objc.with_mut(|view| unsafe {
                let _: () = msg_send![view, removeFromSuperview];
            });
        }
        let mut controls = self.parameter_controls.borrow_mut();
        for (index, spec) in specs.iter().take(8).enumerate() {
            let control = ParameterControl::new(spec);
            self.parameter_panel.add_subview(&control.view);
            LayoutConstraint::activate(&[
                control
                    .view
                    .top
                    .constraint_equal_to(&self.parameter_panel.top)
                    .offset(10.0 + index as f64 * 58.0),
                control
                    .view
                    .leading
                    .constraint_equal_to(&self.parameter_panel.leading)
                    .offset(10.0),
                control
                    .view
                    .trailing
                    .constraint_equal_to(&self.parameter_panel.trailing)
                    .offset(-10.0),
                control.view.height.constraint_equal_to_constant(48.0),
            ]);
            controls.push(control);
        }
        controls.len()
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
        if self
            .dispatch_action(EditorAction::Command(EditorCommand::UpsertShader(shader)))
            .and_then(|_| {
                self.dispatch_action(EditorAction::Command(EditorCommand::UpsertMaterial(
                    material,
                )))
            })
            .is_ok()
        {
            if let Some(slot_id) = *self.active_inspector_slot.borrow() {
                let _ = self.dispatch_action(EditorAction::Command(EditorCommand::BindMaterial {
                    slot: slot_id,
                    material: Some(material_id),
                }));
            }
            *self.active_material.borrow_mut() = Some(material_id);
        }
    }

    pub(crate) fn set_parameter_value(&self, key: &str, parameter: ParameterValue) {
        if let Some(control) = self
            .parameter_controls
            .borrow()
            .iter()
            .find(|control| control.key() == key)
        {
            control.set_value(&parameter);
        }
        let active_material = *self.active_material.borrow();
        let updated = active_material.and_then(|material| {
            self.dispatch_action(EditorAction::Command(EditorCommand::SetMaterialParameter {
                material,
                path: key.to_owned(),
                value: Some(parameter.clone()),
            }))
            .ok()
        });
        if updated.is_some()
            && let (Some(slot_id), Some(bridge)) = (
                *self.active_inspector_slot.borrow(),
                self.bridge.borrow().as_ref(),
            )
        {
            bridge.set_material_parameter(slot_id, key.to_owned(), parameter.clone());
        }
        let formatted_value = format_parameter_value(&parameter);
        self.status.set_text(localization::format(
            if updated.is_some() {
                Key::ParameterUpdated
            } else {
                Key::ParameterWaiting
            },
            &[("key", &key), ("value", &formatted_value)],
        ));
    }

    pub(crate) fn handle_renderer_notification(&self, notification: RendererNotification) {
        match notification {
            RendererNotification::PmxLoaded(info) => {
                let Some(request) = self.pmx_loads.borrow_mut().complete(info.path()) else {
                    return;
                };
                if let Some(character) = request.character
                    && let Err(error) = self.dispatch_action(EditorAction::Command(
                        EditorCommand::SetCharacter(Some(character)),
                    ))
                {
                    eprintln!("Failed to commit the loaded character: {error}");
                }
                self.show_scene_info(&info);
                App::<CharmeApp, Message>::dispatch_main(Message::PmxLoadFinished);
            }
            RendererNotification::MaterialInspectorPreviewReady {
                path,
                slot_id,
                frame,
                ..
            } => {
                let slot_matches = self
                    .active_inspector_slot
                    .borrow()
                    .is_some_and(|active| active == slot_id);
                let scene_matches = self
                    .loaded_scene
                    .borrow()
                    .as_ref()
                    .is_some_and(|scene| scene.path() == path);
                if !scene_matches || !slot_matches {
                    return;
                }
                match make_image(frame, 1.0) {
                    Ok(image) => {
                        self.inspector_preview.set_image(&image);
                        self.set_inspector_preview_ready();
                        *self.current_inspector_preview.borrow_mut() = Some(image);
                    }
                    Err(error) => eprintln!("Failed to create inspector material preview: {error}"),
                }
            }
            RendererNotification::PmxLoadFailed { path, message } => {
                if self.pmx_loads.borrow_mut().complete(&path).is_some() {
                    eprintln!("Failed to load PMX {}: {message}", path.display());
                    App::<CharmeApp, Message>::dispatch_main(Message::PmxLoadFailed {
                        path,
                        message,
                    });
                }
            }
            RendererNotification::MaterialThumbnailReady {
                path,
                slot_id,
                frame,
                ..
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
                    Ok(image) => self.hierarchy.set_material_thumbnail(slot_id, &image),
                    Err(error) => eprintln!("Failed to create material thumbnail: {error}"),
                }
            }
            RendererNotification::ViewportPickResult { path, slot_id, .. } => {
                let scene_matches = self
                    .loaded_scene
                    .borrow()
                    .as_ref()
                    .is_some_and(|scene| scene.path() == path);
                if !scene_matches {
                    return;
                }
                if let Some(slot_id) = slot_id {
                    self.select_hierarchy_item(HierarchyItemId::MaterialSlot(slot_id));
                } else {
                    self.clear_selection();
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
        self.navigation_gizmo.reset();
        self.install_pmx_material_bindings(info);
        self.reflected_inspection
            .replace(inspect_preview_shader().ok());
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            let document = self.controller.borrow();
            for slot in document.document().material_slots() {
                let Some(material_id) = slot.material() else {
                    continue;
                };
                let Some(material) = document.document().material(material_id) else {
                    continue;
                };
                for (path, value) in material.parameters() {
                    bridge.set_material_parameter(slot.id(), path.clone(), value.clone());
                }
            }
        }
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

    fn install_pmx_material_bindings(&self, info: &PmxSceneInfo) {
        let Ok(shader_path) =
            ResourcePath::project_relative("assets/shaders/preview_material.wgsl")
        else {
            return;
        };
        let (existing_shader, existing_materials) = {
            let controller = self.controller.borrow();
            let document = controller.document();
            let shader = document
                .shaders()
                .iter()
                .find(|shader| shader.path() == &shader_path)
                .map(DocumentShaderSource::id);
            let materials = info
                .material_slots()
                .iter()
                .map(|slot| {
                    document
                        .material_slot(slot.id())
                        .and_then(MaterialSlot::material)
                        .filter(|material| document.material(*material).is_some())
                })
                .collect::<Vec<_>>();
            (shader, materials)
        };

        let mut commands = Vec::new();
        let shader_id = existing_shader.unwrap_or_else(|| {
            let shader = DocumentShaderSource::new("Preview Material", shader_path);
            let id = shader.id();
            commands.push(EditorCommand::UpsertShader(shader));
            id
        });
        let material_ids = info
            .material_slots()
            .iter()
            .zip(existing_materials)
            .map(|(slot, existing)| {
                existing.unwrap_or_else(|| {
                    let material = MaterialInstance::new(slot.name(), shader_id);
                    let id = material.id();
                    commands.push(EditorCommand::UpsertMaterial(material));
                    id
                })
            })
            .collect::<Vec<_>>();
        let slots = info
            .material_slots()
            .iter()
            .zip(material_ids)
            .map(|(slot, material)| {
                MaterialSlot::with_id(
                    slot.id(),
                    slot.index() as u32,
                    slot.name(),
                    slot.english_name(),
                    Some(material),
                )
            })
            .collect();
        commands.push(EditorCommand::ReplaceMaterialSlots(slots));
        let _ = self.dispatch_action(EditorAction::Command(EditorCommand::Transaction(commands)));
    }

    pub(crate) fn select_hierarchy_item(&self, item: HierarchyItemId) {
        let scene = self.loaded_scene.borrow();
        let Some(info) = scene.as_ref() else {
            self.set_source_visible(false);
            self.set_parameter_section_visible(false);
            self.inspector_body
                .set_text(localization::text(Key::EmptyScene));
            return;
        };

        self.hierarchy.select_item(item);
        match item {
            HierarchyItemId::Scene | HierarchyItemId::Model => {
                self.active_inspector_slot.replace(None);
                if let Some(bridge) = self.bridge.borrow().as_ref() {
                    bridge.set_selected_material_slot(None);
                }
                self.active_material.replace(None);
                self.set_inspector_preview_visible(false);
                self.set_source_visible(false);
                self.set_parameter_section_visible(false);
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
                self.active_inspector_slot.replace(None);
                if let Some(bridge) = self.bridge.borrow().as_ref() {
                    bridge.set_selected_material_slot(None);
                }
                self.active_material.replace(None);
                self.set_inspector_preview_visible(false);
                self.set_source_visible(false);
                self.set_parameter_section_visible(false);
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
            HierarchyItemId::MaterialSlot(slot_id) => {
                let Some(slot) = info
                    .material_slots()
                    .iter()
                    .find(|slot| slot.id() == slot_id)
                else {
                    return;
                };
                self.active_inspector_slot.replace(Some(slot_id));
                let context = MaterialSelectionContext::resolve(
                    self.controller.borrow().document(),
                    SelectionTarget::MaterialSlot(slot_id),
                );
                self.active_material.replace(context.material);
                let controls = {
                    let inspection = self.reflected_inspection.borrow();
                    let document = self.controller.borrow();
                    inspection
                        .as_ref()
                        .zip(
                            context
                                .material
                                .and_then(|id| document.document().material(id)),
                        )
                        .map(|(inspection, material)| {
                            controls_for_material(inspection, material.parameters())
                        })
                        .unwrap_or_default()
                };
                let control_count = self.install_parameter_controls(&controls);
                let has_parameter_section = self
                    .reflected_inspection
                    .borrow()
                    .as_ref()
                    .and_then(|inspection| {
                        ShaderParameterProvider
                            .provide(
                                self.controller.borrow().document(),
                                context,
                                Some(inspection),
                            )
                            .map(|section| section.has_content)
                    })
                    .unwrap_or(false);
                self.set_source_visible(true);
                self.set_parameter_section_visible(control_count != 0 && has_parameter_section);
                self.set_inspector_preview_loading();
                if let Some(bridge) = self.bridge.borrow().as_ref() {
                    bridge.set_selected_material_slot(Some(slot_id));
                    bridge.request_material_inspector_preview(slot_id);
                }
                self.inspector_heading.set_text(slot.name());
                self.inspector_body
                    .set_text(localization::text(if control_count == 0 {
                        Key::InspectorNoParameters
                    } else {
                        Key::MaterialSubtitle
                    }));
                let missing = localization::text(Key::MissingValue);
                self.set_source_values([
                    slot.index().to_string(),
                    slot.diffuse_texture().unwrap_or(missing).to_owned(),
                    slot.sphere_texture().unwrap_or(missing).to_owned(),
                    slot.toon_texture().unwrap_or(missing).to_owned(),
                ]);
            }
        }
    }

    fn set_inspector_preview_visible(&self, visible: bool) {
        self.inspector_preview_container.set_hidden(!visible);
        self.inspector_preview.set_hidden(!visible);
        self.inspector_spinner.set_hidden(true);
        self.inspector_spinner.stop_animation();
        self.set_preview_layout_expanded(visible);
    }

    fn set_preview_layout_expanded(&self, expanded: bool) {
        if let (Some(with_preview), Some(without_preview)) = (
            self.inspector_heading_preview_top.borrow().as_ref(),
            self.inspector_heading_full_top.borrow().as_ref(),
        ) {
            with_preview.set_active(expanded);
            without_preview.set_active(!expanded);
        }
    }

    fn set_inspector_preview_loading(&self) {
        self.inspector_preview_container.set_hidden(false);
        self.inspector_preview.set_hidden(true);
        self.inspector_spinner.set_hidden(false);
        self.inspector_spinner.start_animation();
        self.set_preview_layout_expanded(true);
    }

    fn set_inspector_preview_ready(&self) {
        self.inspector_preview_container.set_hidden(false);
        self.inspector_preview.set_hidden(false);
        self.inspector_spinner.set_hidden(true);
        self.inspector_spinner.stop_animation();
        self.set_preview_layout_expanded(true);
    }

    fn set_source_visible(&self, visible: bool) {
        self.source_section.set_hidden(!visible);
        self.source_panel.set_hidden(!visible);
        if let (Some(with_source), Some(without_source)) = (
            self.parameter_section_preview_top.borrow().as_ref(),
            self.parameter_section_full_top.borrow().as_ref(),
        ) {
            with_source.set_active(visible);
            without_source.set_active(!visible);
        }
    }

    fn set_source_values(&self, values: [String; 4]) {
        for (row, value) in self.source_rows.borrow().iter().zip(values) {
            row.set_value(value);
        }
    }

    fn set_parameter_section_visible(&self, visible: bool) {
        self.parameter_section.set_hidden(!visible);
        self.parameter_panel.set_hidden(!visible);
    }

    pub(crate) fn orbit(&self, delta_x: f32, delta_y: f32) {
        self.navigation_gizmo.orbit(delta_x, delta_y);
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.orbit(delta_x, delta_y);
        }
    }

    pub(crate) fn navigation_gizmo_mouse_down(&self, x: f64, y: f64) {
        if let Some((delta_yaw, delta_pitch)) = self.navigation_gizmo.orbit_delta_at(x, y) {
            self.orbit(delta_yaw, delta_pitch);
        }
    }

    pub(crate) fn viewport_clicked(&self, x: f64, y: f64) {
        let (width, height, scale) = self.viewport.objc.get(|view| unsafe {
            let bounds: CGRect = msg_send![view, bounds];
            let window: id = msg_send![view, window];
            let scale: f64 = if window.is_null() {
                1.0
            } else {
                msg_send![window, backingScaleFactor]
            };
            (bounds.size.width, bounds.size.height, scale)
        });
        let x = x.clamp(0.0, width) * scale;
        let y = (height - y).clamp(0.0, height) * scale;
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.pick_viewport(x as f32, y as f32);
        }
    }

    pub(crate) fn clear_selection(&self) {
        self.active_inspector_slot.replace(None);
        self.active_material.replace(None);
        self.parameter_controls.borrow_mut().clear();
        self.set_inspector_preview_visible(false);
        self.set_source_visible(false);
        self.set_parameter_section_visible(false);
        self.inspector_heading
            .set_text(localization::text(Key::Inspector));
        self.inspector_body
            .set_text(localization::text(Key::InspectorBody));
        self.hierarchy.clear_selection();
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.set_selected_material_slot(None);
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
            self.sync_size();
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
        self.titlebar.install(&window);
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
        self.viewport.add_subview(&self.navigation_gizmo.view);
        self.sidebar.add_subview(&self.hierarchy_label);
        self.sidebar.add_subview(self.hierarchy.view());
        self.inspector.add_subview(&self.inspector_label);
        self.inspector
            .add_subview(&self.inspector_preview_container);
        self.inspector.add_subview(&self.inspector_heading);
        self.inspector.add_subview(&self.inspector_body);
        self.inspector.add_subview(&self.source_section);
        self.inspector.add_subview(&self.source_panel);
        self.inspector.add_subview(&self.parameter_section);
        self.inspector.add_subview(&self.parameter_panel);

        let heading_top_with_preview = self
            .inspector_heading
            .top
            .constraint_equal_to(&self.inspector_preview_container.bottom)
            .offset(10.0);
        let heading_top_without_preview = self
            .inspector_heading
            .top
            .constraint_equal_to(&self.inspector_label.bottom)
            .offset(10.0);
        let source_top = self
            .source_section
            .top
            .constraint_equal_to(&self.inspector_body.bottom)
            .offset(16.0);
        let section_top_with_source = self
            .parameter_section
            .top
            .constraint_equal_to(&self.source_panel.bottom)
            .offset(16.0);
        let section_top_without_source = self
            .parameter_section
            .top
            .constraint_equal_to(&self.inspector_body.bottom)
            .offset(16.0);

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
            self.navigation_gizmo
                .view
                .top
                .constraint_equal_to(&self.viewport.top)
                .offset(12.0),
            self.navigation_gizmo
                .view
                .trailing
                .constraint_equal_to(&self.viewport.trailing)
                .offset(-12.0),
            self.navigation_gizmo
                .view
                .width
                .constraint_equal_to_constant(128.0),
            self.navigation_gizmo
                .view
                .height
                .constraint_equal_to_constant(128.0),
            self.hierarchy_label
                .top
                .constraint_equal_to(&self.sidebar.top)
                .offset(14.0),
            self.hierarchy_label
                .leading
                .constraint_equal_to(&self.sidebar.leading)
                .offset(14.0),
            self.hierarchy
                .view()
                .top
                .constraint_equal_to(&self.hierarchy_label.bottom)
                .offset(8.0),
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
            self.inspector_label
                .top
                .constraint_equal_to(&self.inspector.top)
                .offset(14.0),
            self.inspector_label
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.inspector_preview_container
                .top
                .constraint_equal_to(&self.inspector_label.bottom)
                .offset(10.0),
            self.inspector_preview_container
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.inspector_preview_container
                .width
                .constraint_equal_to_constant(160.0),
            self.inspector_preview_container
                .height
                .constraint_equal_to_constant(160.0),
            heading_top_with_preview.clone(),
            heading_top_without_preview.clone(),
            self.inspector_heading
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.inspector_heading
                .trailing
                .constraint_equal_to(&self.inspector.trailing)
                .offset(-18.0),
            self.inspector_body
                .top
                .constraint_equal_to(&self.inspector_heading.bottom)
                .offset(4.0),
            self.inspector_body
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.inspector_body
                .trailing
                .constraint_equal_to(&self.inspector.trailing)
                .offset(-18.0),
            source_top.clone(),
            self.source_section
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.source_panel
                .top
                .constraint_equal_to(&self.source_section.bottom)
                .offset(8.0),
            self.source_panel
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.source_panel
                .trailing
                .constraint_equal_to(&self.inspector.trailing)
                .offset(-18.0),
            self.source_panel.height.constraint_equal_to_constant(120.0),
            section_top_with_source.clone(),
            section_top_without_source.clone(),
            self.parameter_section
                .leading
                .constraint_equal_to(&self.inspector.leading)
                .offset(18.0),
            self.parameter_panel
                .top
                .constraint_equal_to(&self.parameter_section.bottom)
                .offset(8.0),
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
                .constraint_equal_to(&self.inspector.bottom)
                .offset(-18.0),
        ]);
        self.inspector_heading_preview_top
            .replace(Some(heading_top_with_preview));
        self.inspector_heading_full_top
            .replace(Some(heading_top_without_preview));
        self.parameter_section_preview_top
            .replace(Some(section_top_with_source));
        self.parameter_section_full_top
            .replace(Some(section_top_without_source));
        self.set_source_visible(false);
        self.set_inspector_preview_visible(false);

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

fn clear_image(image_view: &ImageView) {
    image_view.objc.with_mut(|image_view| unsafe {
        let _: () = msg_send![image_view, setImage: nil];
    });
}

fn format_parameter_value(value: &ParameterValue) -> String {
    match value {
        ParameterValue::Bool(value) => value.to_string(),
        ParameterValue::I32(value) => value.to_string(),
        ParameterValue::U32(value) => value.to_string(),
        ParameterValue::F32(value) => format!("{value:.3}"),
        ParameterValue::Vec2(values) => format!("[{:.3}, {:.3}]", values[0], values[1]),
        ParameterValue::Vec3(values) => {
            format!("[{:.3}, {:.3}, {:.3}]", values[0], values[1], values[2])
        }
        ParameterValue::Vec4(values) => format!(
            "[{:.3}, {:.3}, {:.3}, {:.3}]",
            values[0], values[1], values[2], values[3]
        ),
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
    fn pmx_load_tracker_rejects_results_invalidated_by_a_new_project() {
        let mut tracker = PmxLoadTracker::default();
        tracker.begin(PathBuf::from("old.pmx"), None);
        tracker.invalidate();

        assert!(tracker.complete(Path::new("old.pmx")).is_none());
    }

    #[test]
    fn pmx_load_tracker_accepts_only_the_latest_requested_load() {
        let mut tracker = PmxLoadTracker::default();
        tracker.begin(PathBuf::from("old.pmx"), None);
        tracker.begin(PathBuf::from("new.pmx"), None);

        assert!(tracker.complete(Path::new("old.pmx")).is_none());
        assert!(tracker.complete(Path::new("new.pmx")).is_some());
    }

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
