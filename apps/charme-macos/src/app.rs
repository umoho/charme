use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use cacao::{
    appkit::{
        App, AppDelegate,
        menu::{Menu, MenuItem},
        window::{TitleVisibility, Window, WindowConfig, WindowDelegate},
    },
    button::{BezelStyle, Button},
    color::Color,
    defaults::{UserDefaults, Value},
    filesystem::FileSelectPanel,
    foundation::{YES, id, nil},
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
use core_graphics::geometry::CGRect;

use crate::{
    bridge::RenderBridge,
    frame_image::make_image,
    interaction::OrbitInputView,
    localization::{self, Key},
    parameter_control::ParameterControl,
    shader_inspection::{self, ParameterControlKind, ShaderInspection},
    slider::BrightnessSlider,
};

pub(crate) enum Message {
    ChooseFile,
    OpenPath(PathBuf),
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
}

impl Default for CharmeApp {
    fn default() -> Self {
        let mut config = WindowConfig::default();
        config.set_initial_dimensions(160.0, 160.0, 720.0, 520.0);
        Self {
            startup: Window::with(config, StartupWindow::new()),
            editor: RefCell::new(None),
        }
    }
}

impl AppDelegate for CharmeApp {
    fn did_finish_launching(&self) {
        App::set_menu(menus());
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
            Message::ChooseFile => self.choose_file(),
            Message::OpenPath(path) => self.open_path(path),
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
                    Message::ChooseFile | Message::OpenPath(_) => unreachable!(),
                }
            }
        }
    }
}

impl CharmeApp {
    fn choose_file(&self) {
        let mut panel = FileSelectPanel::new();
        panel.set_can_choose_files(true);
        panel.set_can_choose_directories(false);
        panel.set_allows_multiple_selection(false);
        panel.set_message(localization::text(Key::ChooseFileMessage));
        panel.show(|urls| {
            if let Some(url) = urls.first() {
                App::<CharmeApp, Message>::dispatch_main(Message::OpenPath(url.pathbuf()));
            }
        });
    }

    fn open_path(&self, path: PathBuf) {
        match file_kind(&path) {
            Some(FileKind::Project) => self.open_project(path),
            Some(FileKind::Pmx) => {
                self.with_editor(|editor| editor.load_pmx(path));
            }
            Some(FileKind::Shader) => {
                self.with_editor(|editor| editor.inspect_shader(path));
            }
            None => self.show_startup_error(&format!("无法识别此文件类型\n\n{}", path.display())),
        }
    }

    fn open_project(&self, path: PathBuf) {
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

    fn ensure_editor(&self) {
        if self.editor.borrow().is_none() {
            let mut config = WindowConfig::default();
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileKind {
    Project,
    Pmx,
    Shader,
}

fn file_kind(path: &Path) -> Option<FileKind> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "charme" => Some(FileKind::Project),
        "pmx" => Some(FileKind::Pmx),
        "wgsl" => Some(FileKind::Shader),
        _ => None,
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
                .take(8)
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
    brand: Label,
    title: Label,
    subtitle: Label,
    open_button: Button,
    formats: Label,
    recent_heading: Label,
    recent_buttons: Vec<Button>,
    status: Label,
}

impl StartupWindow {
    fn new() -> Self {
        let content = panel(Color::rgb(24, 25, 30));
        let brand = label("CHARME", 16.0, true, Color::SystemWhite);
        let title = label(
            localization::text(Key::StartupTitle),
            28.0,
            true,
            Color::SystemWhite,
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
            Color::SystemWhite,
        );
        let status = label("", 11.0, false, Color::SystemRed);
        let mut open_button = Button::new(localization::text(Key::OpenFile));
        open_button.set_bezel_style(BezelStyle::Rounded);
        open_button.set_key_equivalent("o");
        open_button.set_action(|| {
            App::<CharmeApp, Message>::dispatch_main(Message::ChooseFile);
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
                App::<CharmeApp, Message>::dispatch_main(Message::OpenPath(project.clone()));
            });
            recent_buttons.push(button);
        }
        recent_heading.set_hidden(recent_buttons.is_empty());

        Self {
            content,
            brand,
            title,
            subtitle,
            open_button,
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
            &self.brand,
            &self.title,
            &self.subtitle,
            &self.formats,
            &self.recent_heading,
            &self.status,
        ] {
            self.content.add_subview(label);
        }
        self.content.add_subview(&self.open_button);
        for button in &self.recent_buttons {
            self.content.add_subview(button);
        }

        let mut constraints = vec![
            self.brand
                .top
                .constraint_equal_to(&self.content.top)
                .offset(24.0),
            self.brand
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(72.0),
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
            self.formats
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.formats
                .top
                .constraint_equal_to(&self.open_button.bottom)
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

struct EditorWindow {
    content: View,
    sidebar: View,
    viewport: View,
    inspector: View,
    left_divider: View,
    right_divider: View,
    image_view: ImageView,
    orbit_input: OrbitInputView,
    status: Label,
    app_title: Label,
    open_button: Button,
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
        let content = panel(Color::rgb(24, 25, 30));
        let sidebar = panel(Color::rgb(31, 33, 39));
        let viewport = panel(Color::SystemBlack);
        let inspector = panel(Color::rgb(31, 33, 39));
        let left_divider = panel(Color::rgb(52, 54, 62));
        let right_divider = panel(Color::rgb(52, 54, 62));
        let image_view = ImageView::new();
        image_view.set_background_color(Color::SystemBlack);
        let orbit_input = OrbitInputView::new();

        let app_title = label("CHARME", 16.0, true, Color::SystemWhite);
        let mut open_button = Button::new(localization::text(Key::OpenFile));
        open_button.set_bezel_style(BezelStyle::TexturedRounded);
        open_button.set_action(|| {
            App::<CharmeApp, Message>::dispatch_main(Message::ChooseFile);
        });
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
            Color::SystemWhite,
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
            Color::SystemWhite,
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
            Color::SystemWhite,
        );
        inspector_body.set_max_number_of_lines(0);
        let parameter_panel = View::new();
        let brightness_label = label(
            localization::text(Key::Brightness),
            12.0,
            false,
            Color::SystemWhite,
        );
        let brightness = BrightnessSlider::new(0.3);
        let status = label(
            localization::text(Key::RendererStarting),
            11.0,
            false,
            Color::SystemWhite,
        );

        Self {
            content,
            sidebar,
            viewport,
            inspector,
            left_divider,
            right_divider,
            image_view,
            orbit_input,
            status,
            app_title,
            open_button,
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
}

impl WindowDelegate for EditorWindow {
    const NAME: &'static str = "CharmeEditorWindow";

    fn did_load(&mut self, window: Window) {
        window.set_title("Charme");
        window.set_title_visibility(TitleVisibility::Hidden);
        window.set_titlebar_appears_transparent(true);
        window.set_titlebar_separator_style(0);
        window.set_minimum_content_size(900.0, 560.0);
        window.set_content_view(&self.content);

        for view in [
            &self.sidebar,
            &self.left_divider,
            &self.viewport,
            &self.right_divider,
            &self.inspector,
        ] {
            self.content.add_subview(view);
        }
        self.viewport.add_subview(&self.image_view);
        self.viewport.add_subview(&self.orbit_input.view);
        self.viewport.add_subview(&self.status);
        self.sidebar.add_subview(&self.open_button);
        for label in [
            &self.app_title,
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
            self.sidebar.top.constraint_equal_to(&self.content.top),
            self.sidebar
                .bottom
                .constraint_equal_to(&self.content.bottom),
            self.sidebar
                .leading
                .constraint_equal_to(&self.content.leading),
            self.sidebar.width.constraint_equal_to_constant(248.0),
            self.left_divider.top.constraint_equal_to(&self.content.top),
            self.left_divider
                .bottom
                .constraint_equal_to(&self.content.bottom),
            self.left_divider
                .leading
                .constraint_equal_to(&self.sidebar.trailing),
            self.left_divider.width.constraint_equal_to_constant(1.0),
            self.inspector.top.constraint_equal_to(&self.content.top),
            self.inspector
                .bottom
                .constraint_equal_to(&self.content.bottom),
            self.inspector
                .trailing
                .constraint_equal_to(&self.content.trailing),
            self.inspector.width.constraint_equal_to_constant(300.0),
            self.right_divider
                .top
                .constraint_equal_to(&self.content.top),
            self.right_divider
                .bottom
                .constraint_equal_to(&self.content.bottom),
            self.right_divider
                .trailing
                .constraint_equal_to(&self.inspector.leading),
            self.right_divider.width.constraint_equal_to_constant(1.0),
            self.viewport.top.constraint_equal_to(&self.content.top),
            self.viewport
                .bottom
                .constraint_equal_to(&self.content.bottom),
            self.viewport
                .leading
                .constraint_equal_to(&self.left_divider.trailing),
            self.viewport
                .trailing
                .constraint_equal_to(&self.right_divider.leading),
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
            self.app_title
                .top
                .constraint_equal_to(&self.sidebar.top)
                .offset(18.0),
            self.app_title
                .leading
                .constraint_equal_to(&self.sidebar.leading)
                .offset(72.0),
            self.open_button
                .leading
                .constraint_equal_to(&self.app_title.trailing)
                .offset(16.0),
            self.open_button
                .top
                .constraint_equal_to(&self.sidebar.top)
                .offset(16.0),
            self.open_button.height.constraint_equal_to_constant(26.0),
            self.scene_heading
                .top
                .constraint_equal_to(&self.open_button.bottom)
                .offset(24.0),
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

pub(crate) fn run() {
    App::new("com.umoho.charme", CharmeApp::default()).run();
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
            "",
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
                MenuItem::new("打开文件…").key("o").action(|| {
                    App::<CharmeApp, Message>::dispatch_main(Message::ChooseFile);
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
                MenuItem::Copy,
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

fn activate_app() {
    App::activate();
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}
