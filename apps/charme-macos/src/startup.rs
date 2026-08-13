use cacao::{
    appkit::{
        App,
        window::{TitleVisibility, Window, WindowDelegate},
    },
    button::{BezelStyle, Button},
    color::Color,
    layout::{Layout, LayoutConstraint},
    text::Label,
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
    formats: Label,
    recent_heading: Label,
    recent_buttons: Vec<Button>,
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
            let title = localization::format(
                Key::RecentProjectTitle,
                &[("name", &name), ("path", &project.display())],
            );
            let mut button = Button::new(&title);
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
