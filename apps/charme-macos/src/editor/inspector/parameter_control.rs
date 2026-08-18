use std::sync::OnceLock;

use cacao::{
    appkit::App,
    foundation::{BOOL, NO, NSArray, NSString, YES, id, nil},
    layout::{Layout, LayoutConstraint},
    objc::{
        class,
        declare::ClassDecl,
        msg_send,
        runtime::{Class, Object, Sel},
        sel, sel_impl,
    },
    text::{Font, Label},
    utils::properties::ObjcProperty,
    view::View,
};
use charme_core::ParameterValue;

use crate::{
    app::{CharmeApp, Message},
    shader_inspection::{ParameterControlKind, ParameterControlSpec},
};

const TARGET_IVAR: &str = "charmeParameterTarget";
const COMPONENT_IVAR: &str = "charmeParameterComponent";

struct ParameterTarget {
    key: String,
    kind: ParameterControlKind,
    values: Vec<f64>,
}

pub(crate) struct ParameterControl {
    pub(crate) view: View,
    key: String,
    _name: Option<Label>,
    value_label: Option<Label>,
    _controls: Vec<ObjcProperty>,
    _target: Box<ParameterTarget>,
}

impl ParameterControl {
    pub(crate) fn new(spec: &ParameterControlSpec) -> Self {
        let view = View::new();
        let values = if spec.initial_values.is_empty() {
            vec![spec.initial]
        } else {
            spec.initial_values.clone()
        };
        let mut target = Box::new(ParameterTarget {
            key: spec.key.clone(),
            kind: spec.kind,
            values: values.clone(),
        });
        let mut controls = Vec::new();

        let (name, value_label) = match spec.kind {
            ParameterControlKind::Boolean => {
                let button = make_button(&mut target, spec.initial >= 0.5, &spec.label);
                add_control(&view, button, &mut controls);
                pin_full_control(button, &view);
                (None, None)
            }
            ParameterControlKind::Color => {
                let name = make_name_label(spec);
                let value_label = make_value_label(format_color(&values));
                let color_well = make_color_well(&mut target, &values);
                view.add_subview(&name);
                view.add_subview(&value_label);
                add_retained_control(&view, color_well, &mut controls);
                LayoutConstraint::activate(&[
                    name.top.constraint_equal_to(&view.top),
                    name.leading.constraint_equal_to(&view.leading),
                    value_label.top.constraint_equal_to(&view.top),
                ]);
                pin_color_well(color_well, &view, &value_label);
                (Some(name), Some(value_label))
            }
            ParameterControlKind::Vector2
            | ParameterControlKind::Vector3
            | ParameterControlKind::Vector4 => {
                let name = make_name_label(spec);
                let value_label = make_value_label(format_values(&values));
                view.add_subview(&name);
                view.add_subview(&value_label);
                LayoutConstraint::activate(&[
                    name.top.constraint_equal_to(&view.top),
                    name.leading.constraint_equal_to(&view.leading),
                    value_label.top.constraint_equal_to(&view.top),
                    value_label.trailing.constraint_equal_to(&view.trailing),
                ]);
                let count = vector_length(spec.kind);
                let sliders = (0..count)
                    .map(|component| {
                        let slider = make_slider(
                            values.get(component).copied().unwrap_or(0.0),
                            spec.minimum,
                            spec.maximum,
                            &mut target,
                            component,
                        );
                        add_control(&view, slider, &mut controls);
                        slider
                    })
                    .collect::<Vec<_>>();
                pin_vector_sliders(&sliders, &view);
                (Some(name), Some(value_label))
            }
            ParameterControlKind::Float
            | ParameterControlKind::SignedInteger
            | ParameterControlKind::UnsignedInteger => {
                let name = make_name_label(spec);
                let value_label = make_value_label(format_value(spec.initial, spec.kind));
                let slider = make_slider(spec.initial, spec.minimum, spec.maximum, &mut target, 0);
                view.add_subview(&name);
                view.add_subview(&value_label);
                add_control(&view, slider, &mut controls);
                LayoutConstraint::activate(&[
                    name.top.constraint_equal_to(&view.top),
                    name.leading.constraint_equal_to(&view.leading),
                    value_label.top.constraint_equal_to(&view.top),
                    value_label.trailing.constraint_equal_to(&view.trailing),
                ]);
                pin_slider(slider, &view);
                (Some(name), Some(value_label))
            }
        };

        Self {
            view,
            key: spec.key.clone(),
            _name: name,
            value_label,
            _controls: controls,
            _target: target,
        }
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn set_value(&self, value: &ParameterValue) {
        let Some(label) = self.value_label.as_ref() else {
            return;
        };
        let text = match value {
            ParameterValue::F32(value) => format!("{value:.3}"),
            ParameterValue::I32(value) => format!("{value}"),
            ParameterValue::U32(value) => format!("{value}"),
            ParameterValue::Bool(value) => format!("{value}"),
            ParameterValue::Vec2(values) => format_values_f32(values),
            ParameterValue::Vec3(values) => format_values_f32(values),
            ParameterValue::Vec4(values) => format_color_f32(values),
        };
        label.set_text(text);
    }
}

fn make_name_label(spec: &ParameterControlSpec) -> Label {
    let name = Label::new();
    name.set_text(&spec.label);
    name.set_font(Font::system(12.0));
    name
}

fn make_value_label(value: String) -> Label {
    let label = Label::new();
    label.set_font(Font::system(11.0));
    label.set_text(value);
    label
}

fn add_control(view: &View, control: id, retained: &mut Vec<ObjcProperty>) {
    view.objc.with_mut(|container| unsafe {
        let _: () = msg_send![container, addSubview: control];
    });
    retained.push(ObjcProperty::retain(control));
}

fn add_retained_control(view: &View, control: id, retained: &mut Vec<ObjcProperty>) {
    view.objc.with_mut(|container| unsafe {
        let _: () = msg_send![container, addSubview: control];
    });
    retained.push(ObjcProperty::from_retained(control));
}

fn make_slider(
    initial: f64,
    minimum: f64,
    maximum: f64,
    target: &mut Box<ParameterTarget>,
    component: usize,
) -> id {
    unsafe {
        let slider: id = msg_send![slider_class(),
            sliderWithValue: initial
            minValue: minimum
            maxValue: maximum
            target: nil
            action: nil
        ];
        let _: () = msg_send![slider, setContinuous: YES];
        let _: () = msg_send![slider, setTranslatesAutoresizingMaskIntoConstraints: NO];
        install_target(slider, target, component);
        let _: () = msg_send![slider, setTarget: slider];
        let _: () = msg_send![slider, setAction: sel!(parameterChanged:)];
        slider
    }
}

fn make_button(target: &mut Box<ParameterTarget>, checked: bool, label: &str) -> id {
    unsafe {
        let title = NSString::new("");
        let button: id = msg_send![button_class(), buttonWithTitle: &*title];
        let _: () = msg_send![button, setTranslatesAutoresizingMaskIntoConstraints: NO];
        let _: () = msg_send![button, setButtonType: 3isize];
        let _: () = msg_send![button, setTitle: &*NSString::new(label)];
        let _: () = msg_send![button, setState: if checked { 1isize } else { 0isize }];
        install_target(button, target, 0);
        let _: () = msg_send![button, setTarget: button];
        let _: () = msg_send![button, setAction: sel!(parameterChanged:)];
        button
    }
}

fn make_color_well(target: &mut Box<ParameterTarget>, values: &[f64]) -> id {
    let values = [
        values.first().copied().unwrap_or(1.0),
        values.get(1).copied().unwrap_or(1.0),
        values.get(2).copied().unwrap_or(1.0),
        values.get(3).copied().unwrap_or(1.0),
    ];
    unsafe {
        let well: id = msg_send![color_well_class(), new];
        let color: id = msg_send![class!(NSColor),
            colorWithSRGBRed: values[0]
            green: values[1]
            blue: values[2]
            alpha: values[3]
        ];
        let _: () = msg_send![well, setColor: color];
        let _: () = msg_send![well, setTranslatesAutoresizingMaskIntoConstraints: NO];
        install_target(well, target, 0);
        let _: () = msg_send![well, setTarget: well];
        let _: () = msg_send![well, setAction: sel!(parameterChanged:)];
        well
    }
}

fn install_target(control: id, target: &mut Box<ParameterTarget>, component: usize) {
    unsafe {
        let target_pointer = (&mut **target as *mut ParameterTarget) as usize;
        (&mut *control).set_ivar(TARGET_IVAR, target_pointer);
        (&mut *control).set_ivar(COMPONENT_IVAR, component);
    }
}

fn pin_slider(slider: id, parent: &View) {
    parent.objc.get(|parent| unsafe {
        let slider_leading: id = msg_send![slider, leadingAnchor];
        let parent_leading: id = msg_send![parent, leadingAnchor];
        let slider_trailing: id = msg_send![slider, trailingAnchor];
        let parent_trailing: id = msg_send![parent, trailingAnchor];
        let slider_top: id = msg_send![slider, topAnchor];
        let slider_bottom: id = msg_send![slider, bottomAnchor];
        let parent_top: id = msg_send![parent, topAnchor];
        let parent_bottom: id = msg_send![parent, bottomAnchor];
        let constraints = NSArray::new(&[
            msg_send![slider_leading, constraintEqualToAnchor: parent_leading],
            msg_send![slider_trailing, constraintEqualToAnchor: parent_trailing],
            msg_send![slider_top, constraintEqualToAnchor: parent_top constant: 22.0],
            msg_send![slider_bottom, constraintEqualToAnchor: parent_bottom],
        ]);
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    });
}

fn pin_vector_sliders(sliders: &[id], parent: &View) {
    parent.objc.get(|parent| unsafe {
        let mut constraints = Vec::new();
        let parent_top: id = msg_send![parent, topAnchor];
        let parent_bottom: id = msg_send![parent, bottomAnchor];
        let parent_leading: id = msg_send![parent, leadingAnchor];
        let parent_trailing: id = msg_send![parent, trailingAnchor];
        for (index, slider) in sliders.iter().copied().enumerate() {
            let leading: id = msg_send![slider, leadingAnchor];
            let trailing: id = msg_send![slider, trailingAnchor];
            let top: id = msg_send![slider, topAnchor];
            let bottom: id = msg_send![slider, bottomAnchor];
            constraints.push(msg_send![top, constraintEqualToAnchor: parent_top constant: 22.0]);
            constraints.push(msg_send![bottom, constraintEqualToAnchor: parent_bottom]);
            if index == 0 {
                constraints.push(msg_send![leading, constraintEqualToAnchor: parent_leading]);
            } else {
                let previous: id = sliders[index - 1];
                let previous_trailing: id = msg_send![previous, trailingAnchor];
                constraints.push(
                    msg_send![leading, constraintEqualToAnchor: previous_trailing constant: 2.0],
                );
                let first: id = sliders[0];
                let first_width: id = msg_send![first, widthAnchor];
                let width: id = msg_send![slider, widthAnchor];
                constraints.push(msg_send![width, constraintEqualToAnchor: first_width]);
            }
            if index + 1 == sliders.len() {
                constraints.push(msg_send![trailing, constraintEqualToAnchor: parent_trailing]);
            }
        }
        let constraints = NSArray::new(&constraints);
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    });
}

fn pin_full_control(control: id, parent: &View) {
    parent.objc.get(|parent| unsafe {
        let top: id = msg_send![control, topAnchor];
        let bottom: id = msg_send![control, bottomAnchor];
        let leading: id = msg_send![control, leadingAnchor];
        let trailing: id = msg_send![control, trailingAnchor];
        let parent_top: id = msg_send![parent, topAnchor];
        let parent_bottom: id = msg_send![parent, bottomAnchor];
        let parent_leading: id = msg_send![parent, leadingAnchor];
        let parent_trailing: id = msg_send![parent, trailingAnchor];
        let constraints = NSArray::new(&[
            msg_send![top, constraintEqualToAnchor: parent_top],
            msg_send![bottom, constraintEqualToAnchor: parent_bottom],
            msg_send![leading, constraintEqualToAnchor: parent_leading],
            msg_send![trailing, constraintEqualToAnchor: parent_trailing],
        ]);
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    });
}

fn pin_color_well(well: id, parent: &View, value_label: &Label) {
    parent.objc.get(|parent| unsafe {
        value_label.objc.get(|value_label| {
            let top: id = msg_send![well, topAnchor];
            let trailing: id = msg_send![well, trailingAnchor];
            let leading: id = msg_send![well, leadingAnchor];
            let parent_top: id = msg_send![parent, topAnchor];
            let parent_trailing: id = msg_send![parent, trailingAnchor];
            let value_trailing: id = msg_send![value_label, trailingAnchor];
            let width: id = msg_send![well, widthAnchor];
            let height: id = msg_send![well, heightAnchor];
            let constraints = NSArray::new(&[
                msg_send![top, constraintEqualToAnchor: parent_top],
                msg_send![trailing, constraintEqualToAnchor: parent_trailing],
                msg_send![width, constraintEqualToConstant: 32.0],
                msg_send![height, constraintEqualToConstant: 22.0],
                msg_send![leading, constraintEqualToAnchor: value_trailing constant: 8.0],
            ]);
            let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
        });
    });
}

fn slider_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let mut declaration = ClassDecl::new("CharmeMaterialParameterSlider", class!(NSSlider))
            .expect("parameter slider class should only be registered once");
        declaration.add_ivar::<usize>(TARGET_IVAR);
        declaration.add_ivar::<usize>(COMPONENT_IVAR);
        declaration.add_method(
            sel!(parameterChanged:),
            parameter_changed as extern "C" fn(&Object, Sel, id),
        );
        declaration.register() as *const Class as usize
    }) as *const Class
}

fn button_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let mut declaration = ClassDecl::new("CharmeMaterialParameterButton", class!(NSButton))
            .expect("parameter button class should only be registered once");
        declaration.add_ivar::<usize>(TARGET_IVAR);
        declaration.add_ivar::<usize>(COMPONENT_IVAR);
        declaration.add_method(
            sel!(parameterChanged:),
            parameter_changed as extern "C" fn(&Object, Sel, id),
        );
        declaration.register() as *const Class as usize
    }) as *const Class
}

fn color_well_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let mut declaration =
            ClassDecl::new("CharmeMaterialParameterColorWell", class!(NSColorWell))
                .expect("parameter color well class should only be registered once");
        declaration.add_ivar::<usize>(TARGET_IVAR);
        declaration.add_ivar::<usize>(COMPONENT_IVAR);
        declaration.add_method(
            sel!(parameterChanged:),
            parameter_changed as extern "C" fn(&Object, Sel, id),
        );
        declaration.register() as *const Class as usize
    }) as *const Class
}

extern "C" fn parameter_changed(control: &Object, _: Sel, sender: id) {
    let target_pointer = unsafe { *control.get_ivar::<usize>(TARGET_IVAR) };
    let Some(target) = (unsafe { (target_pointer as *mut ParameterTarget).as_mut() }) else {
        return;
    };
    let component = unsafe { *control.get_ivar::<usize>(COMPONENT_IVAR) };
    let value = match target.kind {
        ParameterControlKind::Boolean => {
            let state: isize = unsafe { msg_send![sender, state] };
            ParameterValue::Bool(state != 0)
        }
        ParameterControlKind::Color => {
            let values = color_values(sender);
            target.values = values.iter().map(|value| *value as f64).collect();
            ParameterValue::Vec4(values)
        }
        ParameterControlKind::Vector2
        | ParameterControlKind::Vector3
        | ParameterControlKind::Vector4 => {
            let raw_value: f64 = unsafe { msg_send![sender, doubleValue] };
            if let Some(value) = target.values.get_mut(component) {
                *value = raw_value;
            }
            vector_value(target.kind, &target.values)
        }
        ParameterControlKind::Float => {
            let raw_value: f64 = unsafe { msg_send![sender, doubleValue] };
            target.values[0] = raw_value;
            ParameterValue::F32(raw_value as f32)
        }
        ParameterControlKind::SignedInteger => {
            let raw_value: f64 = unsafe { msg_send![sender, doubleValue] };
            let value = raw_value.round() as i32;
            target.values[0] = value as f64;
            ParameterValue::I32(value)
        }
        ParameterControlKind::UnsignedInteger => {
            let raw_value: f64 = unsafe { msg_send![sender, doubleValue] };
            let value = raw_value.max(0.0).round() as u32;
            target.values[0] = value as f64;
            ParameterValue::U32(value)
        }
    };
    App::<CharmeApp, Message>::dispatch_main(Message::ParameterChanged {
        key: target.key.clone(),
        value,
    });
}

fn color_values(sender: id) -> [f32; 4] {
    unsafe {
        let color: id = msg_send![sender, color];
        let mut red = 1.0;
        let mut green = 1.0;
        let mut blue = 1.0;
        let mut alpha = 1.0;
        let _: BOOL = msg_send![color,
            getRed: &mut red
            green: &mut green
            blue: &mut blue
            alpha: &mut alpha
        ];
        [red as f32, green as f32, blue as f32, alpha as f32]
    }
}

fn vector_value(kind: ParameterControlKind, values: &[f64]) -> ParameterValue {
    match kind {
        ParameterControlKind::Vector2 => ParameterValue::Vec2([
            values.first().copied().unwrap_or_default() as f32,
            values.get(1).copied().unwrap_or_default() as f32,
        ]),
        ParameterControlKind::Vector3 => ParameterValue::Vec3([
            values.first().copied().unwrap_or_default() as f32,
            values.get(1).copied().unwrap_or_default() as f32,
            values.get(2).copied().unwrap_or_default() as f32,
        ]),
        ParameterControlKind::Vector4 => ParameterValue::Vec4([
            values.first().copied().unwrap_or_default() as f32,
            values.get(1).copied().unwrap_or_default() as f32,
            values.get(2).copied().unwrap_or_default() as f32,
            values.get(3).copied().unwrap_or_default() as f32,
        ]),
        _ => unreachable!("vector_value called for a non-vector control"),
    }
}

fn vector_length(kind: ParameterControlKind) -> usize {
    match kind {
        ParameterControlKind::Vector2 => 2,
        ParameterControlKind::Vector3 => 3,
        ParameterControlKind::Vector4 => 4,
        _ => 1,
    }
}

fn format_value(value: f64, kind: ParameterControlKind) -> String {
    match kind {
        ParameterControlKind::Float => format!("{value:.3}"),
        ParameterControlKind::SignedInteger | ParameterControlKind::UnsignedInteger => {
            format!("{value:.0}")
        }
        _ => format!("{value:.3}"),
    }
}

fn format_values(values: &[f64]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!("{value:.3}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn format_values_f32(values: &[f32]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!("{value:.3}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn format_color_f32(values: &[f32; 4]) -> String {
    format_color(&values.iter().map(|value| *value as f64).collect::<Vec<_>>())
}

fn format_color(values: &[f64]) -> String {
    let [red, green, blue, alpha] = [
        values.first().copied().unwrap_or(1.0),
        values.get(1).copied().unwrap_or(1.0),
        values.get(2).copied().unwrap_or(1.0),
        values.get(3).copied().unwrap_or(1.0),
    ];
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        (red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (blue.clamp(0.0, 1.0) * 255.0).round() as u8,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
    )
}
