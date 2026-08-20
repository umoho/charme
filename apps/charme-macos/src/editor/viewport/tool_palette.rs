//! Viewport tool palette rendered as a vertical strip on the viewport edge.
//!
//! The palette is built from the platform-independent [`ToolRegistry`]
//! presentation model, so registering a new tool automatically adds a button
//! without native view changes.

use cacao::{
    color::Color,
    foundation::{NO, NSArray, NSInteger, NSString, id},
    objc::{class, msg_send, runtime::Sel, sel, sel_impl},
    utils::properties::ObjcProperty,
    view::View,
};
use charme_application::{ToolPaletteEntry, ToolRegistry, ViewportToolId};

use crate::{
    app::menu_target,
    localization::{self, Key},
};

const BUTTON_SIZE: f64 = 26.0;
const STACK_SPACING: f64 = 4.0;

/// Vertical tool button strip pinned to the viewport edge.
pub(crate) struct ToolPalette {
    pub(crate) view: View,
    buttons: Vec<ToolButton>,
}

struct ToolButton {
    id: ViewportToolId,
    button: ObjcProperty,
}

impl ToolPalette {
    pub(crate) fn new(registry: &ToolRegistry) -> Self {
        let view = View::new();
        // Translucent panel so the strip reads as a floating tool rack.
        view.set_background_color(Color::rgba(22, 24, 29, 210));
        view.layer.set_corner_radius(8.0);
        let model = registry.palette_model(ViewportToolId::SelectMaterialSlot);

        let stack = unsafe {
            let stack: id = msg_send![class!(NSStackView), new];
            // NSUserInterfaceLayoutOrientationVertical = 1.
            let _: () = msg_send![stack, setOrientation: 1usize];
            // NSLayoutAttributeCenterX = 9.
            let _: () = msg_send![stack, setAlignment: 9usize];
            let _: () = msg_send![stack, setSpacing: STACK_SPACING];
            let _: () = msg_send![stack, setTranslatesAutoresizingMaskIntoConstraints: NO];
            stack
        };

        let mut buttons = Vec::new();
        for entry in model.entries {
            let button = make_button(&entry);
            unsafe {
                let _: () = msg_send![stack, addArrangedSubview: button];
            }
            buttons.push(ToolButton {
                id: entry.id,
                button: ObjcProperty::from_retained(button),
            });
        }

        view.objc.with_mut(|container| unsafe {
            let _: () = msg_send![container, addSubview: stack];
            pin_center(stack, container);
        });

        let palette = Self { view, buttons };
        palette.set_active(ViewportToolId::SelectMaterialSlot);
        palette
    }

    /// Highlights the active tool button.
    pub(crate) fn set_active(&self, tool: ViewportToolId) {
        for button in &self.buttons {
            let state: NSInteger = if button.id == tool { 1 } else { 0 };
            button.button.get(|button| unsafe {
                let _: () = msg_send![button, setState: state];
            });
        }
    }
}

fn make_button(entry: &ToolPaletteEntry) -> id {
    let description =
        localization::text(Key::from_resource_key(entry.tooltip_key).unwrap_or(Key::AppName));
    unsafe {
        let image = symbol_image(entry.symbol_name, description);
        let action = tool_action(entry.id);
        let button: id = msg_send![class!(NSButton), buttonWithImage: image target: menu_target() action: action];
        // NSButtonTypeToggle = 2, NSImageOnly = 1.
        let _: () = msg_send![button, setButtonType: 2usize];
        let _: () = msg_send![button, setImagePosition: 1usize];
        let _: () = msg_send![button, setBordered: NO];
        let _: () = msg_send![button, setState: if entry.active { 1usize } else { 0usize }];
        let tooltip = NSString::new(description);
        let _: () = msg_send![button, setToolTip: &*tooltip];
        let _: () = msg_send![button, setTranslatesAutoresizingMaskIntoConstraints: NO];
        let width: id = msg_send![button, widthAnchor];
        let height: id = msg_send![button, heightAnchor];
        let width_constraint: id = msg_send![width, constraintEqualToConstant: BUTTON_SIZE];
        let height_constraint: id = msg_send![height, constraintEqualToConstant: BUTTON_SIZE];
        let size_constraints = NSArray::new(&[width_constraint, height_constraint]);
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*size_constraints];
        button
    }
}

fn tool_action(id: ViewportToolId) -> Sel {
    match id {
        ViewportToolId::SelectMaterialSlot => sel!(charmeSelectMaterialSlot:),
        ViewportToolId::SelectPrimitive => sel!(charmeSelectPrimitive:),
    }
}

fn symbol_image(name: &str, description: &str) -> id {
    unsafe {
        let name = NSString::new(name);
        let description = NSString::new(description);
        let symbol: id = msg_send![class!(NSImage), imageWithSystemSymbolName: &*name accessibilityDescription: &*description];
        // NSFontWeightRegular = 0.0.
        let config: id = msg_send![
            class!(NSImageSymbolConfiguration),
            configurationWithPointSize: 15.0
                weight: 0.0
        ];
        msg_send![symbol, imageWithSymbolConfiguration: config]
    }
}

unsafe fn pin_center(child: id, parent: id) {
    let child_center_x: id = unsafe { msg_send![child, centerXAnchor] };
    let parent_center_x: id = unsafe { msg_send![parent, centerXAnchor] };
    let child_center_y: id = unsafe { msg_send![child, centerYAnchor] };
    let parent_center_y: id = unsafe { msg_send![parent, centerYAnchor] };
    let constraints = NSArray::new(&[
        unsafe { msg_send![child_center_x, constraintEqualToAnchor: parent_center_x] },
        unsafe { msg_send![child_center_y, constraintEqualToAnchor: parent_center_y] },
    ]);
    unsafe {
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    }
}
