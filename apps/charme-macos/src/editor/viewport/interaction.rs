use std::sync::OnceLock;

use cacao::{
    appkit::App,
    foundation::{BOOL, NO, NSArray, NSUInteger, YES, id, nil},
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
use charme_renderer::ViewportSelectionAction;
use core_graphics::geometry::CGPoint;

use crate::app::{CharmeApp, EditorMessage, Message};

const DOWN_X_IVAR: &str = "charmeOrbitDownX";
const DOWN_Y_IVAR: &str = "charmeOrbitDownY";
const DID_DRAG_IVAR: &str = "charmeOrbitDidDrag";
const CLICK_DRAG_THRESHOLD: f64 = 3.0;
const SCROLL_ORBIT_SENSITIVITY: f32 = 0.0035;
const SCROLL_ZOOM_SENSITIVITY: f32 = 0.035;
const MAGNIFICATION_ZOOM_SENSITIVITY: f32 = 1.0;
const CONTROL_MODIFIER_FLAG: NSUInteger = 1 << 18;
const OPTION_MODIFIER_FLAG: NSUInteger = 1 << 19;
const COMMAND_MODIFIER_FLAG: NSUInteger = 1 << 20;
// kVK_Tab and kVK_Escape virtual key codes.
const KEY_TAB: u16 = 48;
const KEY_ESCAPE: u16 = 53;

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
            _input: ObjcProperty::from_retained(input),
        }
    }

    /// Returns the underlying input view that accepts key events.
    pub(crate) fn input_view(&self) -> id {
        self._input.get(|input| input as *const Object as id)
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
        declaration.add_ivar::<f64>(DOWN_X_IVAR);
        declaration.add_ivar::<f64>(DOWN_Y_IVAR);
        declaration.add_ivar::<usize>(DID_DRAG_IVAR);
        declaration.add_method(
            sel!(mouseDown:),
            mouse_down as extern "C" fn(&mut Object, Sel, id),
        );
        declaration.add_method(
            sel!(mouseDragged:),
            mouse_dragged as extern "C" fn(&mut Object, Sel, id),
        );
        declaration.add_method(sel!(mouseUp:), mouse_up as extern "C" fn(&Object, Sel, id));
        declaration.add_method(
            sel!(scrollWheel:),
            scroll_wheel as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(magnifyWithEvent:),
            magnify_with_event as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(acceptsFirstMouse:),
            accepts_first_mouse as extern "C" fn(&Object, Sel, id) -> bool,
        );
        declaration.add_method(
            sel!(acceptsFirstResponder),
            accepts_first_responder as extern "C" fn(&Object, Sel) -> bool,
        );
        declaration.add_method(sel!(keyDown:), key_down as extern "C" fn(&Object, Sel, id));
        declaration.register() as *const Class as usize
    }) as *const Class
}

extern "C" fn mouse_down(view: &mut Object, _: Sel, event: id) {
    let window_point: CGPoint = unsafe { msg_send![event, locationInWindow] };
    let point: CGPoint = unsafe { msg_send![view, convertPoint: window_point fromView: nil] };
    unsafe {
        view.set_ivar(DOWN_X_IVAR, point.x);
        view.set_ivar(DOWN_Y_IVAR, point.y);
        view.set_ivar(DID_DRAG_IVAR, 0usize);
    }
}

extern "C" fn mouse_dragged(view: &mut Object, _: Sel, event: id) {
    let window_point: CGPoint = unsafe { msg_send![event, locationInWindow] };
    let point: CGPoint = unsafe { msg_send![view, convertPoint: window_point fromView: nil] };
    unsafe {
        let down_x = *view.get_ivar::<f64>(DOWN_X_IVAR);
        let down_y = *view.get_ivar::<f64>(DOWN_Y_IVAR);
        if (point.x - down_x).powi(2) + (point.y - down_y).powi(2)
            > CLICK_DRAG_THRESHOLD * CLICK_DRAG_THRESHOLD
        {
            view.set_ivar(DID_DRAG_IVAR, 1usize);
        }
    }

    let delta_x: f64 = unsafe { msg_send![event, deltaX] };
    let delta_y: f64 = unsafe { msg_send![event, deltaY] };
    App::<CharmeApp, Message>::dispatch_main(Message::Editor(EditorMessage::Orbit {
        delta_x: -(delta_x as f32) * 0.01,
        delta_y: -(delta_y as f32) * 0.01,
    }));
}

extern "C" fn mouse_up(view: &Object, _: Sel, event: id) {
    let did_drag = unsafe { *view.get_ivar::<usize>(DID_DRAG_IVAR) != 0 };
    if did_drag {
        return;
    }

    let window_point: CGPoint = unsafe { msg_send![event, locationInWindow] };
    let point: CGPoint = unsafe { msg_send![view, convertPoint: window_point fromView: nil] };
    let modifier_flags: NSUInteger = unsafe { msg_send![event, modifierFlags] };
    let selection_action = if modifier_flags & OPTION_MODIFIER_FLAG != 0 {
        ViewportSelectionAction::Remove
    } else if modifier_flags & COMMAND_MODIFIER_FLAG != 0 {
        ViewportSelectionAction::Toggle
    } else {
        ViewportSelectionAction::Replace
    };
    App::<CharmeApp, Message>::dispatch_main(Message::Editor(EditorMessage::ViewportClicked {
        x: point.x,
        y: point.y,
        selection_action,
    }));
}

extern "C" fn scroll_wheel(_: &Object, _: Sel, event: id) {
    let delta_x: f64 = unsafe { msg_send![event, scrollingDeltaX] };
    let delta_y: f64 = unsafe { msg_send![event, scrollingDeltaY] };
    let has_precise_deltas: BOOL = unsafe { msg_send![event, hasPreciseScrollingDeltas] };
    let modifier_flags: NSUInteger = unsafe { msg_send![event, modifierFlags] };

    let Some(action) = scroll_action(delta_x, delta_y, has_precise_deltas == YES, modifier_flags)
    else {
        return;
    };

    match action {
        ScrollAction::Orbit { delta_x, delta_y } => {
            App::<CharmeApp, Message>::dispatch_main(Message::Editor(EditorMessage::Orbit {
                delta_x,
                delta_y,
            }));
        }
        ScrollAction::Zoom(delta) => {
            App::<CharmeApp, Message>::dispatch_main(Message::Editor(EditorMessage::Zoom(delta)));
        }
    }
}

extern "C" fn magnify_with_event(_: &Object, _: Sel, event: id) {
    let magnification: f64 = unsafe { msg_send![event, magnification] };
    if magnification == 0.0 {
        return;
    }
    App::<CharmeApp, Message>::dispatch_main(Message::Editor(EditorMessage::Zoom(
        -(magnification as f32) * MAGNIFICATION_ZOOM_SENSITIVITY,
    )));
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ScrollAction {
    Orbit { delta_x: f32, delta_y: f32 },
    Zoom(f32),
}

fn scroll_action(
    delta_x: f64,
    delta_y: f64,
    has_precise_deltas: bool,
    modifier_flags: NSUInteger,
) -> Option<ScrollAction> {
    if delta_x == 0.0 && delta_y == 0.0 {
        return None;
    }

    let zoom_modifier = modifier_flags & (CONTROL_MODIFIER_FLAG | COMMAND_MODIFIER_FLAG) != 0;
    if has_precise_deltas && !zoom_modifier {
        Some(ScrollAction::Orbit {
            delta_x: -(delta_x as f32) * SCROLL_ORBIT_SENSITIVITY,
            delta_y: -(delta_y as f32) * SCROLL_ORBIT_SENSITIVITY,
        })
    } else {
        Some(ScrollAction::Zoom(
            -(delta_y as f32) * SCROLL_ZOOM_SENSITIVITY,
        ))
    }
}

extern "C" fn accepts_first_mouse(_: &Object, _: Sel, _: id) -> bool {
    YES
}

extern "C" fn accepts_first_responder(_: &Object, _: Sel) -> bool {
    YES
}

extern "C" fn key_down(_: &Object, _: Sel, event: id) {
    let key_code: u16 = unsafe { msg_send![event, keyCode] };
    match key_code {
        KEY_TAB => {
            App::<CharmeApp, Message>::dispatch_main(Message::Editor(
                EditorMessage::CycleViewportTool,
            ));
        }
        KEY_ESCAPE => {
            App::<CharmeApp, Message>::dispatch_main(Message::Editor(
                EditorMessage::ResetViewportTool,
            ));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precise_unmodified_scroll_orbits_on_both_axes() {
        let action = scroll_action(12.0, -6.0, true, 0);
        let Some(ScrollAction::Orbit { delta_x, delta_y }) = action else {
            panic!("precise scroll should orbit without a zoom modifier");
        };
        assert!((delta_x + 0.042).abs() < 1e-6);
        assert!((delta_y - 0.021).abs() < 1e-6);
    }

    #[test]
    fn command_scroll_zoom_takes_priority_over_precise_orbit() {
        let action = scroll_action(12.0, -6.0, true, COMMAND_MODIFIER_FLAG);
        let Some(ScrollAction::Zoom(delta)) = action else {
            panic!("command scroll should zoom");
        };
        assert!((delta - 0.21).abs() < 1e-6);
    }

    #[test]
    fn ordinary_wheel_scroll_keeps_zoom_behavior() {
        let action = scroll_action(12.0, 2.0, false, 0);
        let Some(ScrollAction::Zoom(delta)) = action else {
            panic!("ordinary wheel scroll should zoom");
        };
        assert!((delta + 0.07).abs() < 1e-6);
    }

    #[test]
    fn zero_scroll_is_ignored() {
        assert_eq!(scroll_action(0.0, 0.0, true, 0), None);
    }
}
