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
        App, AppDelegate,
        menu::{Menu, MenuItem},
        toolbar::{ItemIdentifier, Toolbar, ToolbarDelegate, ToolbarItem},
        window::{TitleVisibility, Window, WindowConfig, WindowDelegate, WindowToolbarStyle},
    },
    button::{BezelStyle, Button},
    color::{Color, Theme},
    defaults::{UserDefaults, Value},
    filesystem::FileSelectPanel,
    foundation::{BOOL, NO, NSString, YES, id, nil},
    image::{Image, ImageView},
    layout::{Layout, LayoutConstraint},
    notification_center::Dispatcher,
    objc::{class, msg_send, sel, sel_impl},
    text::{Font, Label},
    view::View,
};
use charme_core::{
    EditorCommand, EditorSession, MaterialId, MaterialInstance, ParameterValue, ResourcePath,
    ShaderSource as DocumentShaderSource,
};
use charme_renderer::{Frame, OutputSize, PmxSceneInfo, RendererNotification};
use core_graphics::geometry::{CGPoint, CGRect};

#[cfg(feature = "debug-ui")]
use crate::debug::DebugState;

use crate::{
    bridge::RenderBridge,
    docking::{
        Axis, DockNode, DockTree, DockTreeBuilder, LayoutOptions, NodeId, PanelId, Rect,
        compute_geometry,
    },
    frame_image::make_image,
    interaction::OrbitInputView,
    localization::{self, Key},
    parameter_control::ParameterControl,
    shader_inspection::{self, ParameterControlKind, ShaderInspection},
    slider::BrightnessSlider,
};

pub(crate) enum Message {
    ChooseProject,
    OpenProject(PathBuf),
    NewProject,
    Frame {
        frame: Frame,
        scale: f64,
    },
    Brightness(f32),
    Orbit {
        delta_x: f32,
        delta_y: f32,
    },
    Zoom(f32),
    ChoosePmx,
    LoadPmx(PathBuf),
    ChooseShader,
    InspectShader(PathBuf),
    ShaderInspected {
        path: PathBuf,
        result: Result<ShaderInspection, String>,
    },
    ParameterChanged {
        key: String,
        value: f64,
        kind: ParameterControlKind,
    },
    RendererNotification(RendererNotification),
    Failed(String),
}

pub(crate) struct CharmeApp {
    startup: Window<StartupWindow>,
    editor: RefCell<Option<Window<EditorWindow>>>,
    #[cfg(feature = "debug-ui")]
    debug_state: DebugState,
}

impl Default for CharmeApp {
    fn default() -> Self {
        Self::new()
    }
}

impl CharmeApp {
    fn new() -> Self {
        let mut config = WindowConfig::default();
        config.set_initial_dimensions(160.0, 160.0, 720.0, 520.0);
        Self {
            startup: Window::with(config, StartupWindow::new()),
            editor: RefCell::new(None),
            #[cfg(feature = "debug-ui")]
            debug_state: DebugState::Startup,
        }
    }

    #[cfg(feature = "debug-ui")]
    pub(crate) fn new_debug(debug_state: DebugState) -> Self {
        Self {
            debug_state,
            ..Self::new()
        }
    }
}

impl AppDelegate for CharmeApp {
    fn did_finish_launching(&self) {
        App::set_menu(menus());
        localize_standard_menu_items();
        set_application_menu_name();
        #[cfg(feature = "debug-ui")]
        if !matches!(self.debug_state, DebugState::Startup) {
            self.ensure_editor();
            activate_app();
            return;
        }
        self.startup.show();
        activate_app();
    }

    fn should_terminate_after_last_window_closed(&self) -> bool {
        true
    }
}

impl Dispatcher for CharmeApp {
    type Message = Message;

    fn on_ui_message(&self, message: Self::Message) {
        match message {
            Message::ChooseProject => self.choose_project(),
            Message::OpenProject(path) => self.open_project(path),
            Message::NewProject => self.new_project(),
            Message::ChoosePmx => self.choose_pmx(),
            other => {
                let editor = self.editor.borrow();
                let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref())
                else {
                    return;
                };
                match other {
                    Message::Frame { frame, scale } => window.display(frame, scale),
                    Message::Brightness(value) => window.set_brightness(value),
                    Message::Orbit { delta_x, delta_y } => window.orbit(delta_x, delta_y),
                    Message::Zoom(delta) => window.zoom(delta),
                    Message::LoadPmx(path) => window.load_pmx(path),
                    Message::ChooseShader => window.choose_shader(),
                    Message::InspectShader(path) => window.inspect_shader(path),
                    Message::ShaderInspected { path, result } => {
                        window.show_shader_result(path, result);
                    }
                    Message::ParameterChanged { key, value, kind } => {
                        window.set_parameter_value(&key, value, kind);
                    }
                    Message::RendererNotification(notification) => {
                        window.handle_renderer_notification(notification);
                    }
                    Message::Failed(error) => window.show_error(&error),
                    Message::ChooseProject
                    | Message::OpenProject(_)
                    | Message::NewProject
                    | Message::ChoosePmx => unreachable!(),
                }
            }
        }
    }
}

impl CharmeApp {
    fn choose_project(&self) {
        let mut panel = FileSelectPanel::new();
        panel.set_can_choose_files(true);
        panel.set_can_choose_directories(false);
        panel.set_allows_multiple_selection(false);
        panel.set_message(localization::text(Key::ChooseProjectMessage));
        panel.show(|urls| {
            if let Some(url) = urls.first() {
                App::<CharmeApp, Message>::dispatch_main(Message::OpenProject(url.pathbuf()));
            }
        });
    }

    fn choose_pmx(&self) {
        let mut panel = FileSelectPanel::new();
        panel.set_can_choose_files(true);
        panel.set_can_choose_directories(false);
        panel.set_allows_multiple_selection(false);
        panel.set_message(localization::text(Key::ChoosePmxMessage));
        panel.show(|urls| {
            if let Some(url) = urls.first() {
                App::<CharmeApp, Message>::dispatch_main(Message::LoadPmx(url.pathbuf()));
            }
        });
    }

    fn open_project(&self, path: PathBuf) {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("charme"))
        {
            self.show_startup_error(&format!("请选择.charme项目文件\n\n{}", path.display()));
            return;
        }
        let session = match EditorSession::open(&path) {
            Ok(session) => session,
            Err(error) => {
                self.show_startup_error(&format!("无法打开项目\n\n{error}"));
                return;
            }
        };
        let project_directory = path.parent().unwrap_or_else(|| Path::new("."));
        let character = session
            .document()
            .character()
            .map(|character| character.path.resolve(project_directory));
        self.with_editor(|editor| {
            editor.install_session(session);
            if let Some(character) = character {
                editor.load_pmx(character);
            }
        });
        remember_project(&path);
    }

    fn new_project(&self) {
        self.with_editor(|editor| editor.reset_session());
    }

    fn ensure_editor(&self) {
        if self.editor.borrow().is_none() {
            let mut config = WindowConfig::default();
            config.set_toolbar_style(WindowToolbarStyle::Unified);
            config.set_initial_dimensions(80.0, 80.0, 1280.0, 800.0);
            let window = Window::with(config, EditorWindow::new());
            window.show();
            window
                .delegate
                .as_ref()
                .expect("editor window delegate should exist")
                .start_renderer();
            *self.editor.borrow_mut() = Some(window);
            hide_window(&self.startup);
        } else if let Some(window) = self.editor.borrow().as_ref() {
            window.show();
            hide_window(&self.startup);
        }
    }

    fn with_editor(&self, action: impl FnOnce(&EditorWindow)) {
        self.ensure_editor();
        let editor = self.editor.borrow();
        let window = editor
            .as_ref()
            .and_then(|window| window.delegate.as_deref())
            .expect("editor window should exist");
        action(window);
    }

    fn show_startup_error(&self, error: &str) {
        if let Some(startup) = self.startup.delegate.as_ref() {
            startup.show_error(error);
        }
    }
}

const RECENT_PROJECTS_KEY: &str = "recent-projects";

fn recent_projects() -> Vec<PathBuf> {
    let defaults = UserDefaults::standard();
    defaults
        .get(RECENT_PROJECTS_KEY)
        .and_then(|value| match value {
            Value::String(value) => Some(value),
            _ => None,
        })
        .map(|value| {
            value
                .lines()
                .map(PathBuf::from)
                .filter(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("charme"))
                        && path.is_file()
                })
                .take(5)
                .collect()
        })
        .unwrap_or_default()
}

fn remember_project(path: &Path) {
    let mut projects = recent_projects();
    projects.retain(|candidate| candidate != path);
    projects.insert(0, path.to_path_buf());
    projects.truncate(8);
    let value = projects
        .iter()
        .map(|project| project.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    UserDefaults::standard().insert(RECENT_PROJECTS_KEY, Value::string(value));
}

fn hide_window<T>(window: &Window<T>) {
    unsafe {
        let _: () = msg_send![&*window.objc, orderOut: nil];
    }
}

struct StartupWindow {
    content: View,
    title: Label,
    subtitle: Label,
    open_button: Button,
    new_button: Button,
    formats: Label,
    recent_heading: Label,
    recent_buttons: Vec<Button>,
    status: Label,
}

impl StartupWindow {
    fn new() -> Self {
        let content = panel(Color::MacOSWindowBackgroundColor);
        let title = label(
            localization::text(Key::StartupTitle),
            28.0,
            true,
            Color::Label,
        );
        let subtitle = label(
            localization::text(Key::StartupSubtitle),
            14.0,
            false,
            Color::LabelSecondary,
        );
        let formats = label(
            localization::text(Key::StartupFormats),
            12.0,
            false,
            Color::LabelSecondary,
        );
        let recent_heading = label(
            localization::text(Key::RecentProjects),
            13.0,
            true,
            Color::Label,
        );
        let status = label("", 11.0, false, Color::SystemRed);
        let mut open_button = Button::new(localization::text(Key::OpenProject));
        open_button.set_bezel_style(BezelStyle::Rounded);
        open_button.set_key_equivalent("o");
        open_button.set_action(|| {
            App::<CharmeApp, Message>::dispatch_main(Message::ChooseProject);
        });
        let mut new_button = Button::new(localization::text(Key::NewProject));
        new_button.set_bordered(false);
        new_button.set_text_color(Color::SystemBlue);
        new_button.set_action(|| {
            App::<CharmeApp, Message>::dispatch_main(Message::NewProject);
        });

        let projects = recent_projects();
        let mut recent_buttons = Vec::new();
        for project in projects {
            let name = project
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or(localization::text(Key::ProjectFallback));
            let mut button = Button::new(&format!("{name} · {}", project.display()));
            button.set_bezel_style(BezelStyle::TexturedRounded);
            button.set_action(move || {
                App::<CharmeApp, Message>::dispatch_main(Message::OpenProject(project.clone()));
            });
            recent_buttons.push(button);
        }
        recent_heading.set_hidden(recent_buttons.is_empty());

        Self {
            content,
            title,
            subtitle,
            open_button,
            new_button,
            formats,
            recent_heading,
            recent_buttons,
            status,
        }
    }

    fn show_error(&self, error: &str) {
        self.status.set_text(error);
    }
}

impl WindowDelegate for StartupWindow {
    const NAME: &'static str = "CharmeStartupWindow";

    fn did_load(&mut self, window: Window) {
        window.set_title("Charme");
        window.set_title_visibility(TitleVisibility::Hidden);
        window.set_titlebar_appears_transparent(true);
        window.set_titlebar_separator_style(0);
        window.set_minimum_content_size(560.0, 420.0);
        window.set_content_view(&self.content);

        for label in [
            &self.title,
            &self.subtitle,
            &self.formats,
            &self.recent_heading,
            &self.status,
        ] {
            self.content.add_subview(label);
        }
        self.content.add_subview(&self.open_button);
        self.content.add_subview(&self.new_button);
        for button in &self.recent_buttons {
            self.content.add_subview(button);
        }

        let mut constraints = vec![
            self.title
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.title
                .top
                .constraint_equal_to(&self.content.top)
                .offset(150.0),
            self.subtitle
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.subtitle
                .top
                .constraint_equal_to(&self.title.bottom)
                .offset(12.0),
            self.open_button
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.open_button
                .top
                .constraint_equal_to(&self.subtitle.bottom)
                .offset(28.0),
            self.open_button.width.constraint_equal_to_constant(150.0),
            self.open_button.height.constraint_equal_to_constant(34.0),
            self.new_button
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.new_button
                .top
                .constraint_equal_to(&self.open_button.bottom)
                .offset(8.0),
            self.new_button.width.constraint_equal_to_constant(150.0),
            self.new_button.height.constraint_equal_to_constant(30.0),
            self.formats
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.formats
                .top
                .constraint_equal_to(&self.new_button.bottom)
                .offset(12.0),
            self.recent_heading
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(48.0),
            self.recent_heading
                .top
                .constraint_equal_to(&self.formats.bottom)
                .offset(42.0),
            self.status
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(48.0),
            self.status
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-48.0),
            self.status
                .bottom
                .constraint_equal_to(&self.content.bottom)
                .offset(-20.0),
        ];
        for (index, button) in self.recent_buttons.iter().enumerate() {
            constraints.extend([
                button
                    .leading
                    .constraint_equal_to(&self.content.leading)
                    .offset(48.0),
                button
                    .trailing
                    .constraint_equal_to(&self.content.trailing)
                    .offset(-48.0),
                button
                    .top
                    .constraint_equal_to(&self.recent_heading.bottom)
                    .offset(10.0 + index as f64 * 34.0),
                button.height.constraint_equal_to_constant(28.0),
            ]);
        }
        LayoutConstraint::activate(&constraints);
    }
}

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
    fn new(node: NodeId, axis: Axis) -> Self {
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

struct EditorWindow {
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
    scene_heading: Label,
    scene_info: Label,
    materials_heading: Label,
    material_list: Label,
    inspector_heading: Label,
    inspector_body: Label,
    parameter_panel: View,
    parameter_controls: RefCell<Vec<ParameterControl>>,
    session: RefCell<EditorSession>,
    active_material: RefCell<Option<MaterialId>>,
    brightness_label: Label,
    brightness: BrightnessSlider,
    current_image: RefCell<Option<Image>>,
    bridge: RefCell<Option<RenderBridge>>,
}

impl EditorWindow {
    fn new() -> Self {
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

        let scene_heading = label(
            localization::text(Key::Scene),
            11.0,
            true,
            Color::LabelSecondary,
        );
        let scene_info = label(
            localization::text(Key::EmptyScene),
            13.0,
            false,
            Color::Label,
        );
        scene_info.set_max_number_of_lines(0);
        let materials_heading = label(
            localization::text(Key::Materials),
            11.0,
            true,
            Color::LabelSecondary,
        );
        let material_list = label(
            localization::text(Key::EmptyMaterials),
            12.0,
            false,
            Color::Label,
        );
        material_list.set_max_number_of_lines(0);

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
            scene_heading,
            scene_info,
            materials_heading,
            material_list,
            inspector_heading,
            inspector_body,
            parameter_panel,
            parameter_controls: RefCell::new(Vec::new()),
            session: RefCell::new(EditorSession::new("未命名角色")),
            active_material: RefCell::new(None),
            brightness_label,
            brightness,
            current_image: RefCell::new(None),
            bridge: RefCell::new(None),
        }
    }

    fn install_session(&self, session: EditorSession) {
        self.session.replace(session);
        self.active_material.replace(None);
        self.parameter_controls.borrow_mut().clear();
        self.scene_info
            .set_text(localization::text(Key::ProjectOpened));
        self.material_list
            .set_text(localization::text(Key::WaitingCharacter));
        self.inspector_heading
            .set_text(localization::text(Key::Inspector));
        self.inspector_body
            .set_text(localization::text(Key::InspectorBody));
    }

    fn reset_session(&self) {
        self.session.replace(EditorSession::new("未命名项目"));
        self.active_material.replace(None);
        self.parameter_controls.borrow_mut().clear();
        self.scene_info
            .set_text(localization::text(Key::EmptyScene));
        self.material_list
            .set_text(localization::text(Key::EmptyMaterials));
        self.inspector_heading
            .set_text(localization::text(Key::Inspector));
        self.inspector_body
            .set_text(localization::text(Key::InspectorBody));
    }

    fn start_renderer(&self) {
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

    fn display(&self, frame: Frame, scale: f64) {
        let sequence = frame.sequence();
        let width = frame.width();
        let height = frame.height();
        match make_image(frame, scale) {
            Ok(image) => {
                self.image_view.set_image(&image);
                *self.current_image.borrow_mut() = Some(image);
                self.status.set_text(format!(
                    "第{sequence}帧 · {width}×{height}px · 拖动旋转 · 滚动缩放"
                ));
            }
            Err(error) => self.show_error(&error),
        }
    }

    fn load_pmx(&self, path: PathBuf) {
        self.scene_info
            .set_text(format!("加载中…\n{}", path.display()));
        self.material_list
            .set_text(localization::text(Key::LoadingMaterials));
        self.status
            .set_text(localization::text(Key::LoadingPmxTextures));
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.load_pmx(path);
        }
    }

    fn choose_shader(&self) {
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

    fn inspect_shader(&self, path: PathBuf) {
        self.inspector_heading
            .set_text(localization::text(Key::InspectingShader));
        self.inspector_body.set_text(path.display().to_string());
        self.parameter_controls.borrow_mut().clear();
        shader_inspection::inspect_shader(path);
    }

    fn show_shader_result(&self, path: PathBuf, result: Result<ShaderInspection, String>) {
        let inspection = match result {
            Ok(inspection) => inspection,
            Err(error) => {
                self.inspector_heading
                    .set_text(localization::text(Key::ShaderError));
                self.inspector_body
                    .set_text(format!("{}\n\n{error}", path.display()));
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
        self.inspector_body.set_text(format!(
            "{file_name}\n{}个参数块 · {}个诊断{}",
            inspection.parameter_block_count,
            inspection.diagnostics.len(),
            if inspection.non_scalar_field_count == 0 {
                String::new()
            } else {
                format!(" · {}个非标量字段", inspection.non_scalar_field_count)
            }
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
        self.status
            .set_text(format!("已反射{file_name} · {}个标量控件", controls.len()));
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

    fn set_parameter_value(&self, key: &str, value: f64, kind: ParameterControlKind) {
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
        self.status.set_text(if updated.is_some() {
            format!("{key} = {value:.3} · 文档已修改 · 预览已更新")
        } else {
            format!("{key} = {value:.3} · 等待预览绑定")
        });
    }

    fn handle_renderer_notification(&self, notification: RendererNotification) {
        match notification {
            RendererNotification::PmxLoaded(info) => self.show_scene_info(&info),
            RendererNotification::PmxLoadFailed { path, message } => {
                self.scene_info
                    .set_text(format!("无法加载\n{}", path.display()));
                self.show_error(&message);
            }
            RendererNotification::MaterialParameterRejected { path, message } => {
                self.show_error(&format!("参数{path}被拒绝：{message}"));
            }
            _ => {}
        }
    }

    fn show_scene_info(&self, info: &PmxSceneInfo) {
        self.scene_info.set_text(format!(
            "{}\n\n{}个顶点 · {}个索引",
            info.name(),
            info.vertex_count(),
            info.index_count()
        ));
        let slots = info
            .material_slots()
            .iter()
            .take(24)
            .map(|slot| format!("{:02}  {}", slot.index(), slot.name()))
            .collect::<Vec<_>>();
        let remaining = info.material_slots().len().saturating_sub(slots.len());
        let mut text = if slots.is_empty() {
            localization::text(Key::NoMaterials).to_owned()
        } else {
            slots.join("\n")
        };
        if remaining > 0 {
            text.push_str(&format!("\n…以及{remaining}个"));
        }
        self.material_list.set_text(text);

        if let Some(slot) = info.material_slots().first() {
            self.inspector_body.set_text(format!(
                "{}\n\n源材质槽{}\n漫反射：{}\nSphere：{}\nToon：{}\n\nCharme材质控件即将支持。",
                slot.name(),
                slot.index(),
                slot.diffuse_texture().unwrap_or("—"),
                slot.sphere_texture().unwrap_or("—"),
                slot.toon_texture().unwrap_or("—"),
            ));
        }
        self.status.set_text(if info.warnings().is_empty() {
            format!(
                "已加载{} · {}个材质槽",
                info.name(),
                info.material_slots().len()
            )
        } else {
            format!("已加载{} · {}个警告", info.name(), info.warnings().len())
        });
    }

    fn set_brightness(&self, value: f32) {
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.set_brightness(value);
        }
    }

    fn orbit(&self, delta_x: f32, delta_y: f32) {
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.orbit(delta_x, delta_y);
        }
    }

    fn zoom(&self, delta: f32) {
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.zoom(delta);
        }
    }

    fn show_error(&self, error: &str) {
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

    fn did_load(&mut self, window: Window) {
        window.set_title("Charme");
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
        for label in [
            &self.scene_heading,
            &self.scene_info,
            &self.materials_heading,
            &self.material_list,
        ] {
            self.sidebar.add_subview(label);
        }
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
            self.scene_heading
                .top
                .constraint_equal_to(&self.sidebar.top)
                .offset(22.0),
            self.scene_heading
                .leading
                .constraint_equal_to(&self.sidebar.leading)
                .offset(16.0),
            self.scene_info
                .top
                .constraint_equal_to(&self.scene_heading.bottom)
                .offset(10.0),
            self.scene_info
                .leading
                .constraint_equal_to(&self.sidebar.leading)
                .offset(16.0),
            self.scene_info
                .trailing
                .constraint_equal_to(&self.sidebar.trailing)
                .offset(-16.0),
            self.materials_heading
                .top
                .constraint_equal_to(&self.scene_info.bottom)
                .offset(26.0),
            self.materials_heading
                .leading
                .constraint_equal_to(&self.sidebar.leading)
                .offset(16.0),
            self.material_list
                .top
                .constraint_equal_to(&self.materials_heading.bottom)
                .offset(10.0),
            self.material_list
                .leading
                .constraint_equal_to(&self.sidebar.leading)
                .offset(16.0),
            self.material_list
                .trailing
                .constraint_equal_to(&self.sidebar.trailing)
                .offset(-16.0),
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

#[cfg(not(feature = "debug-ui"))]
pub(crate) fn run() {
    App::new("com.umoho.charme", CharmeApp::default()).run();
}

#[cfg(feature = "debug-ui")]
pub(crate) fn run_with_debug_state(state: DebugState) {
    App::new("com.umoho.charme", CharmeApp::new_debug(state)).run();
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

fn panel(color: Color) -> View {
    let view = View::new();
    view.set_background_color(color);
    view
}

fn label(text: &str, size: f64, bold: bool, color: Color) -> Label {
    let label = Label::new();
    label.set_text(text);
    label.set_font(if bold {
        Font::bold_system(size)
    } else {
        Font::system(size)
    });
    label.set_text_color(color);
    label
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new(
            "Charme",
            vec![
                MenuItem::About("Charme".to_owned()),
                MenuItem::Separator,
                MenuItem::Services,
                MenuItem::Separator,
                MenuItem::Hide,
                MenuItem::HideOthers,
                MenuItem::ShowAll,
                MenuItem::Separator,
                MenuItem::Quit,
            ],
        ),
        Menu::new(
            localization::text(Key::FileMenu),
            vec![
                MenuItem::new(localization::text(Key::NewProjectMenu)).action(|| {
                    App::<CharmeApp, Message>::dispatch_main(Message::NewProject);
                }),
                MenuItem::new(localization::text(Key::OpenProjectMenu))
                    .key("o")
                    .action(|| {
                        App::<CharmeApp, Message>::dispatch_main(Message::ChooseProject);
                    }),
                MenuItem::new(localization::text(Key::ImportMenu)),
                // This item is moved below into the Import submenu after AppKit
                // has installed the menu hierarchy.
                MenuItem::new(localization::text(Key::ImportPmxMenu)).action(|| {
                    App::<CharmeApp, Message>::dispatch_main(Message::ChoosePmx);
                }),
                MenuItem::new(localization::text(Key::InspectShaderMenu)).action(|| {
                    App::<CharmeApp, Message>::dispatch_main(Message::ChooseShader);
                }),
                MenuItem::Separator,
                MenuItem::CloseWindow,
            ],
        ),
        Menu::new(
            localization::text(Key::EditMenu),
            vec![
                MenuItem::Undo,
                MenuItem::Redo,
                MenuItem::Separator,
                MenuItem::Cut,
                MenuItem::Copy,
                MenuItem::Paste,
                MenuItem::Separator,
                MenuItem::SelectAll,
            ],
        ),
        Menu::new(
            localization::text(Key::ViewMenu),
            vec![MenuItem::EnterFullScreen],
        ),
        Menu::new(
            localization::text(Key::WindowMenu),
            vec![MenuItem::Minimize, MenuItem::Zoom],
        ),
    ]
}

fn localize_standard_menu_items() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let main_menu: id = msg_send![app, mainMenu];
        if main_menu.is_null() {
            return;
        }

        let app_menu_item: id = msg_send![main_menu, itemAtIndex: 0];
        let app_menu: id = msg_send![app_menu_item, submenu];
        set_menu_item_title(app_menu, 0, Key::About);
        set_menu_item_title(app_menu, 2, Key::Services);
        set_menu_item_title(app_menu, 4, Key::HideApp);
        set_menu_item_title(app_menu, 5, Key::HideOthers);
        set_menu_item_title(app_menu, 6, Key::ShowAll);
        set_menu_item_title(app_menu, 8, Key::Quit);

        let file_menu_item: id = msg_send![main_menu, itemAtIndex: 1];
        let file_menu: id = msg_send![file_menu_item, submenu];
        install_import_submenu(file_menu);
        set_menu_item_title(file_menu, 5, Key::CloseWindow);

        let edit_menu_item: id = msg_send![main_menu, itemAtIndex: 2];
        let edit_menu: id = msg_send![edit_menu_item, submenu];
        set_menu_item_title(edit_menu, 0, Key::Undo);
        set_menu_item_title(edit_menu, 1, Key::Redo);
        set_menu_item_title(edit_menu, 3, Key::Cut);
        set_menu_item_title(edit_menu, 4, Key::Copy);
        set_menu_item_title(edit_menu, 5, Key::Paste);
        set_menu_item_title(edit_menu, 7, Key::SelectAll);

        let view_menu_item: id = msg_send![main_menu, itemAtIndex: 3];
        let view_menu: id = msg_send![view_menu_item, submenu];
        set_menu_item_title(view_menu, 0, Key::EnterFullScreen);

        let window_menu_item: id = msg_send![main_menu, itemAtIndex: 4];
        let window_menu: id = msg_send![window_menu_item, submenu];
        set_menu_item_title(window_menu, 0, Key::Minimize);
        set_menu_item_title(window_menu, 1, Key::Zoom);
    }
}

fn set_menu_item_title(menu: id, index: usize, key: Key) {
    if menu.is_null() {
        return;
    }
    unsafe {
        let item: id = msg_send![menu, itemAtIndex: index];
        if item.is_null() {
            return;
        }
        let title = NSString::new(localization::text(key));
        let _: () = msg_send![item, setTitle: &*title];
    }
}

fn install_import_submenu(file_menu: id) {
    if file_menu.is_null() {
        return;
    }
    unsafe {
        // Cacao 0.3 does not expose submenu construction. The PMX item is
        // created normally so its Rust callback remains owned by Cacao, then
        // moved under the localized Import item at runtime.
        let parent: id = msg_send![file_menu, itemAtIndex: 2];
        let child: id = msg_send![file_menu, itemAtIndex: 3];
        if parent.is_null() || child.is_null() {
            return;
        }

        let menu_class = class!(NSMenu);
        let allocated: id = msg_send![menu_class, alloc];
        let title = NSString::new(localization::text(Key::ImportMenu));
        let submenu: id = msg_send![allocated, initWithTitle: &*title];
        if submenu.is_null() {
            return;
        }
        // Keep the Cacao callback alive while moving the item between menus.
        let _: () = msg_send![child, retain];
        let _: () = msg_send![file_menu, removeItemAtIndex: 3usize];
        let _: () = msg_send![submenu, addItem: child];
        let _: () = msg_send![parent, setSubmenu: submenu];
        let _: () = msg_send![child, release];
        let _: () = msg_send![submenu, release];
    }
}

fn set_application_menu_name() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let main_menu: id = msg_send![app, mainMenu];
        let app_menu_item: id = msg_send![main_menu, itemAtIndex: 0];
        let title = NSString::new("Charme");
        let _: () = msg_send![app_menu_item, setTitle: &*title];
    }
}

fn activate_app() {
    App::activate();
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}
