mod menu;

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
};

use cacao::{
    appkit::{
        Alert, App, AppDelegate,
        window::{Window, WindowConfig, WindowToolbarStyle},
    },
    defaults::{UserDefaults, Value},
    filesystem::{FileSavePanel, FileSelectPanel},
    foundation::nil,
    notification_center::Dispatcher,
    objc::{msg_send, sel, sel_impl},
};
use charme_application::{ApplicationEvent, EditorAction, EditorController};
use charme_core::ParameterValue;
use url::Url;

#[cfg(feature = "debug-ui")]
use crate::debug::DebugState;

use crate::{
    editor::{EditorWindow, HierarchyItemId},
    localization::{self, Key},
    startup::StartupWindow,
};

use menu::{
    activate_app, ensure_charme_extension, install_native_menus, refresh_recent_projects_menu,
    set_application_menu_name, update_menu_state,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MenuContext {
    Startup,
    Editor,
}

pub(crate) enum Message {
    ChooseProject,
    OpenProject(PathBuf),
    NewProject,
    SaveProject,
    ChooseSaveProject,
    SaveProjectAs(PathBuf),
    Undo,
    Redo,
    MenuContextChanged(MenuContext),
    Orbit { delta_x: f32, delta_y: f32 },
    Zoom(f32),
    ChoosePmx,
    LoadPmx(PathBuf),
    ChooseShader,
    InspectShader(PathBuf),
    ParameterChanged { key: String, value: ParameterValue },
    HierarchySelectionChanged(HierarchyItemId),
    Application(ApplicationEvent),
}

pub(crate) struct CharmeApp {
    startup: Window<StartupWindow>,
    editor: RefCell<Option<Window<EditorWindow>>>,
    menu_context: Cell<MenuContext>,
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
            menu_context: Cell::new(MenuContext::Startup),
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
        install_native_menus();
        set_application_menu_name();
        update_menu_state(MenuContext::Startup, false, false, false);
        #[cfg(feature = "debug-ui")]
        if !matches!(self.debug_state, DebugState::Startup) {
            self.ensure_editor();
            activate_app();
            return;
        }
        self.startup.show();
        activate_app();
    }

    fn open_urls(&self, urls: Vec<Url>) {
        for path in urls.into_iter().filter_map(|url| url.to_file_path().ok()) {
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("charme"))
            {
                App::<CharmeApp, Message>::dispatch_main(Message::OpenProject(path));
            }
        }
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
            Message::SaveProject => self.save_project(),
            Message::ChooseSaveProject => self.choose_save_project(),
            Message::SaveProjectAs(path) => self.save_project_as(path),
            Message::Undo => self.undo(),
            Message::Redo => self.redo(),
            Message::MenuContextChanged(context) => {
                self.menu_context.set(context);
                self.refresh_menus();
            }
            Message::ChoosePmx => self.choose_pmx(),
            Message::Application(ApplicationEvent::EditorUpdated(update)) => {
                update_menu_state(
                    self.menu_context.get(),
                    update.view_model.dirty,
                    update.view_model.can_undo,
                    update.view_model.can_redo,
                );
            }
            other => {
                let editor = self.editor.borrow();
                let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref())
                else {
                    return;
                };
                match other {
                    Message::Application(event) => match event {
                        ApplicationEvent::FrameReady { frame, scale } => {
                            window.display(frame, scale)
                        }
                        ApplicationEvent::ShaderInspected { path, result } => {
                            window.show_shader_result(path, result);
                        }
                        ApplicationEvent::Renderer(notification) => {
                            window.handle_renderer_notification(notification);
                        }
                        ApplicationEvent::Failed(error) => window.show_error(&error),
                        _ => {}
                    },
                    Message::Orbit { delta_x, delta_y } => window.orbit(delta_x, delta_y),
                    Message::Zoom(delta) => window.zoom(delta),
                    Message::LoadPmx(path) => window.import_pmx(path),
                    Message::ChooseShader => window.choose_shader(),
                    Message::InspectShader(path) => window.inspect_shader(path),
                    Message::ParameterChanged { key, value } => {
                        window.set_parameter_value(&key, value);
                    }
                    Message::HierarchySelectionChanged(item) => {
                        window.select_hierarchy_item(item);
                    }
                    Message::ChooseProject
                    | Message::OpenProject(_)
                    | Message::NewProject
                    | Message::SaveProject
                    | Message::ChooseSaveProject
                    | Message::SaveProjectAs(_)
                    | Message::Undo
                    | Message::Redo
                    | Message::MenuContextChanged(_)
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

    fn save_project(&self) {
        let editor_windows = self.editor.borrow();
        let Some(editor) = editor_windows
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
        else {
            return;
        };
        if editor.controller.borrow().project_path().is_none() {
            drop(editor_windows);
            self.choose_save_project();
            return;
        }
        if let Err(error) = editor.save_project() {
            eprintln!("Failed to save project: {error}");
            editor.show_error(localization::text(Key::SaveProjectFailed));
        }
        drop(editor_windows);
        self.refresh_menus();
    }

    fn choose_save_project(&self) {
        let suggested = {
            let editor_windows = self.editor.borrow();
            let Some(editor) = editor_windows
                .as_ref()
                .and_then(|window| window.delegate.as_ref())
            else {
                return;
            };
            format!(
                "{}.charme",
                editor.controller.borrow().view_model().document_name
            )
        };
        let mut panel = FileSavePanel::new();
        panel.set_suggested_filename(&suggested);
        panel.set_message(localization::text(Key::SaveProjectMessage));
        panel.show(|path| {
            if let Some(path) = path {
                App::<CharmeApp, Message>::dispatch_main(Message::SaveProjectAs(PathBuf::from(
                    ensure_charme_extension(path),
                )));
            }
        });
    }

    fn undo(&self) {
        let editor_windows = self.editor.borrow();
        if let Some(editor) = editor_windows
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
        {
            let _ = editor.dispatch_action(EditorAction::Undo);
            self.refresh_menus();
        }
    }

    fn redo(&self) {
        let editor_windows = self.editor.borrow();
        if let Some(editor) = editor_windows
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
        {
            let _ = editor.dispatch_action(EditorAction::Redo);
            self.refresh_menus();
        }
    }

    fn save_project_as(&self, path: PathBuf) {
        let editor_windows = self.editor.borrow();
        let Some(editor) = editor_windows
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
        else {
            return;
        };
        if let Err(error) = editor.save_project_as(path) {
            eprintln!("Failed to save project: {error}");
            editor.show_error(localization::text(Key::SaveProjectFailed));
        }
        drop(editor_windows);
        self.refresh_menus();
    }

    fn refresh_menus(&self) {
        let editor = self.editor.borrow();
        let (dirty, can_undo, can_redo) = editor
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
            .map(|window| {
                let view_model = window.controller.borrow().view_model();
                (view_model.dirty, view_model.can_undo, view_model.can_redo)
            })
            .unwrap_or((false, false, false));
        update_menu_state(self.menu_context.get(), dirty, can_undo, can_redo);
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
            self.show_startup_error(&localization::format(
                Key::InvalidProjectFile,
                &[("path", &path.display())],
            ));
            return;
        }
        let controller = match EditorController::open(&path) {
            Ok(controller) => controller,
            Err(error) => {
                eprintln!("Failed to open project {}: {error}", path.display());
                self.show_startup_error(localization::text(Key::OpenProjectFailed));
                return;
            }
        };
        let project_directory = path.parent().unwrap_or_else(|| Path::new("."));
        let character = controller
            .document()
            .character()
            .map(|character| character.path.resolve(project_directory));
        self.with_editor(|editor| {
            editor.install_controller(controller);
            if let Some(character) = character {
                editor.load_pmx(character);
            }
        });
        remember_project(&path);
        refresh_recent_projects_menu();
        self.refresh_menus();
    }

    fn new_project(&self) {
        self.with_editor(|editor| editor.reset_controller());
    }

    fn ensure_editor(&self) {
        if self.editor.borrow().is_none() {
            let mut config = WindowConfig::default();
            config.set_toolbar_style(WindowToolbarStyle::UnifiedCompact);
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
        Alert::new(localization::text(Key::AppName), error).show();
    }
}

const RECENT_PROJECTS_KEY: &str = "recent-projects";

pub(crate) fn recent_projects() -> Vec<PathBuf> {
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

pub(crate) fn dispatch(message: Message) {
    App::<CharmeApp, Message>::dispatch_main(message);
}

fn hide_window<T>(window: &Window<T>) {
    unsafe {
        let _: () = msg_send![&*window.objc, orderOut: nil];
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
