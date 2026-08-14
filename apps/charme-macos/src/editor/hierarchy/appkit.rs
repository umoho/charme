use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::OnceLock,
};

use cacao::objc::declare::ClassDecl;
use cacao::objc::runtime::{Class, Object, Sel};
use cacao::{
    foundation::{BOOL, NO, NSArray, NSInteger, NSString, YES, id, nil},
    image::Image,
    objc::{class, msg_send, sel, sel_impl},
    utils::properties::ObjcProperty,
    view::View,
};
use charme_core::MaterialSlotId;

use super::model::HierarchySnapshot;
use crate::app::{Message, dispatch};

const STATE_IVAR: &str = "charmeHierarchyState";
const ITEM_INDEX_IVAR: &str = "charmeHierarchyItemIndex";
const CELL_IDENTIFIER: &str = "CharmeHierarchyCell";

struct NativeNode {
    item: ObjcProperty,
}

struct NativeState {
    snapshot: HierarchySnapshot,
    nodes: Vec<NativeNode>,
    thumbnails: Vec<Option<ObjcProperty>>,
}

impl NativeState {
    fn new(snapshot: HierarchySnapshot) -> Self {
        let nodes = (0..snapshot.nodes.len())
            .map(|index| {
                let item = unsafe {
                    let item: id = msg_send![hierarchy_item_class(), new];
                    (&mut *item).set_ivar(ITEM_INDEX_IVAR, index);
                    item
                };
                NativeNode {
                    item: ObjcProperty::retain(item),
                }
            })
            .collect();
        let thumbnails = (0..snapshot.nodes.len()).map(|_| None).collect();
        Self {
            snapshot,
            nodes,
            thumbnails,
        }
    }

    fn node_index(item: id) -> Option<usize> {
        if item.is_null() {
            return None;
        }
        Some(unsafe { *(&*item).get_ivar::<usize>(ITEM_INDEX_IVAR) })
    }

    fn item(&self, index: usize) -> id {
        self.nodes[index]
            .item
            .get(|item| item as *const Object as id)
    }
}

/// Native `NSOutlineView` embedded in a Cacao view hierarchy.
pub(super) struct NativeHierarchyView {
    pub(super) view: View,
    outline: ObjcProperty,
    _scroll_view: ObjcProperty,
    _delegate: ObjcProperty,
    state: Box<RefCell<NativeState>>,
}

impl NativeHierarchyView {
    pub(super) fn new(snapshot: HierarchySnapshot) -> Self {
        let view = View::new();
        let mut state = Box::new(RefCell::new(NativeState::new(snapshot)));

        let delegate = unsafe {
            let delegate: id = msg_send![hierarchy_delegate_class(), new];
            let state_pointer = (&mut *state as *mut RefCell<NativeState>) as usize;
            (&mut *delegate).set_ivar(STATE_IVAR, state_pointer);
            delegate
        };

        let (scroll_view, outline) = unsafe {
            let outline: id = msg_send![class!(NSOutlineView), new];
            let column_identifier = NSString::new("CharmeHierarchyColumn");
            let column: id = msg_send![class!(NSTableColumn), alloc];
            let column: id = msg_send![column, initWithIdentifier: &*column_identifier];
            let _: () = msg_send![column, setResizingMask: 1usize];
            let _: () = msg_send![outline, addTableColumn: column];
            let _: () = msg_send![outline, setOutlineTableColumn: column];
            let _: () = msg_send![outline, setHeaderView: nil];
            let clear: id = msg_send![class!(NSColor), clearColor];
            let _: () = msg_send![outline, setBackgroundColor: clear];
            let _: () = msg_send![outline, setDataSource: delegate];
            let _: () = msg_send![outline, setDelegate: delegate];
            let _: () = msg_send![outline, setRowHeight: 22.0f64];
            let _: () = msg_send![outline, setIndentationPerLevel: 14.0f64];
            let _: () = msg_send![outline, setAllowsMultipleSelection: NO];
            let _: () = msg_send![outline, setAllowsEmptySelection: YES];
            // Do not let AppKit's automatic style switch between inset and
            // full-width selection as the outline gains or loses child rows.
            let _: () = msg_send![outline, setStyle: 1isize];
            let _: () = msg_send![outline, setSelectionHighlightStyle: 0isize];
            let _: () = msg_send![outline, setColumnAutoresizingStyle: 1usize];
            let _: () = msg_send![outline, setAutoresizingMask: 2usize];
            let _: () = msg_send![column, release];

            let scroll_view: id = msg_send![class!(NSScrollView), new];
            let _: () = msg_send![scroll_view, setTranslatesAutoresizingMaskIntoConstraints: NO];
            let _: () = msg_send![scroll_view, setHasVerticalScroller: YES];
            let _: () = msg_send![scroll_view, setAutohidesScrollers: YES];
            let _: () = msg_send![scroll_view, setBorderType: 0usize];
            let _: () = msg_send![scroll_view, setDrawsBackground: NO];
            let _: () = msg_send![scroll_view, setDocumentView: outline];
            (scroll_view, outline)
        };

        view.objc.with_mut(|container| unsafe {
            let _: () = msg_send![container, addSubview: scroll_view];
            pin_to_edges(scroll_view, container);
        });

        let hierarchy = Self {
            view,
            outline: ObjcProperty::retain(outline),
            _scroll_view: ObjcProperty::retain(scroll_view),
            _delegate: ObjcProperty::retain(delegate),
            state,
        };
        hierarchy.reload_and_expand();
        hierarchy
    }

    pub(super) fn set_snapshot(&self, snapshot: HierarchySnapshot) {
        // Keep old item objects alive until NSOutlineView has discarded its row cache.
        let old_state = self.state.replace(NativeState::new(snapshot));
        self.reload_and_expand();
        drop(old_state);
    }

    pub(super) fn set_thumbnail(&self, slot_id: MaterialSlotId, image: &Image) {
        let image_id = (&*image.0 as *const Object) as id;
        let item = {
            let mut state = self.state.borrow_mut();
            let Some((node_index, _)) =
                state.snapshot.nodes.iter().enumerate().find(|(_, node)| {
                    node.id == super::model::HierarchyItemId::MaterialSlot(slot_id)
                })
            else {
                return;
            };
            state.thumbnails[node_index] = Some(ObjcProperty::retain(image_id));
            state.item(node_index)
        };
        let outline = self.outline.get(|outline| outline as *const Object as id);
        unsafe {
            let _: () = msg_send![outline, reloadItem: item reloadChildren: NO];
        }
    }

    fn reload_and_expand(&self) {
        let outline = self.outline.get(|outline| outline as *const Object as id);
        unsafe {
            let _: () = msg_send![outline, deselectAll: nil];
            let _: () = msg_send![outline, reloadData];
        }

        let expandable_items = {
            let state = self.state.borrow();
            state
                .snapshot
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| !node.children.is_empty())
                .map(|(index, _)| state.item(index) as usize)
                .collect::<Vec<_>>()
        };
        for item in expandable_items {
            unsafe {
                let _: () = msg_send![outline, expandItem: item as id];
            }
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

unsafe fn layout_cell(image_view: id, text_field: id, cell: id) {
    let image_leading: id = unsafe { msg_send![image_view, leadingAnchor] };
    let cell_leading: id = unsafe { msg_send![cell, leadingAnchor] };
    let image_trailing: id = unsafe { msg_send![image_view, trailingAnchor] };
    let image_center_y: id = unsafe { msg_send![image_view, centerYAnchor] };
    let cell_center_y: id = unsafe { msg_send![cell, centerYAnchor] };
    let text_leading: id = unsafe { msg_send![text_field, leadingAnchor] };
    let text_trailing: id = unsafe { msg_send![text_field, trailingAnchor] };
    let cell_trailing: id = unsafe { msg_send![cell, trailingAnchor] };
    let text_center_y: id = unsafe { msg_send![text_field, centerYAnchor] };
    let constraints = NSArray::new(&[
        unsafe { msg_send![image_leading, constraintEqualToAnchor: cell_leading] },
        unsafe { msg_send![image_center_y, constraintEqualToAnchor: cell_center_y] },
        unsafe {
            let anchor: id = msg_send![image_view, widthAnchor];
            msg_send![anchor, constraintEqualToConstant: 18.0f64]
        },
        unsafe {
            let anchor: id = msg_send![image_view, heightAnchor];
            msg_send![anchor, constraintEqualToConstant: 18.0f64]
        },
        unsafe {
            msg_send![text_leading, constraintEqualToAnchor: image_trailing constant: 5.0f64]
        },
        unsafe { msg_send![text_trailing, constraintLessThanOrEqualToAnchor: cell_trailing] },
        unsafe { msg_send![text_center_y, constraintEqualToAnchor: cell_center_y] },
    ]);
    unsafe {
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    }
}

fn with_state<R: Clone>(object: &Object, fallback: R, action: impl FnOnce(&NativeState) -> R) -> R {
    catch_unwind(AssertUnwindSafe(|| unsafe {
        let pointer = *object.get_ivar::<usize>(STATE_IVAR);
        let Some(state) = (pointer as *const RefCell<NativeState>).as_ref() else {
            return fallback.clone();
        };
        let state = state.borrow();
        action(&state)
    }))
    .unwrap_or(fallback)
}

extern "C" fn number_of_children(delegate: &Object, _: Sel, _: id, item: id) -> NSInteger {
    with_state(delegate, 0, |state| {
        NativeState::node_index(item)
            .and_then(|index| state.snapshot.nodes.get(index))
            .map_or(state.snapshot.roots.len(), |node| node.children.len()) as NSInteger
    })
}

extern "C" fn child_of_item(
    delegate: &Object,
    _: Sel,
    _: id,
    child_index: NSInteger,
    item: id,
) -> id {
    with_state(delegate, nil, |state| {
        let children = NativeState::node_index(item)
            .and_then(|index| state.snapshot.nodes.get(index))
            .map_or(state.snapshot.roots.as_slice(), |node| {
                node.children.as_slice()
            });
        usize::try_from(child_index)
            .ok()
            .and_then(|index| children.get(index))
            .map_or(nil, |&index| state.item(index))
    })
}

extern "C" fn is_item_expandable(delegate: &Object, _: Sel, _: id, item: id) -> BOOL {
    with_state(delegate, NO, |state| {
        let expandable = NativeState::node_index(item)
            .and_then(|index| state.snapshot.nodes.get(index))
            .is_some_and(|node| !node.children.is_empty());
        if expandable { YES } else { NO }
    })
}

extern "C" fn view_for_item(delegate: &Object, _: Sel, outline: id, _: id, item: id) -> id {
    let item_data = with_state(delegate, None, |state| {
        NativeState::node_index(item)
            .and_then(|index| state.snapshot.nodes.get(index).map(|node| (index, node)))
            .map(|(index, node)| {
                let thumbnail = state.thumbnails[index]
                    .as_ref()
                    .map(|image| image.get(|image| image as *const Object as id));
                (node.title.clone(), thumbnail)
            })
    });
    let Some((title, thumbnail)) = item_data else {
        return nil;
    };

    unsafe {
        let identifier = NSString::new(CELL_IDENTIFIER);
        let mut cell: id = msg_send![outline, makeViewWithIdentifier: &*identifier owner: nil];
        if cell.is_null() {
            cell = msg_send![class!(NSTableCellView), new];
            let _: () = msg_send![cell, setIdentifier: &*identifier];

            let image_view: id = msg_send![class!(NSImageView), new];
            let _: () = msg_send![image_view, setTranslatesAutoresizingMaskIntoConstraints: NO];
            let _: () = msg_send![image_view, setImageScaling: 3usize];
            let _: () = msg_send![cell, addSubview: image_view];
            let _: () = msg_send![cell, setImageView: image_view];

            let text_field: id =
                msg_send![class!(NSTextField), labelWithString: &*NSString::new("")];
            let _: () = msg_send![text_field, setTranslatesAutoresizingMaskIntoConstraints: NO];
            let font: id = msg_send![class!(NSFont), systemFontOfSize: 12.0f64];
            let _: () = msg_send![text_field, setFont: font];
            let _: () = msg_send![text_field, setLineBreakMode: 4usize];
            let _: () = msg_send![cell, addSubview: text_field];
            let _: () = msg_send![cell, setTextField: text_field];
            layout_cell(image_view, text_field, cell);
        }
        let text_field: id = msg_send![cell, textField];
        let image_view: id = msg_send![cell, imageView];
        let _: () = msg_send![image_view, setImage: thumbnail.unwrap_or(nil)];
        let _: () = msg_send![image_view, setHidden: if thumbnail.is_some() { NO } else { YES }];
        let title = NSString::new(&title);
        let _: () = msg_send![text_field, setStringValue: &*title];
        cell
    }
}

extern "C" fn selection_did_change(delegate: &Object, _: Sel, notification: id) {
    let selection = catch_unwind(AssertUnwindSafe(|| unsafe {
        let outline: id = msg_send![notification, object];
        let row: NSInteger = msg_send![outline, selectedRow];
        if row < 0 {
            return None;
        }
        let item: id = msg_send![outline, itemAtRow: row];
        with_state(delegate, None, |state| {
            NativeState::node_index(item)
                .and_then(|index| state.snapshot.nodes.get(index))
                .map(|node| node.id)
        })
    }))
    .ok()
    .flatten();

    if let Some(selection) = selection {
        dispatch(Message::HierarchySelectionChanged(selection));
    }
}

fn hierarchy_delegate_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let mut declaration = ClassDecl::new("CharmeHierarchyDelegate", class!(NSObject))
            .expect("hierarchy delegate class is registered only once");
        declaration.add_ivar::<usize>(STATE_IVAR);
        declaration.add_method(
            sel!(outlineView:numberOfChildrenOfItem:),
            number_of_children as extern "C" fn(&Object, Sel, id, id) -> NSInteger,
        );
        declaration.add_method(
            sel!(outlineView:child:ofItem:),
            child_of_item as extern "C" fn(&Object, Sel, id, NSInteger, id) -> id,
        );
        declaration.add_method(
            sel!(outlineView:isItemExpandable:),
            is_item_expandable as extern "C" fn(&Object, Sel, id, id) -> BOOL,
        );
        declaration.add_method(
            sel!(outlineView:viewForTableColumn:item:),
            view_for_item as extern "C" fn(&Object, Sel, id, id, id) -> id,
        );
        declaration.add_method(
            sel!(outlineViewSelectionDidChange:),
            selection_did_change as extern "C" fn(&Object, Sel, id),
        );
        declaration.register()
    })
}

fn hierarchy_item_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut declaration = ClassDecl::new("CharmeHierarchyItem", class!(NSObject))
            .expect("hierarchy item class is registered only once");
        declaration.add_ivar::<usize>(ITEM_INDEX_IVAR);
        declaration.register()
    })
}
