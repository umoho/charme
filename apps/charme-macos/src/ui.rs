use cacao::{
    color::Color,
    text::{Font, Label},
    view::View,
};

pub(crate) fn panel(color: Color) -> View {
    let view = View::new();
    view.set_background_color(color);
    view
}

pub(crate) fn label(text: &str, size: f64, bold: bool, color: Color) -> Label {
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
