use cacao::{
    color::Color,
    layout::{Layout, LayoutConstraint},
    text::{Font, Label, LineBreakMode},
    view::View,
};

use crate::ui::label;

/// A compact two-column Inspector property row.
pub(crate) struct PropertyRow {
    pub(crate) view: View,
    _name: Label,
    value: Label,
}

impl PropertyRow {
    pub(crate) fn new(name: &str) -> Self {
        let view = View::new();
        let name_label = label(name, 11.0, false, Color::LabelSecondary);
        let value = label("", 11.0, false, Color::Label);
        value.set_font(Font::system(11.0));
        value.set_max_number_of_lines(1);
        value.set_line_break_mode(LineBreakMode::TruncateTail);

        view.add_subview(&name_label);
        view.add_subview(&value);
        LayoutConstraint::activate(&[
            name_label.leading.constraint_equal_to(&view.leading),
            name_label.center_y.constraint_equal_to(&view.center_y),
            value
                .leading
                .constraint_equal_to(&view.leading)
                .offset(92.0),
            value.trailing.constraint_equal_to(&view.trailing),
            value.center_y.constraint_equal_to(&view.center_y),
        ]);

        Self {
            view,
            _name: name_label,
            value,
        }
    }

    pub(crate) fn set_value(&self, value: impl Into<String>) {
        self.value.set_text(value.into());
    }
}
