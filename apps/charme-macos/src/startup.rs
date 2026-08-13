use cacao::{
    appkit::{
        App,
        window::{TitleVisibility, Window, WindowDelegate},
    },
    button::{BezelStyle, Button},
    color::Color,
    foundation::id,
    image::{Image, MacSystemIcon},
    layout::{Layout, LayoutConstraint},
    objc::{msg_send, sel, sel_impl},
    text::{Font, Label, TextAlign},
    view::View,
};

use crate::{
    app::{CharmeApp, MenuContext, Message, recent_projects},
    localization::{self, Key},
    ui::{label, panel},
};

fn style_action_button(button: &Button) {
    button.set_bezel_style(BezelStyle::ShadowlessSquare);
    button.set_background_color(Color::SystemFillSecondary);
    button.set_font(Font::system(13.0));
    button.set_text_color(Color::Label);
    button.objc.with_mut(|object| unsafe {
        let tint: id = Color::SystemBlue.into();
        let _: () = msg_send![object, setContentTintColor: tint];
        // NSImageAbove keeps the icon and title visually close to the IntelliJ-style
        // action tiles while retaining a native NSButton for keyboard access.
        let _: () = msg_send![object, setImagePosition: 5usize];
    });
}

pub(crate) struct StartupWindow {
    content: View,
    title: Label,
    subtitle: Label,
    open_button: Button,
    new_button: Button,
    formats: Label,
    recent_heading: Label,
    recent_panel: View,
    empty_recent: Label,
    recent_buttons: Vec<Button>,
    divider: View,
    status: Label,
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
        let recent_panel = panel(Color::SystemFillQuaternary);
        recent_panel.layer.set_corner_radius(8.0);
        let empty_recent = label(
            localization::text(Key::NoRecentProjectsHint),
            13.0,
            false,
            Color::LabelSecondary,
        );
        empty_recent.set_text_alignment(TextAlign::Center);
        let divider = panel(Color::Separator);
        let status = label("", 11.0, false, Color::SystemRed);

        let mut open_button = Button::new(localization::text(Key::OpenProject));
        open_button.set_image(Image::system_icon(MacSystemIcon::Folder));
        style_action_button(&open_button);
        open_button.set_key_equivalent("o");
        open_button.set_action(|| {
            App::<CharmeApp, Message>::dispatch_main(Message::ChooseProject);
        });
        let mut new_button = Button::new(localization::text(Key::NewProject));
        new_button.set_image(Image::system_icon(MacSystemIcon::Add));
        style_action_button(&new_button);
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
            let title = localization::format(
                Key::RecentProjectRow,
                &[("name", &name), ("path", &project.display())],
            );
            let mut button = Button::new(&title);
            button.set_image(Image::system_icon(MacSystemIcon::Folder));
            button.set_bezel_style(BezelStyle::Inline);
            button.set_bordered(false);
            button.set_text_color(Color::Label);
            button.set_action(move || {
                App::<CharmeApp, Message>::dispatch_main(Message::OpenProject(project.clone()));
            });
            recent_buttons.push(button);
        }
        empty_recent.set_hidden(!recent_buttons.is_empty());

        Self {
            content,
            title,
            subtitle,
            open_button,
            new_button,
            formats,
            recent_heading,
            recent_panel,
            empty_recent,
            recent_buttons,
            divider,
            status,
        }
    }

    pub(crate) fn show_error(&self, error: &str) {
        self.status.set_text(error);
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
        window.set_minimum_content_size(560.0, 520.0);
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
        self.content.add_subview(&self.recent_panel);
        self.recent_panel.add_subview(&self.empty_recent);
        for button in &self.recent_buttons {
            self.recent_panel.add_subview(button);
        }
        self.content.add_subview(&self.divider);
        self.content.add_subview(&self.new_button);
        self.content.add_subview(&self.open_button);

        let mut constraints = vec![
            self.title
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.title
                .top
                .constraint_equal_to(&self.content.top)
                .offset(64.0),
            self.subtitle
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.subtitle
                .top
                .constraint_equal_to(&self.title.bottom)
                .offset(12.0),
            self.formats
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.formats
                .top
                .constraint_equal_to(&self.subtitle.bottom)
                .offset(8.0),
            self.recent_heading
                .center_x
                .constraint_equal_to(&self.content.center_x),
            self.recent_heading
                .top
                .constraint_equal_to(&self.formats.bottom)
                .offset(28.0),
            self.recent_panel
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(72.0),
            self.recent_panel
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-72.0),
            self.recent_panel
                .top
                .constraint_equal_to(&self.recent_heading.bottom)
                .offset(10.0),
            self.recent_panel.height.constraint_equal_to_constant(190.0),
            self.empty_recent
                .center_x
                .constraint_equal_to(&self.recent_panel.center_x),
            self.empty_recent
                .center_y
                .constraint_equal_to(&self.recent_panel.center_y),
            self.empty_recent.width.constraint_equal_to_constant(240.0),
            self.divider
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(72.0),
            self.divider
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-72.0),
            self.divider
                .top
                .constraint_equal_to(&self.recent_panel.bottom)
                .offset(28.0),
            self.divider.height.constraint_equal_to_constant(1.0),
            self.new_button
                .center_x
                .constraint_equal_to(&self.content.center_x)
                .offset(-76.0),
            self.new_button
                .top
                .constraint_equal_to(&self.divider.bottom)
                .offset(16.0),
            self.new_button.width.constraint_equal_to_constant(112.0),
            self.new_button.height.constraint_equal_to_constant(88.0),
            self.open_button
                .center_x
                .constraint_equal_to(&self.content.center_x)
                .offset(76.0),
            self.open_button
                .top
                .constraint_equal_to(&self.divider.bottom)
                .offset(16.0),
            self.open_button.width.constraint_equal_to_constant(112.0),
            self.open_button.height.constraint_equal_to_constant(88.0),
            self.status
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(72.0),
            self.status
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-72.0),
            self.status
                .bottom
                .constraint_equal_to(&self.content.bottom)
                .offset(-12.0),
        ];
        for (index, button) in self.recent_buttons.iter().enumerate() {
            constraints.extend([
                button
                    .leading
                    .constraint_equal_to(&self.recent_panel.leading)
                    .offset(12.0),
                button
                    .trailing
                    .constraint_equal_to(&self.recent_panel.trailing)
                    .offset(-12.0),
                button
                    .top
                    .constraint_equal_to(&self.recent_panel.top)
                    .offset(5.0 + index as f64 * 36.0),
                button.height.constraint_equal_to_constant(34.0),
            ]);
        }
        LayoutConstraint::activate(&constraints);
    }
}
