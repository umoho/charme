use std::path::Path;

use cacao::{
    appkit::window::{TitleVisibility, Window, WindowConfig, WindowDelegate, WindowStyle},
    color::Color,
    layout::{Layout, LayoutConstraint},
    progress::{ProgressIndicator, ProgressIndicatorStyle},
    text::{Label, LineBreakMode},
    view::View,
};

use crate::{
    localization::{self, Key},
    ui::{label, panel},
};

pub(crate) struct PmxLoadingSheet {
    content: View,
    heading: Label,
    stage: Label,
    progress: ProgressIndicator,
}

impl PmxLoadingSheet {
    pub(crate) fn window(path: &Path) -> Window<Self> {
        let mut config = WindowConfig::default();
        config.set_styles(&[WindowStyle::Titled]);
        config.set_initial_dimensions(0.0, 0.0, 420.0, 150.0);
        Window::with(config, Self::new(path))
    }

    fn new(path: &Path) -> Self {
        let content = panel(Color::MacOSWindowBackgroundColor);
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| path.display().to_string());
        let heading = label(&file_name, 15.0, true, Color::Label);
        heading.set_line_break_mode(LineBreakMode::TruncateMiddle);
        heading.set_max_number_of_lines(1);
        let stage = label(
            localization::text(Key::LoadingPmxTextures),
            11.0,
            false,
            Color::LabelSecondary,
        );
        stage.set_max_number_of_lines(1);
        let progress = ProgressIndicator::new();
        progress.set_style(ProgressIndicatorStyle::Bar);
        progress.set_indeterminate(true);
        progress.start_animation();

        Self {
            content,
            heading,
            stage,
            progress,
        }
    }
}

impl WindowDelegate for PmxLoadingSheet {
    const NAME: &'static str = "CharmePmxLoadingSheet";

    fn did_load(&mut self, window: Window) {
        window.set_title(localization::text(Key::AppName));
        window.set_title_visibility(TitleVisibility::Visible);
        window.set_content_view(&self.content);
        window.set_minimum_content_size(360.0, 130.0);

        self.content.add_subview(&self.heading);
        self.content.add_subview(&self.stage);
        self.content.add_subview(&self.progress);

        LayoutConstraint::activate(&[
            self.heading
                .top
                .constraint_equal_to(&self.content.top)
                .offset(20.0),
            self.heading
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(24.0),
            self.heading
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-24.0),
            self.heading.height.constraint_equal_to_constant(20.0),
            self.stage
                .top
                .constraint_equal_to(&self.heading.bottom)
                .offset(7.0),
            self.stage
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(24.0),
            self.stage
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-24.0),
            self.stage.height.constraint_equal_to_constant(16.0),
            self.progress
                .top
                .constraint_equal_to(&self.stage.bottom)
                .offset(12.0),
            self.progress
                .leading
                .constraint_equal_to(&self.content.leading)
                .offset(24.0),
            self.progress
                .trailing
                .constraint_equal_to(&self.content.trailing)
                .offset(-24.0),
            self.progress.height.constraint_equal_to_constant(18.0),
            self.progress
                .bottom
                .constraint_equal_to(&self.content.bottom)
                .offset(-20.0),
        ]);
    }

    fn will_close(&self) {
        self.progress.stop_animation();
    }
}
