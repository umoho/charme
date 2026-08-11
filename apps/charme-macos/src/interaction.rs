use std::sync::OnceLock;

use cacao::{
    appkit::App,
    foundation::{NO, NSArray, YES, id},
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

pub(crate) struct OrbitInputView {
    pub(crate) view: View,
    _input: ObjcProperty,
}

impl OrbitInputView {
    pub(crate) fn new() -> Self {
        let view = View::new();
        let input = unsafe {
            let input: id = msg_send![input_class(), new];
            let _: () = msg_send![input, setTranslatesAutoresizingMaskIntoConstraints: NO];
            input
        };

        view.objc.with_mut(|container| unsafe {
            let _: () = msg_send![container, addSubview: input];
            pin_to_edges(input, container);
        });

        Self {
            view,
            _input: ObjcProperty::retain(input),
        }
    }
}

unsafe fn pin_to_edges(child: id, parent: id) {
    let child_leading: id = unsafe { msg_send![child, leadingAnchor] };
    let parent_leading: id = unsafe { msg_send![parent, leadingAnchor] };
    let child_trailing: id = unsafe { msg_send![child, trailingAnchor] };
    let parent_trailing: id = unsafe { msg_send![parent, trailingAnchor] };
    let child_top: id = unsafe { msg_send![child, topAnchor] };
    let parent_top: id = unsafe { msg_send![parent, topAnchor] };
    let child_bottom: id = unsafe { msg_send![child, bottomAnchor] };
    let parent_bottom: id = unsafe { msg_send![parent, bottomAnchor] };
    let constraints = NSArray::new(&[
        unsafe { msg_send![child_leading, constraintEqualToAnchor: parent_leading] },
        unsafe { msg_send![child_trailing, constraintEqualToAnchor: parent_trailing] },
        unsafe { msg_send![child_top, constraintEqualToAnchor: parent_top] },
        unsafe { msg_send![child_bottom, constraintEqualToAnchor: parent_bottom] },
    ]);
    unsafe {
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    }
}

fn input_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let superclass = class!(NSView);
        let mut declaration = ClassDecl::new("CharmeOrbitInputView", superclass)
            .expect("orbit input class should only be registered once");
        declaration.add_method(
            sel!(mouseDown:),
            mouse_down as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(mouseDragged:),
            mouse_dragged as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(scrollWheel:),
            scroll_wheel as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(acceptsFirstMouse:),
            accepts_first_mouse as extern "C" fn(&Object, Sel, id) -> bool,
        );
        declaration.register() as *const Class as usize
    }) as *const Class
}

extern "C" fn mouse_down(_: &Object, _: Sel, _: id) {}

extern "C" fn mouse_dragged(_: &Object, _: Sel, event: id) {
    let delta_x: f64 = unsafe { msg_send![event, deltaX] };
    let delta_y: f64 = unsafe { msg_send![event, deltaY] };
    App::<CharmeApp, Message>::dispatch_main(Message::Orbit {
        delta_x: -(delta_x as f32) * 0.01,
        delta_y: -(delta_y as f32) * 0.01,
    });
}

extern "C" fn scroll_wheel(_: &Object, _: Sel, event: id) {
    let delta_y: f64 = unsafe { msg_send![event, scrollingDeltaY] };
    App::<CharmeApp, Message>::dispatch_main(Message::Zoom(-(delta_y as f32) * 0.035));
}

extern "C" fn accepts_first_mouse(_: &Object, _: Sel, _: id) -> bool {
    YES
}
