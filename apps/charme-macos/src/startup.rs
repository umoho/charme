use cacao::{
    appkit::{
        App,
        window::{TitleVisibility, Window, WindowDelegate},
    },
    button::{BezelStyle, Button},
    color::Color,
    image::{Image, ImageView, MacSystemIcon},
    layout::{Layout, LayoutConstraint},
    objc::{msg_send, sel, sel_impl},
    scrollview::ScrollView,
    text::{Label, LineBreakMode, TextAlign},
    view::View,
};

use crate::{
    app::{CharmeApp, MenuContext, Message, recent_projects},
    localization::{self, Key},
    ui::{label, panel},
};

pub(crate) struct StartupWindow {
    content: View,
    title: Label,
    subtitle: Label,
    open_button: Button,
    new_button: Button,
    recent_heading: Label,
    recent_scroll: ScrollView,
    recent_panel: View,
    recent_content_height: f64,
    empty_recent: Label,
    recent_buttons: Vec<Button>,
    recent_names: Vec<Label>,
    recent_paths: Vec<Label>,
    recent_icons: Vec<ImageView>,
}

impl StartupWindow {
    pub(crate) fn new() -> Self {
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
        let recent_heading = label(
            localization::text(Key::RecentProjects),
            13.0,
            true,
            Color::Label,
        );
        let recent_scroll = ScrollView::new();
        let recent_panel = panel(Color::SystemFillQuaternary);
        recent_panel.layer.set_corner_radius(8.0);
        let empty_recent = label(
            localization::text(Key::NoRecentProjectsHint),
            13.0,
            false,
            Color::LabelSecondary,
        );
        empty_recent.set_text_alignment(TextAlign::Center);
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
        let mut recent_names = Vec::new();
        let mut recent_paths = Vec::new();
        let mut recent_icons = Vec::new();
        for project in projects {
            let name = project
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or(localization::text(Key::ProjectFallback));
            let name_label = label(name, 13.0, true, Color::Label);
            let path = label(
                &project.display().to_string(),
                11.0,
                false,
                Color::LabelSecondary,
            );
            path.set_line_break_mode(LineBreakMode::TruncateMiddle);
            path.set_max_number_of_lines(1);

            let icon = ImageView::new();
            icon.set_image(&Image::system_icon(MacSystemIcon::Folder));

            // Keep the hit target separate from the visible content. NSButton's
            // image/title layout is platform-controlled and can overlap multiline text.
            let mut button = Button::new("");
            button.set_bezel_style(BezelStyle::Inline);
            button.set_bordered(false);
            let project_path = project.clone();
            button.set_action(move || {
                App::<CharmeApp, Message>::dispatch_main(Message::OpenProject(
                    project_path.clone(),
                ));
            });

            recent_buttons.push(button);
            recent_names.push(name_label);
            recent_paths.push(path);
            recent_icons.push(icon);
        }
        empty_recent.set_hidden(!recent_buttons.is_empty());
        let recent_content_height = (16.0 + recent_buttons.len() as f64 * 42.0).max(150.0);

        recent_scroll.objc.with_mut(|scroll| {
            recent_panel.objc.with_mut(|panel| unsafe {
                let _: () = msg_send![scroll, setDocumentView: panel];
                let _: () = msg_send![scroll, setHasHorizontalScroller: false];
                let _: () = msg_send![scroll, setAutohidesScrollers: true];
            });
        });

        Self {
            content,
            title,
            subtitle,
            open_button,
            new_button,
            recent_heading,
            recent_scroll,
            recent_panel,
            recent_content_height,
            empty_recent,
            recent_buttons,
            recent_names,
            recent_paths,
            recent_icons,
        }
    }
}

impl WindowDelegate for StartupWindow {
    const NAME: &'static str = "CharmeStartupWindow";

    fn did_become_key(&self) {
        App::<CharmeApp, Message>::dispatch_main(Message::MenuContextChanged(MenuContext::Startup));
    }

    fn did_load(&mut self, window: Window) {
        window.set_title(localization::text(Key::AppName));
        window.set_title_visibility(TitleVisibility::Hidden);
        window.set_titlebar_appears_transparent(true);
        window.set_titlebar_separator_style(0);
        window.set_minimum_content_size(560.0, 560.0);
        window.set_content_view(&self.content);

        for label in [&self.title, &self.subtitle, &self.recent_heading] {
            self.content.add_subview(label);
        }
        self.content.add_subview(&self.recent_scroll);
        self.recent_panel.add_subview(&self.empty_recent);
        for name in &self.recent_names {
            self.recent_panel.add_subview(name);
        }
        for path in &self.recent_paths {
            self.recent_panel.add_subview(path);
        }
        for icon in &self.recent_icons {
            self.recent_panel.add_subview(icon);
        }
        for button in &self.recent_buttons {
            self.recent_panel.add_subview(button);
        }
        self.content.add_subview(&self.new_button);
        self.content.add_subview(&self.open_button);

        let mut constraints = vec![
            self.title
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.title
                .top
                .constraint_equal_to(&self.content.top)
                .offset(52.0),
            self.subtitle
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.subtitle
                .top
                .constraint_equal_to(&self.title.bottom)
                .offset(12.0),
            self.recent_heading
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.recent_heading
                .bottom
                .constraint_equal_to(&self.recent_scroll.top)
                .offset(-10.0),
            self.recent_scroll
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(72.0),
            self.recent_scroll
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-72.0),
            self.recent_scroll
                .bottom
                .constraint_equal_to(&self.content.bottom)
                .offset(-36.0),
            self.recent_scroll
                .height
                .constraint_equal_to_constant(160.0),
            self.recent_panel
                .width
                .constraint_equal_to(&self.recent_scroll.width),
            self.recent_panel
                .height
                .constraint_equal_to_constant(self.recent_content_height),
            self.empty_recent
                .center_x
                .constraint_equal_to(&self.recent_panel.center_x),
            self.empty_recent
                .center_y
                .constraint_equal_to(&self.recent_panel.center_y),
            self.empty_recent.width.constraint_equal_to_constant(240.0),
            self.new_button
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.new_button
                .top
                .constraint_equal_to(&self.open_button.bottom)
                .offset(6.0),
            self.new_button.width.constraint_equal_to_constant(140.0),
            self.new_button.height.constraint_equal_to_constant(38.0),
            self.open_button
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.open_button
                .top
                .constraint_equal_to(&self.subtitle.bottom)
                .offset(20.0),
            self.open_button.width.constraint_equal_to_constant(140.0),
            self.open_button.height.constraint_equal_to_constant(42.0),
        ];
        for (index, (((button, name), path), icon)) in self
            .recent_buttons
            .iter()
            .zip(&self.recent_names)
            .zip(&self.recent_paths)
            .zip(&self.recent_icons)
            .enumerate()
        {
            let row_top = 8.0 + index as f64 * 42.0;
            constraints.extend([
                button
                    .leading
                    .constraint_equal_to(&self.recent_panel.leading)
                    .offset(8.0),
                button
                    .trailing
                    .constraint_equal_to(&self.recent_panel.trailing)
                    .offset(-8.0),
                button
                    .top
                    .constraint_equal_to(&self.recent_panel.top)
                    .offset(row_top),
                button.height.constraint_equal_to_constant(38.0),
                icon.leading
                    .constraint_equal_to(&self.recent_panel.leading)
                    .offset(16.0),
                icon.center_y.constraint_equal_to(&name.center_y),
                icon.width.constraint_equal_to_constant(20.0),
                icon.height.constraint_equal_to_constant(20.0),
                name.leading
                    .constraint_equal_to(&self.recent_panel.leading)
                    .offset(46.0),
                name.trailing
                    .constraint_equal_to(&self.recent_panel.trailing)
                    .offset(-18.0),
                name.top
                    .constraint_equal_to(&self.recent_panel.top)
                    .offset(row_top + 2.0),
                name.height.constraint_equal_to_constant(18.0),
                path.leading
                    .constraint_equal_to(&self.recent_panel.leading)
                    .offset(46.0),
                path.trailing
                    .constraint_equal_to(&self.recent_panel.trailing)
                    .offset(-18.0),
                path.top.constraint_equal_to(&name.bottom),
                path.height.constraint_equal_to_constant(16.0),
            ]);
        }
        LayoutConstraint::activate(&constraints);
    }
}
