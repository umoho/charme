use std::{path::PathBuf, sync::OnceLock};

use cacao::objc::declare::ClassDecl;
use cacao::objc::runtime::{Class, Object, Sel};
use cacao::{
    appkit::App,
    foundation::{NO, NSString, YES, id, nil},
    objc::{class, msg_send, sel, sel_impl},
};

use super::{CharmeApp, MenuContext, Message, SelectionLevel, recent_projects};
use crate::localization::{self, Key};

pub(super) fn install_native_menus() {
    unsafe {
        let main_menu: id = msg_send![class!(NSMenu), new];
        add_submenu(
            main_menu,
            localization::text(Key::AppName),
            build_application_menu(),
        );
        add_submenu(
            main_menu,
            localization::text(Key::FileMenu),
            build_file_menu(),
        );
        add_submenu(
            main_menu,
            localization::text(Key::EditMenu),
            build_edit_menu(),
        );
        add_submenu(
            main_menu,
            localization::text(Key::SelectMenu),
            build_select_menu(),
        );
        add_submenu(
            main_menu,
            localization::text(Key::ViewMenu),
            build_view_menu(),
        );
        add_submenu(
            main_menu,
            localization::text(Key::WindowMenu),
            build_window_menu(),
        );
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setMainMenu: main_menu];
    }
}

fn build_application_menu() -> id {
    unsafe {
        let menu = new_menu(localization::text(Key::AppName));
        add_item(
            menu,
            menu_item(
                localization::text(Key::About),
                sel!(orderFrontStandardAboutPanel:),
                "",
                0,
                nil,
            ),
        );
        add_separator(menu);
        let services: id = msg_send![class!(NSApplication), sharedApplication];
        let services_menu: id = msg_send![services, servicesMenu];
        add_item(
            menu,
            submenu_item(localization::text(Key::Services), services_menu),
        );
        add_separator(menu);
        add_item(
            menu,
            menu_item(
                localization::text(Key::HideApp),
                sel!(hide:),
                "h",
                COMMAND,
                nil,
            ),
        );
        add_item(
            menu,
            menu_item(
                localization::text(Key::HideOthers),
                sel!(hide:),
                "h",
                COMMAND | OPTION,
                nil,
            ),
        );
        add_item(
            menu,
            menu_item(
                localization::text(Key::ShowAll),
                sel!(unhideAllApplications:),
                "",
                0,
                nil,
            ),
        );
        add_separator(menu);
        add_item(
            menu,
            menu_item(
                localization::text(Key::Quit),
                sel!(terminate:),
                "q",
                COMMAND,
                nil,
            ),
        );
        menu
    }
}

fn build_file_menu() -> id {
    {
        let menu = new_menu(localization::text(Key::FileMenu));
        let target = menu_target();
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::NewProjectMenu),
                sel!(charmeNewProject:),
                "n",
                COMMAND,
                target,
            ),
        );
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::OpenProjectMenu),
                sel!(charmeChooseProject:),
                "o",
                COMMAND,
                target,
            ),
        );
        add_item(
            menu,
            submenu_item(
                localization::text(Key::RecentProjectsMenu),
                build_recent_menu(),
            ),
        );
        add_item(
            menu,
            submenu_item(localization::text(Key::ImportMenu), build_import_menu()),
        );
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::InspectShaderMenu),
                sel!(charmeChooseShader:),
                "",
                0,
                target,
            ),
        );
        add_separator(menu);
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::SaveProjectMenu),
                sel!(charmeSaveProject:),
                "s",
                COMMAND,
                target,
            ),
        );
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::SaveAsProjectMenu),
                sel!(charmeChooseSaveProject:),
                "s",
                COMMAND | SHIFT,
                target,
            ),
        );
        add_separator(menu);
        add_item(
            menu,
            menu_item(
                localization::text(Key::CloseWindow),
                sel!(performClose:),
                "w",
                COMMAND,
                nil,
            ),
        );
        menu
    }
}

fn build_import_menu() -> id {
    {
        let menu = new_menu(localization::text(Key::ImportMenu));
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::ImportPmxMenu),
                sel!(charmeChoosePmx:),
                "",
                0,
                menu_target(),
            ),
        );
        menu
    }
}

fn build_edit_menu() -> id {
    {
        let menu = new_menu(localization::text(Key::EditMenu));
        let target = menu_target();
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::Undo),
                sel!(charmeUndo:),
                "z",
                COMMAND,
                target,
            ),
        );
        add_item(
            menu,
            menu_item_with_target(
                localization::text(Key::Redo),
                sel!(charmeRedo:),
                "z",
                COMMAND | SHIFT,
                target,
            ),
        );
        add_separator(menu);
        add_item(
            menu,
            menu_item(localization::text(Key::Cut), sel!(cut:), "x", COMMAND, nil),
        );
        add_item(
            menu,
            menu_item(
                localization::text(Key::Copy),
                sel!(copy:),
                "c",
                COMMAND,
                nil,
            ),
        );
        add_item(
            menu,
            menu_item(
                localization::text(Key::Paste),
                sel!(paste:),
                "v",
                COMMAND,
                nil,
            ),
        );
        add_separator(menu);
        add_item(
            menu,
            menu_item(
                localization::text(Key::SelectAll),
                sel!(selectAll:),
                "a",
                COMMAND,
                nil,
            ),
        );
        menu
    }
}

fn build_select_menu() -> id {
    let menu = new_menu(localization::text(Key::SelectMenu));
    let target = menu_target();
    add_item(
        menu,
        submenu_item(
            localization::text(Key::SelectionLevelMenu),
            build_selection_level_menu(),
        ),
    );
    add_separator(menu);
    add_item(
        menu,
        menu_item_with_target(
            localization::text(Key::SelectAll),
            sel!(charmeSelectAll:),
            "",
            0,
            target,
        ),
    );
    add_item(
        menu,
        menu_item_with_target(
            localization::text(Key::DeselectAll),
            sel!(charmeDeselectAll:),
            "",
            0,
            target,
        ),
    );
    add_item(
        menu,
        menu_item_with_target(
            localization::text(Key::InvertSelection),
            sel!(charmeInvertSelection:),
            "",
            0,
            target,
        ),
    );
    menu
}

fn build_selection_level_menu() -> id {
    let menu = new_menu(localization::text(Key::SelectionLevelMenu));
    let target = menu_target();
    add_item(
        menu,
        menu_item_with_target(
            localization::text(Key::MaterialSlotSelectionLevel),
            sel!(charmeSelectMaterialSlot:),
            "",
            0,
            target,
        ),
    );
    add_item(
        menu,
        menu_item_with_target(
            localization::text(Key::PrimitiveSelectionLevel),
            sel!(charmeSelectPrimitive:),
            "",
            0,
            target,
        ),
    );
    menu
}

fn build_view_menu() -> id {
    let menu = new_menu(localization::text(Key::ViewMenu));
    // The canonical title and selector let AppKit replace this item with its
    // native, bundle-localized full-screen command and manage enter/exit state.
    add_item(
        menu,
        menu_item(
            localization::text(Key::EnterFullScreen),
            sel!(toggleFullScreen:),
            "f",
            COMMAND | CONTROL,
            nil,
        ),
    );
    menu
}

fn build_window_menu() -> id {
    {
        let menu = new_menu(localization::text(Key::WindowMenu));
        add_item(
            menu,
            menu_item(
                localization::text(Key::Minimize),
                sel!(performMiniaturize:),
                "m",
                COMMAND,
                nil,
            ),
        );
        add_item(
            menu,
            menu_item(
                localization::text(Key::Zoom),
                sel!(performZoom:),
                "",
                0,
                nil,
            ),
        );
        add_separator(menu);
        add_item(
            menu,
            menu_item(
                localization::text(Key::BringAllToFront),
                sel!(arrangeInFront:),
                "",
                0,
                nil,
            ),
        );
        menu
    }
}

fn new_menu(title: &str) -> id {
    unsafe {
        let title = NSString::new(title);
        let menu: id = msg_send![class!(NSMenu), alloc];
        msg_send![menu, initWithTitle: &*title]
    }
}

fn add_submenu(parent: id, title: &str, submenu: id) {
    unsafe {
        let item = submenu_item(title, submenu);
        let _: () = msg_send![parent, addItem: item];
    }
}

fn submenu_item(title: &str, submenu: id) -> id {
    unsafe {
        let title = NSString::new(title);
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let item: id =
            msg_send![item, initWithTitle: &*title action: nil keyEquivalent: &*NSString::new("")];
        let _: () = msg_send![item, setSubmenu: submenu];
        item
    }
}

fn add_item(menu: id, item: id) {
    unsafe {
        let _: () = msg_send![menu, addItem: item];
    }
}

fn add_separator(menu: id) {
    unsafe {
        let item: id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: item];
    }
}

const COMMAND: usize = 1 << 20;
const OPTION: usize = 1 << 19;
const CONTROL: usize = 1 << 18;
const SHIFT: usize = 1 << 17;

fn menu_item(title: &str, action: Sel, key: &str, modifiers: usize, target: id) -> id {
    menu_item_with_target(title, action, key, modifiers, target)
}

fn menu_item_with_target(title: &str, action: Sel, key: &str, modifiers: usize, target: id) -> id {
    unsafe {
        let title = NSString::new(title);
        let key = NSString::new(key);
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let item: id = msg_send![item, initWithTitle: &*title action: action keyEquivalent: &*key];
        let _: () = msg_send![item, setKeyEquivalentModifierMask: modifiers];
        if !target.is_null() {
            let _: () = msg_send![item, setTarget: target];
        }
        item
    }
}

fn menu_target() -> id {
    static TARGET: OnceLock<usize> = OnceLock::new();
    *TARGET.get_or_init(|| unsafe {
        let target: id = msg_send![menu_target_class(), new];
        target as usize
    }) as id
}

fn menu_target_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let mut declaration = ClassDecl::new("CharmeMenuTarget", class!(NSObject))
            .expect("menu target class is registered only once");
        declaration.add_method(
            sel!(charmeNewProject:),
            menu_new_project as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeChooseProject:),
            menu_choose_project as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeChoosePmx:),
            menu_choose_pmx as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeChooseShader:),
            menu_choose_shader as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeSaveProject:),
            menu_save_project as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeChooseSaveProject:),
            menu_choose_save_project as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeUndo:),
            menu_undo as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeRedo:),
            menu_redo as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeSelectMaterialSlot:),
            menu_select_material_slot as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeSelectPrimitive:),
            menu_select_primitive as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeSelectAll:),
            menu_select_all as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeDeselectAll:),
            menu_deselect_all as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(charmeInvertSelection:),
            menu_invert_selection as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(menuOpenRecent:),
            menu_open_recent as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(sel!(noop:), menu_noop as extern "C" fn(&Object, Sel, id));
        declaration.register()
    })
}

extern "C" fn menu_new_project(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::NewProject);
}
extern "C" fn menu_choose_project(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::ChooseProject);
}
extern "C" fn menu_choose_pmx(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::ChoosePmx);
}
extern "C" fn menu_choose_shader(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::ChooseShader);
}
extern "C" fn menu_save_project(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::SaveProject);
}
extern "C" fn menu_choose_save_project(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::ChooseSaveProject);
}
extern "C" fn menu_undo(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::Undo);
}
extern "C" fn menu_redo(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::Redo);
}
extern "C" fn menu_select_material_slot(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::SelectionLevelChanged(
        SelectionLevel::MaterialSlot,
    ));
}
extern "C" fn menu_select_primitive(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::SelectionLevelChanged(
        SelectionLevel::Primitive,
    ));
}
extern "C" fn menu_select_all(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::SelectAll);
}
extern "C" fn menu_deselect_all(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::DeselectAll);
}
extern "C" fn menu_invert_selection(_: &Object, _: Sel, _: id) {
    App::<CharmeApp, Message>::dispatch_main(Message::InvertSelection);
}
extern "C" fn menu_noop(_: &Object, _: Sel, _: id) {}
extern "C" fn menu_open_recent(_: &Object, _: Sel, sender: id) {
    unsafe {
        let path: id = msg_send![sender, representedObject];
        if !path.is_null() {
            App::<CharmeApp, Message>::dispatch_main(Message::OpenProject(PathBuf::from(
                NSString::retain(path).to_string(),
            )));
        }
    }
}

pub(super) fn refresh_recent_projects_menu() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let main_menu: id = msg_send![app, mainMenu];
        let file_item: id = msg_send![main_menu, itemAtIndex: 1usize];
        let file_menu: id = msg_send![file_item, submenu];
        let recent_item: id = msg_send![file_menu, itemAtIndex: 2usize];
        let submenu = build_recent_menu();
        let _: () = msg_send![recent_item, setSubmenu: submenu];
        let _: () = msg_send![recent_item, setEnabled: YES];
        let _: () = msg_send![submenu, release];
    }
}

fn build_recent_menu() -> id {
    unsafe {
        let projects = recent_projects();
        let menu = new_menu(localization::text(Key::RecentProjectsMenu));
        if projects.is_empty() {
            let item = menu_item(
                localization::text(Key::NoRecentProjects),
                sel!(noop:),
                "",
                0,
                menu_target(),
            );
            let _: () = msg_send![item, setEnabled: NO];
            add_item(menu, item);
        } else {
            for project in projects {
                let name = project
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or(localization::text(Key::ProjectFallback));
                let title = localization::format(
                    Key::RecentProjectTitle,
                    &[("name", &name), ("path", &project.display())],
                );
                let item =
                    menu_item_with_target(&title, sel!(menuOpenRecent:), "", 0, menu_target());
                let path = NSString::new(&project.to_string_lossy());
                let _: () = msg_send![item, setRepresentedObject: &*path];
                add_item(menu, item);
            }
        }
        menu
    }
}

pub(super) fn update_menu_state(
    context: MenuContext,
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
    selection_level: SelectionLevel,
    has_scene: bool,
    has_primitive_selection: bool,
) {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let main_menu: id = msg_send![app, mainMenu];
        if main_menu.is_null() {
            return;
        }
        let file_item: id = msg_send![main_menu, itemAtIndex: 1usize];
        let file: id = msg_send![file_item, submenu];
        let edit_item: id = msg_send![main_menu, itemAtIndex: 2usize];
        let edit: id = msg_send![edit_item, submenu];
        let select_item: id = msg_send![main_menu, itemAtIndex: 3usize];
        let select: id = msg_send![select_item, submenu];
        let editor = context == MenuContext::Editor;
        set_menu_item_state(file, 3, editor, editor);
        set_menu_item_state(file, 4, editor, editor);
        set_menu_item_state(file, 6, editor, editor && dirty);
        set_menu_item_state(file, 7, editor, editor);
        set_menu_item_state(edit, 0, true, can_undo);
        set_menu_item_state(edit, 1, true, can_redo);
        set_menu_item_state(select, 0, true, editor);
        let level_item: id = msg_send![select, itemAtIndex: 0usize];
        let levels: id = msg_send![level_item, submenu];
        set_menu_item_state(levels, 0, true, editor);
        set_menu_item_state(levels, 1, true, editor);
        set_menu_item_checked(
            levels,
            0,
            editor && selection_level == SelectionLevel::MaterialSlot,
        );
        set_menu_item_checked(
            levels,
            1,
            editor && selection_level == SelectionLevel::Primitive,
        );
        let primitive_selection = editor && selection_level == SelectionLevel::Primitive;
        set_menu_item_state(select, 2, true, primitive_selection && has_scene);
        set_menu_item_state(
            select,
            3,
            true,
            primitive_selection && has_primitive_selection,
        );
        set_menu_item_state(select, 4, true, primitive_selection && has_scene);
    }
}

fn set_menu_item_state(menu: id, index: usize, visible: bool, enabled: bool) {
    unsafe {
        if menu.is_null() {
            return;
        }
        let item: id = msg_send![menu, itemAtIndex: index];
        if item.is_null() {
            return;
        }
        let _: () = msg_send![item, setHidden: if visible { NO } else { YES }];
        let _: () = msg_send![item, setEnabled: if enabled { YES } else { NO }];
    }
}

fn set_menu_item_checked(menu: id, index: usize, checked: bool) {
    unsafe {
        if menu.is_null() {
            return;
        }
        let item: id = msg_send![menu, itemAtIndex: index];
        if item.is_null() {
            return;
        }
        let _: () = msg_send![item, setState: if checked { 1isize } else { 0isize }];
    }
}

pub(super) fn ensure_charme_extension(path: String) -> String {
    if path.to_ascii_lowercase().ends_with(".charme") {
        path
    } else {
        format!("{path}.charme")
    }
}

pub(super) fn set_application_menu_name() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let main_menu: id = msg_send![app, mainMenu];
        let app_menu_item: id = msg_send![main_menu, itemAtIndex: 0];
        let title = NSString::new(localization::text(Key::AppName));
        let _: () = msg_send![app_menu_item, setTitle: &*title];
    }
}

pub(super) fn activate_app() {
    App::activate();
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}
