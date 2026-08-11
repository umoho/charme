use std::sync::OnceLock;

use cacao::{
    appkit::App,
    foundation::{NO, NSArray, YES, id, nil},
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

use crate::{
    app::{CharmeApp, Message},
    shader_inspection::{ParameterControlKind, ParameterControlSpec},
};

const TARGET_IVAR: &str = "charmeParameterTarget";

struct ParameterTarget {
    key: String,
    kind: ParameterControlKind,
}

pub(crate) struct ParameterControl {
    pub(crate) view: View,
    key: String,
    _name: Label,
    value_label: Label,
    _slider: ObjcProperty,
    _target: Box<ParameterTarget>,
}

impl ParameterControl {
    pub(crate) fn new(spec: &ParameterControlSpec) -> Self {
        let view = View::new();
        let name = Label::new();
        name.set_text(&spec.label);
        name.set_font(Font::system(12.0));
        let value_label = Label::new();
        value_label.set_font(Font::system(11.0));
        value_label.set_text(format_value(spec.initial, spec.kind));

        let mut target = Box::new(ParameterTarget {
            key: spec.key.clone(),
            kind: spec.kind,
        });
        let slider = unsafe {
            let slider: id = msg_send![slider_class(),
                sliderWithValue: spec.initial
                minValue: spec.minimum
                maxValue: spec.maximum
                target: nil
                action: nil
            ];
            let _: () = msg_send![slider, setContinuous: YES];
            let _: () = msg_send![slider, setTranslatesAutoresizingMaskIntoConstraints: NO];
            let _: () = msg_send![slider, setTarget: slider];
            let _: () = msg_send![slider, setAction: sel!(parameterChanged:)];
            let target_pointer = (&mut *target as *mut ParameterTarget) as usize;
            (&mut *slider).set_ivar(TARGET_IVAR, target_pointer);
            slider
        };

        view.add_subview(&name);
        view.add_subview(&value_label);
        view.objc.with_mut(|container| unsafe {
            let _: () = msg_send![container, addSubview: slider];
        });
        LayoutConstraint::activate(&[
            name.top.constraint_equal_to(&view.top),
            name.leading.constraint_equal_to(&view.leading),
            value_label.top.constraint_equal_to(&view.top),
            value_label.trailing.constraint_equal_to(&view.trailing),
        ]);
        pin_slider(slider, &view);

        Self {
            view,
            key: spec.key.clone(),
            _name: name,
            value_label,
            _slider: ObjcProperty::retain(slider),
            _target: target,
        }
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) fn set_value(&self, value: f64, kind: ParameterControlKind) {
        self.value_label.set_text(format_value(value, kind));
    }
}

fn pin_slider(slider: id, parent: &View) {
    parent.objc.get(|parent| unsafe {
        let slider_leading: id = msg_send![slider, leadingAnchor];
        let parent_leading: id = msg_send![parent, leadingAnchor];
        let slider_trailing: id = msg_send![slider, trailingAnchor];
        let parent_trailing: id = msg_send![parent, trailingAnchor];
        let slider_bottom: id = msg_send![slider, bottomAnchor];
        let parent_bottom: id = msg_send![parent, bottomAnchor];
        let constraints = NSArray::new(&[
            msg_send![slider_leading, constraintEqualToAnchor: parent_leading],
            msg_send![slider_trailing, constraintEqualToAnchor: parent_trailing],
            msg_send![slider_bottom, constraintEqualToAnchor: parent_bottom],
        ]);
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    });
}

fn slider_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let mut declaration = ClassDecl::new("CharmeMaterialParameterSlider", class!(NSSlider))
            .expect("parameter slider class should only be registered once");
        declaration.add_ivar::<usize>(TARGET_IVAR);
        declaration.add_method(
            sel!(parameterChanged:),
            parameter_changed as extern "C" fn(&Object, Sel, id),
        );
        declaration.register() as *const Class as usize
    }) as *const Class
}

extern "C" fn parameter_changed(control: &Object, _: Sel, sender: id) {
    let target_pointer = unsafe { *control.get_ivar::<usize>(TARGET_IVAR) };
    let Some(target) = (unsafe { (target_pointer as *const ParameterTarget).as_ref() }) else {
        return;
    };
    let raw_value: f64 = unsafe { msg_send![sender, doubleValue] };
    let value = match target.kind {
        ParameterControlKind::Float => raw_value,
        ParameterControlKind::SignedInteger | ParameterControlKind::UnsignedInteger => {
            raw_value.round()
        }
    };
    App::<CharmeApp, Message>::dispatch_main(Message::ParameterChanged {
        key: target.key.clone(),
        value,
        kind: target.kind,
    });
}

fn format_value(value: f64, kind: ParameterControlKind) -> String {
    match kind {
        ParameterControlKind::Float => format!("{value:.3}"),
        ParameterControlKind::SignedInteger | ParameterControlKind::UnsignedInteger => {
            format!("{value:.0}")
        }
    }
}
