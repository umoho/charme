use std::{cell::RefCell, path::PathBuf};

use cacao::{
    appkit::{
        App, AppDelegate,
        menu::{Menu, MenuItem},
        window::{Window, WindowConfig, WindowDelegate},
    },
    color::Color,
    filesystem::FileSelectPanel,
    foundation::{YES, id},
    image::{Image, ImageView},
    layout::{Layout, LayoutConstraint},
    notification_center::Dispatcher,
    objc::{class, msg_send, sel, sel_impl},
    text::{Font, Label},
    view::View,
};
use charme_renderer::{Frame, OutputSize, PmxSceneInfo, RendererNotification};
use core_graphics::geometry::CGRect;

use crate::{
    bridge::RenderBridge, frame_image::make_image, interaction::OrbitInputView,
    slider::BrightnessSlider,
};

pub(crate) enum Message {
    Frame { frame: Frame, scale: f64 },
    Brightness(f32),
    Orbit { delta_x: f32, delta_y: f32 },
    Zoom(f32),
    ChoosePmx,
    LoadPmx(PathBuf),
    RendererNotification(RendererNotification),
    Failed(String),
}

pub(crate) struct CharmeApp {
    window: Window<EditorWindow>,
}

impl Default for CharmeApp {
    fn default() -> Self {
        let mut config = WindowConfig::default();
        config.set_initial_dimensions(80.0, 80.0, 1280.0, 800.0);
        Self {
            window: Window::with(config, EditorWindow::new()),
        }
    }
}

impl AppDelegate for CharmeApp {
    fn did_finish_launching(&self) {
        App::set_menu(menus());
        self.window.show();
        self.window
            .delegate
            .as_ref()
            .expect("window delegate should exist")
            .start_renderer();
        activate_app();
    }

    fn should_terminate_after_last_window_closed(&self) -> bool {
        true
    }
}

impl Dispatcher for CharmeApp {
    type Message = Message;

    fn on_ui_message(&self, message: Self::Message) {
        let window = self
            .window
            .delegate
            .as_ref()
            .expect("window delegate should exist");
        match message {
            Message::Frame { frame, scale } => window.display(frame, scale),
            Message::Brightness(value) => window.set_brightness(value),
            Message::Orbit { delta_x, delta_y } => window.orbit(delta_x, delta_y),
            Message::Zoom(delta) => window.zoom(delta),
            Message::ChoosePmx => window.choose_pmx(),
            Message::LoadPmx(path) => window.load_pmx(path),
            Message::RendererNotification(notification) => {
                window.handle_renderer_notification(notification);
            }
            Message::Failed(error) => window.show_error(&error),
        }
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
    scene_heading: Label,
    scene_info: Label,
    materials_heading: Label,
    material_list: Label,
    inspector_heading: Label,
    inspector_body: Label,
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

        let app_title = label("CHARME", 18.0, true, Color::SystemWhite);
        let scene_heading = label("SCENE", 11.0, true, Color::LabelSecondary);
        let scene_info = label(
            "No character loaded\n\nChoose File → Open PMX…",
            13.0,
            false,
            Color::SystemWhite,
        );
        scene_info.set_max_number_of_lines(0);
        let materials_heading = label("MATERIAL SLOTS", 11.0, true, Color::LabelSecondary);
        let material_list = label("—", 12.0, false, Color::SystemWhite);
        material_list.set_max_number_of_lines(0);

        let inspector_heading = label("INSPECTOR", 11.0, true, Color::LabelSecondary);
        let inspector_body = label(
            "Select a material slot to edit its Charme material.\n\nShader-driven controls will appear here.",
            13.0,
            false,
            Color::SystemWhite,
        );
        inspector_body.set_max_number_of_lines(0);
        let brightness_label = label("Viewport brightness", 12.0, false, Color::SystemWhite);
        let brightness = BrightnessSlider::new(0.3);
        let status = label(
            "Initializing Bevy renderer…",
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
            scene_heading,
            scene_info,
            materials_heading,
            material_list,
            inspector_heading,
            inspector_body,
            brightness_label,
            brightness,
            current_image: RefCell::new(None),
            bridge: RefCell::new(None),
        }
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
                    "Frame {sequence} · {width}×{height} px · Drag to orbit · Scroll to zoom"
                ));
            }
            Err(error) => self.show_error(&error),
        }
    }

    fn choose_pmx(&self) {
        let mut panel = FileSelectPanel::new();
        panel.set_can_choose_files(true);
        panel.set_can_choose_directories(false);
        panel.set_allows_multiple_selection(false);
        panel.set_message("Choose a PMX character model to preview in Charme.");
        panel.show(|urls| {
            if let Some(url) = urls.first() {
                App::<CharmeApp, Message>::dispatch_main(Message::LoadPmx(url.pathbuf()));
            }
        });
    }

    fn load_pmx(&self, path: PathBuf) {
        self.scene_info
            .set_text(format!("Loading…\n{}", path.display()));
        self.material_list.set_text("Loading material slots…");
        self.status.set_text("Loading PMX and textures…");
        if let Some(bridge) = self.bridge.borrow().as_ref() {
            bridge.load_pmx(path);
        }
    }

    fn handle_renderer_notification(&self, notification: RendererNotification) {
        match notification {
            RendererNotification::PmxLoaded(info) => self.show_scene_info(&info),
            RendererNotification::PmxLoadFailed { path, message } => {
                self.scene_info
                    .set_text(format!("Could not load\n{}", path.display()));
                self.show_error(&message);
            }
            _ => {}
        }
    }

    fn show_scene_info(&self, info: &PmxSceneInfo) {
        self.scene_info.set_text(format!(
            "{}\n\n{} vertices · {} indices",
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
            "No material slots".to_owned()
        } else {
            slots.join("\n")
        };
        if remaining > 0 {
            text.push_str(&format!("\n… and {remaining} more"));
        }
        self.material_list.set_text(text);

        if let Some(slot) = info.material_slots().first() {
            self.inspector_body.set_text(format!(
                "{}\n\nSource slot {}\nDiffuse: {}\nSphere: {}\nToon: {}\n\nCharme material controls are coming next.",
                slot.name(),
                slot.index(),
                slot.diffuse_texture().unwrap_or("—"),
                slot.sphere_texture().unwrap_or("—"),
                slot.toon_texture().unwrap_or("—"),
            ));
        }
        self.status.set_text(if info.warnings().is_empty() {
            format!(
                "Loaded {} · {} material slots",
                info.name(),
                info.material_slots().len()
            )
        } else {
            format!(
                "Loaded {} with {} warning(s)",
                info.name(),
                info.warnings().len()
            )
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
        self.status.set_text(format!("Error: {error}"));
    }
}

impl WindowDelegate for EditorWindow {
    const NAME: &'static str = "CharmeEditorWindow";

    fn did_load(&mut self, window: Window) {
        window.set_title("Charme · Character Material Editor");
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
                .offset(16.0),
            self.scene_heading
                .top
                .constraint_equal_to(&self.app_title.bottom)
                .offset(28.0),
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
            "File",
            vec![
                MenuItem::new("Open PMX…").key("o").action(|| {
                    App::<CharmeApp, Message>::dispatch_main(Message::ChoosePmx);
                }),
                MenuItem::Separator,
                MenuItem::CloseWindow,
            ],
        ),
        Menu::new(
            "Edit",
            vec![
                MenuItem::Undo,
                MenuItem::Redo,
                MenuItem::Separator,
                MenuItem::Copy,
            ],
        ),
        Menu::new("View", vec![MenuItem::EnterFullScreen]),
        Menu::new("Window", vec![MenuItem::Minimize, MenuItem::Zoom]),
    ]
}

fn activate_app() {
    App::activate();
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}
