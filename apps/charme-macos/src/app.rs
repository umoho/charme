mod menu;

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
};

use cacao::{
    appkit::{
        Alert, App, AppDelegate, TerminateResponse,
        window::{Window, WindowConfig, WindowToolbarStyle},
    },
    defaults::{UserDefaults, Value},
    filesystem::{FileSavePanel, FileSelectPanel},
    foundation::{NO, NSArray, NSString, id, nil},
    notification_center::Dispatcher,
    objc::{msg_send, sel, sel_impl},
};
use charme_application::{ApplicationEvent, EditorAction, EditorController, ViewportToolId};
use charme_core::ParameterValue;
use charme_renderer::{
    Frame, PmxLoadProgress, PmxSourceIdentity, RendererNotification, ViewportSelectionAction,
};
use url::Url;

#[cfg(feature = "debug-ui")]
use crate::debug::DebugState;

use crate::{
    editor::{EditorWindow, HierarchyItemId},
    loading::{PmxLoadingSheet, display_pmx_source},
    localization::{self, Key},
    startup::StartupWindow,
};

use menu::{
    activate_app, install_native_menus, refresh_recent_projects_menu, set_application_menu_name,
    update_menu_state,
};

pub(crate) use menu::{ensure_charme_extension, menu_target};

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
    ConfirmQuit,
    Undo,
    Redo,
    SelectAll,
    DeselectAll,
    InvertSelection,
    SplitSelectedPrimitives,
    MenuContextChanged(MenuContext),
    ToolChanged(ViewportToolId),
    ChoosePmx,
    PmxLoadStarted {
        request_id: u64,
        source: PmxSourceIdentity,
    },
    PmxLoadProgress {
        progress: PmxLoadProgress,
    },
    PmxLoadFinished {
        request_id: Option<u64>,
    },
    PmxLoadFailed {
        request_id: Option<u64>,
        source: PmxSourceIdentity,
        message: String,
    },
    Editor(EditorMessage),
    Application(ApplicationEvent),
    Preview(PreviewEvent),
}

pub(crate) enum EditorMessage {
    Orbit {
        delta_x: f32,
        delta_y: f32,
    },
    NavigationGizmoMouseDown {
        x: f64,
        y: f64,
    },
    ViewportClicked {
        x: f64,
        y: f64,
        selection_action: ViewportSelectionAction,
    },
    /// Tab pressed in the viewport: cycle the active tool.
    CycleViewportTool,
    /// Escape pressed in the viewport: return to the primary tool.
    ResetViewportTool,
    Zoom(f32),
    LoadPmx(PathBuf),
    ChooseShader,
    InspectShader(PathBuf),
    ParameterChanged {
        key: String,
        value: ParameterValue,
    },
    HierarchySelectionChanged(Vec<HierarchyItemId>),
}

pub(crate) enum PreviewEvent {
    FrameReady { frame: Frame, scale: f64 },
    Renderer(RendererNotification),
    Failed(String),
}

struct ActivePmxLoadingSheet {
    request_id: u64,
    window: Window<PmxLoadingSheet>,
}

pub(crate) struct CharmeApp {
    startup: Window<StartupWindow>,
    editor: RefCell<Option<Window<EditorWindow>>>,
    pmx_loading: RefCell<Option<ActivePmxLoadingSheet>>,
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
            pmx_loading: RefCell::new(None),
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
        update_menu_state(
            MenuContext::Startup,
            false,
            false,
            false,
            ViewportToolId::SelectMaterialSlot,
            false,
            false,
        );
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

    fn should_terminate(&self) -> TerminateResponse {
        let (dirty, confirmed) = {
            let editor = self.editor.borrow();
            let window = editor.as_ref().and_then(|window| window.delegate.as_ref());
            (
                window.is_some_and(|window| window.controller.borrow().view_model().dirty),
                window.is_some_and(|window| window.has_discard_confirmed()),
            )
        };
        if dirty && !confirmed {
            App::<CharmeApp, Message>::dispatch_main(Message::ConfirmQuit);
            TerminateResponse::Later
        } else {
            TerminateResponse::Now
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
            Message::ConfirmQuit => self.confirm_quit(),
            Message::Undo => self.undo(),
            Message::Redo => self.redo(),
            Message::SelectAll => self.select_all_primitives(),
            Message::DeselectAll => self.deselect_all(),
            Message::InvertSelection => self.invert_selection(),
            Message::SplitSelectedPrimitives => self.split_selected_primitives(),
            Message::MenuContextChanged(context) => {
                self.menu_context.set(context);
                self.refresh_menus();
            }
            Message::ToolChanged(tool) => self.set_tool(tool),
            Message::ChoosePmx => self.choose_pmx(),
            Message::PmxLoadStarted { request_id, source } => {
                self.show_pmx_loading(request_id, source)
            }
            Message::PmxLoadProgress { progress } => self.update_pmx_loading(progress),
            Message::PmxLoadFinished { request_id } => self.finish_pmx_loading(request_id),
            Message::PmxLoadFailed {
                request_id,
                source,
                message,
            } => self.show_pmx_load_error(request_id, source, message),
            Message::Application(ApplicationEvent::EditorUpdated(_)) => {
                self.refresh_menus();
            }
            Message::Application(event) => {
                let editor = self.editor.borrow();
                let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref())
                else {
                    return;
                };
                match event {
                    ApplicationEvent::ShaderInspected { path, result } => {
                        window.show_shader_result(path, result);
                    }
                    ApplicationEvent::Failed(error) => window.show_error(&error),
                    ApplicationEvent::EditorUpdated(_) => unreachable!(
                        "editor updates are handled before resolving the editor window"
                    ),
                    _ => {}
                }
            }
            Message::Preview(event) => {
                let editor = self.editor.borrow();
                let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref())
                else {
                    return;
                };
                match event {
                    PreviewEvent::FrameReady { frame, scale } => window.display(frame, scale),
                    PreviewEvent::Renderer(notification) => {
                        window.handle_renderer_notification(notification);
                        drop(editor);
                        self.refresh_menus();
                    }
                    PreviewEvent::Failed(error) => {
                        App::<CharmeApp, Message>::dispatch_main(Message::PmxLoadFinished {
                            request_id: None,
                        });
                        window.show_error(&error);
                    }
                }
            }
            Message::Editor(message) => {
                let editor = self.editor.borrow();
                let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref())
                else {
                    return;
                };
                match message {
                    EditorMessage::Orbit { delta_x, delta_y } => window.orbit(delta_x, delta_y),
                    EditorMessage::NavigationGizmoMouseDown { x, y } => {
                        window.navigation_gizmo_mouse_down(x, y);
                    }
                    EditorMessage::ViewportClicked {
                        x,
                        y,
                        selection_action,
                    } => window.viewport_clicked(x, y, selection_action),
                    EditorMessage::CycleViewportTool => {
                        window.cycle_tool();
                        drop(editor);
                        self.refresh_menus();
                    }
                    EditorMessage::ResetViewportTool => {
                        window.reset_tool();
                        drop(editor);
                        self.refresh_menus();
                    }
                    EditorMessage::Zoom(delta) => window.zoom(delta),
                    EditorMessage::LoadPmx(path) => window.import_pmx(path),
                    EditorMessage::ChooseShader => window.choose_shader(),
                    EditorMessage::InspectShader(path) => window.inspect_shader(path),
                    EditorMessage::ParameterChanged { key, value } => {
                        window.set_parameter_value(&key, value);
                    }
                    EditorMessage::HierarchySelectionChanged(items) => {
                        window.handle_hierarchy_selection_changed(items);
                        drop(editor);
                        self.refresh_menus();
                    }
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
            tracing::error!(error = %error, "Failed to save project");
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
            tracing::error!(error = %error, "Failed to save project");
            editor.show_error(localization::text(Key::SaveProjectFailed));
        }
        drop(editor_windows);
        self.refresh_menus();
    }

    fn refresh_menus(&self) {
        let editor = self.editor.borrow();
        let (dirty, can_undo, can_redo, has_scene, has_primitive_selection) = editor
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
            .map(|window| {
                let view_model = window.controller.borrow().view_model();
                (
                    view_model.dirty,
                    view_model.can_undo,
                    view_model.can_redo,
                    window.has_loaded_scene(),
                    window.has_primitive_selection(),
                )
            })
            .unwrap_or((false, false, false, false, false));
        let tool = editor
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
            .map(|window| window.tool())
            .unwrap_or(ViewportToolId::SelectMaterialSlot);
        update_menu_state(
            self.menu_context.get(),
            dirty,
            can_undo,
            can_redo,
            tool,
            has_scene,
            has_primitive_selection,
        );
    }

    fn select_all_primitives(&self) {
        let editor = self.editor.borrow();
        if let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref()) {
            window.select_all_primitives();
        }
        drop(editor);
        self.refresh_menus();
    }

    fn deselect_all(&self) {
        let editor = self.editor.borrow();
        if let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref()) {
            window.deselect_all_selection();
        }
        drop(editor);
        self.refresh_menus();
    }

    fn invert_selection(&self) {
        let editor = self.editor.borrow();
        if let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref()) {
            window.invert_primitive_selection();
        }
        drop(editor);
        self.refresh_menus();
    }

    fn split_selected_primitives(&self) {
        let editor = self.editor.borrow();
        if let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref()) {
            window.split_selected_primitives_by_connectivity();
        }
        drop(editor);
        self.refresh_menus();
    }

    fn set_tool(&self, tool: ViewportToolId) {
        let editor = self.editor.borrow();
        if let Some(window) = editor.as_ref().and_then(|window| window.delegate.as_ref()) {
            window.set_tool(tool);
        }
        drop(editor);
        self.refresh_menus();
    }

    fn choose_pmx(&self) {
        let mut panel = FileSelectPanel::new();
        panel.set_can_choose_files(true);
        panel.set_can_choose_directories(false);
        panel.set_allows_multiple_selection(false);
        panel.set_message(localization::text(Key::ChoosePmxMessage));
        set_model_file_types(&panel);
        panel.show(|urls| {
            if let Some(url) = urls.first() {
                App::<CharmeApp, Message>::dispatch_main(Message::Editor(EditorMessage::LoadPmx(
                    url.pathbuf(),
                )));
            }
        });
    }

    fn open_project(&self, path: PathBuf) {
        if !self.confirm_replace_editor_document() {
            return;
        }
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
                tracing::error!(
                    path = %path.display(),
                    error = %error,
                    "Failed to open project"
                );
                self.show_startup_error(localization::text(Key::OpenProjectFailed));
                return;
            }
        };
        let project_directory = path.parent().unwrap_or_else(|| Path::new("."));
        let character = controller.document().character().cloned();
        self.with_editor(|editor| {
            editor.install_controller(controller);
            if let Some(character) = character {
                let character_path = character.path.resolve(project_directory);
                editor.load_pmx(character_path, Some(character));
            }
        });
        remember_project(&path);
        refresh_recent_projects_menu();
        self.refresh_menus();
    }

    fn new_project(&self) {
        if !self.confirm_replace_editor_document() {
            return;
        }
        self.with_editor(|editor| editor.reset_controller());
    }

    /// Resolves a pending application-termination request after the user has
    /// answered the unsaved-changes confirmation.
    fn confirm_quit(&self) {
        let proceed = {
            let editor = self.editor.borrow();
            let window = editor.as_ref().and_then(|window| window.delegate.as_ref());
            let proceed = window
                .map(|window| window.confirm_unsaved_changes())
                .unwrap_or(true);
            if proceed && let Some(window) = window {
                window.mark_discard_confirmed();
            }
            proceed
        };
        App::reply_to_termination_request(proceed);
    }

    /// Confirms that the current document may be replaced by a new or
    /// opened project without losing unsaved changes.
    fn confirm_replace_editor_document(&self) -> bool {
        self.editor
            .borrow()
            .as_ref()
            .and_then(|window| window.delegate.as_ref())
            .map(|window| window.confirm_unsaved_changes())
            .unwrap_or(true)
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

    fn show_pmx_loading(&self, request_id: u64, source: PmxSourceIdentity) {
        self.finish_pmx_loading(None);

        let editor_windows = self.editor.borrow();
        let Some(editor_window) = editor_windows.as_ref() else {
            return;
        };
        let sheet = PmxLoadingSheet::window(&source);
        editor_window.begin_sheet(&sheet, || {});
        drop(editor_windows);
        self.pmx_loading.replace(Some(ActivePmxLoadingSheet {
            request_id,
            window: sheet,
        }));
    }

    fn update_pmx_loading(&self, progress: PmxLoadProgress) {
        let loading = self.pmx_loading.borrow();
        let Some(active) = loading
            .as_ref()
            .filter(|active| active.request_id == progress.request_id())
        else {
            return;
        };
        if let Some(sheet) = active.window.delegate.as_ref() {
            sheet.set_progress(&progress);
        }
    }

    fn finish_pmx_loading(&self, request_id: Option<u64>) {
        let active = {
            let mut loading = self.pmx_loading.borrow_mut();
            let matches = match request_id {
                Some(request_id) => loading
                    .as_ref()
                    .is_some_and(|active| active.request_id == request_id),
                None => loading.is_some(),
            };
            matches.then(|| loading.take()).flatten()
        };
        let Some(active) = active else {
            return;
        };
        let editor_windows = self.editor.borrow();
        if let Some(editor_window) = editor_windows.as_ref() {
            editor_window.end_sheet(&active.window);
            active.window.close();
        }
    }

    fn show_pmx_load_error(
        &self,
        request_id: Option<u64>,
        identity: PmxSourceIdentity,
        message: String,
    ) {
        if let Some(request_id) = request_id {
            if self
                .pmx_loading
                .borrow()
                .as_ref()
                .is_none_or(|active| active.request_id != request_id)
            {
                return;
            }
            self.finish_pmx_loading(Some(request_id));
        }
        let source = display_pmx_source(&identity);
        let short_error = localization::format(Key::PmxLoadFailed, &[("path", &source)]);
        {
            let editor_windows = self.editor.borrow();
            if let Some(editor) = editor_windows
                .as_ref()
                .and_then(|window| window.delegate.as_ref())
            {
                editor.show_error(&short_error);
            }
        }
        let details = localization::format(
            Key::PmxLoadFailedDetails,
            &[("path", &source), ("error", &message)],
        );
        Alert::new(localization::text(Key::AppName), &details).show();
    }

    fn show_startup_error(&self, error: &str) {
        Alert::new(localization::text(Key::AppName), error).show();
    }
}

fn set_model_file_types(panel: &FileSelectPanel) {
    let extensions = [NSString::new("pmx"), NSString::new("zip")];
    let identifiers = extensions
        .iter()
        .map(|extension| &*extension.objc as *const _ as id)
        .collect::<Vec<_>>();
    let allowed = NSArray::new(&identifiers);
    unsafe {
        let _: () = msg_send![&*panel.panel, setAllowedFileTypes: &*allowed];
        let _: () = msg_send![&*panel.panel, setAllowsOtherFileTypes: NO];
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
