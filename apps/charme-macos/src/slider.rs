use std::sync::OnceLock;

use cacao::{
    appkit::App,
    foundation::{NO, NSArray, YES, id, nil},
    objc::{
        class,
        declare::ClassDecl,
        msg_send,
        runtime::{Class, Object, Sel},
        sel, sel_impl,
    },
    utils::properties::ObjcProperty,
    view::View,
};

use crate::app::{CharmeApp, Message};

pub(crate) struct BrightnessSlider {
    pub(crate) view: View,
    _control: ObjcProperty,
}

impl BrightnessSlider {
    pub(crate) fn new(value: f64) -> Self {
        let view = View::new();
        let slider = unsafe {
            let slider: id = msg_send![slider_class(),
                sliderWithValue: value
                minValue: 0.0f64
                maxValue: 1.0f64
                target: nil
                action: nil
            ];
            let _: () = msg_send![slider, setContinuous: YES];
            let _: () = msg_send![slider, setTranslatesAutoresizingMaskIntoConstraints: NO];
            let _: () = msg_send![slider, setTarget: slider];
            let _: () = msg_send![slider, setAction: sel!(brightnessChanged:)];
            slider
        };

        view.objc.with_mut(|container| unsafe {
            let _: () = msg_send![container, addSubview: slider];
            pin_to_edges(slider, container);
        });

        Self {
            view,
            _control: ObjcProperty::retain(slider),
        }
    }
}

unsafe fn pin_to_edges(child: id, parent: id) {
    let child_leading: id = unsafe { msg_send![child, leadingAnchor] };
    let parent_leading: id = unsafe { msg_send![parent, leadingAnchor] };
    let child_trailing: id = unsafe { msg_send![child, trailingAnchor] };
    let parent_trailing: id = unsafe { msg_send![parent, trailingAnchor] };
    let child_center_y: id = unsafe { msg_send![child, centerYAnchor] };
    let parent_center_y: id = unsafe { msg_send![parent, centerYAnchor] };
    let leading: id = unsafe { msg_send![child_leading, constraintEqualToAnchor: parent_leading] };
    let trailing: id =
        unsafe { msg_send![child_trailing, constraintEqualToAnchor: parent_trailing] };
    let center_y: id =
        unsafe { msg_send![child_center_y, constraintEqualToAnchor: parent_center_y] };
    let constraints = NSArray::new(&[leading, trailing, center_y]);
    unsafe {
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    }
}

fn slider_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let superclass = class!(NSSlider);
        let mut declaration = ClassDecl::new("CharmeBrightnessSlider", superclass)
            .expect("brightness slider class should only be registered once");
        declaration.add_method(
            sel!(brightnessChanged:),
            brightness_changed as extern "C" fn(&Object, Sel, id),
        );
        declaration.register() as *const Class as usize
    }) as *const Class
}

extern "C" fn brightness_changed(_: &Object, _: Sel, sender: id) {
    let value: f64 = unsafe { msg_send![sender, doubleValue] };
    App::<CharmeApp, Message>::dispatch_main(Message::Brightness(value as f32));
}
