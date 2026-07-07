#![cfg(target_os = "macos")]
#![allow(unexpected_cfgs)]

use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicyRegular, NSBackingStoreBuffered, NSMenu,
    NSMenuItem, NSView, NSWindow, NSWindowStyleMask,
};
use cocoa::base::{id, nil, BOOL, NO, YES};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
use fastpad_core::{AppSettings, DocumentManager, EditorMode, OpenIntent};
use fastpad_viewport::{ViewAnchor, ViewportRequest};
use libc::c_char;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::c_void;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Once;

const NS_MODAL_RESPONSE_OK: i64 = 1;
const NS_VIEW_WIDTH_SIZABLE: u64 = 2;
const NS_VIEW_HEIGHT_SIZABLE: u64 = 16;
const NS_VIEW_MIN_Y_MARGIN: u64 = 8;

struct AppState {
    manager: DocumentManager,
    window: id,
    text_view: id,
    status_field: id,
    next_anchor: Option<ViewAnchor>,
}

impl AppState {
    unsafe fn new(window: id, text_view: id, status_field: id) -> Self {
        Self {
            manager: DocumentManager::new(AppSettings::default()),
            window,
            text_view,
            status_field,
            next_anchor: None,
        }
    }

    unsafe fn open_path(&mut self, path: &Path) {
        match self.manager.open(path, OpenIntent::default()) {
            Ok(id) => {
                if let Some(doc) = self.manager.get(id) {
                    let mut doc = doc.write();
                    let mode = doc.mode();
                    let settings = self.manager.settings().clone();
                    match doc.initial_viewport(&settings) {
                        Ok(viewport) => {
                            let text = if mode == EditorMode::Edit {
                                self.next_anchor = None;
                                match doc.full_text_for_editing() {
                                    Ok(text) => text,
                                    Err(error) => {
                                        self.show_error(&format!("Open failed: {error:#}"));
                                        return;
                                    }
                                }
                            } else {
                                self.next_anchor = Some(viewport.next_anchor());
                                viewport.text()
                            };
                            set_text_view(self.text_view, &text, mode == EditorMode::Edit);
                            set_window_title(self.window, &format!("{} - FastPad", doc.title()));
                            set_status(self.status_field, &doc.status_line());
                        }
                        Err(error) => self.show_error(&format!("Open failed: {error:#}")),
                    }
                }
            }
            Err(error) => self.show_error(&format!("Open failed: {error:#}")),
        }
    }

    unsafe fn new_document(&mut self) {
        let id = self.manager.new_untitled();
        if let Some(doc) = self.manager.get(id) {
            let doc = doc.read();
            self.next_anchor = None;
            set_text_view(self.text_view, "", true);
            set_window_title(self.window, "Untitled - FastPad");
            set_status(self.status_field, &doc.status_line());
        }
    }

    unsafe fn page_down(&mut self) {
        let Some(anchor) = self.next_anchor else {
            return;
        };
        let Some(doc) = self.manager.active() else {
            return;
        };
        let settings = self.manager.settings().clone();
        let mut doc = doc.write();
        if doc.mode() == EditorMode::Edit {
            return;
        }
        match doc.viewport(ViewportRequest {
            anchor,
            max_lines: settings.initial_viewport_lines,
            max_bytes: settings.initial_viewport_bytes,
        }) {
            Ok(viewport) => {
                self.next_anchor = Some(viewport.next_anchor());
                set_text_view(self.text_view, &viewport.text(), false);
                set_status(self.status_field, &doc.status_line());
            }
            Err(error) => self.show_error(&format!("Page failed: {error:#}")),
        }
    }

    unsafe fn save_active(&mut self) {
        let Some(doc) = self.manager.active() else {
            return;
        };
        let mut doc = doc.write();
        if doc.mode() != EditorMode::Edit {
            self.show_error("Save is disabled in View/Analysis Mode.");
            return;
        }
        let ui_text = text_view_string(self.text_view);
        match doc.edit_buffer_mut() {
            Ok(buffer) => {
                let len = buffer.len_chars();
                if let Err(error) = buffer.replace(0..len, &ui_text) {
                    self.show_error(&format!("Save failed: {error:#}"));
                    return;
                }
            }
            Err(error) => {
                self.show_error(&format!("Save failed: {error:#}"));
                return;
            }
        }
        match doc.save() {
            Ok(()) => set_status(self.status_field, &doc.status_line()),
            Err(error) => self.show_error(&format!("Save failed: {error:#}")),
        }
    }

    unsafe fn show_not_implemented(&self) {
        set_status(
            self.status_field,
            "This Notepad++-style command is visible for parity but is not implemented yet.",
        );
    }

    unsafe fn show_error(&self, message: &str) {
        set_status(self.status_field, message);
        set_text_view(self.text_view, message, false);
    }
}

fn main() {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyRegular);

        let delegate_class = app_delegate_class();
        let delegate: id = msg_send![delegate_class, new];

        let (window, text_view, status_field) = create_main_window();
        let state = Box::into_raw(Box::new(AppState::new(window, text_view, status_field)));
        (*delegate).set_ivar("state", state as *mut c_void);
        app.setDelegate_(delegate);

        build_menu(app, delegate);
        window.makeKeyAndOrderFront_(nil);
        app.activateIgnoringOtherApps_(YES);

        if let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) {
            (*state).open_path(&path);
        } else {
            set_text_view(
                text_view,
                "FastPad\n\nUse File > Open... to inspect a text file.",
                false,
            );
            set_status(
                status_field,
                "No document open - View/Analysis Mode opens huge files read-only",
            );
        }

        app.run();
    }
}

unsafe fn create_main_window() -> (id, id, id) {
    let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(1080., 720.));
    let style = NSWindowStyleMask::NSTitledWindowMask
        | NSWindowStyleMask::NSClosableWindowMask
        | NSWindowStyleMask::NSMiniaturizableWindowMask
        | NSWindowStyleMask::NSResizableWindowMask;
    let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
        frame,
        style,
        NSBackingStoreBuffered,
        NO,
    );
    window.center();
    set_window_title(window, "FastPad");

    let content: id = window.contentView();
    let bounds: NSRect = msg_send![content, bounds];
    let status_height = 28.;
    let scroll_frame = NSRect::new(
        NSPoint::new(0., status_height),
        NSSize::new(bounds.size.width, bounds.size.height - status_height),
    );
    let status_frame = NSRect::new(
        NSPoint::new(10., 4.),
        NSSize::new(bounds.size.width - 20., status_height - 8.),
    );

    let scroll: id = msg_send![class!(NSScrollView), alloc];
    let scroll: id = msg_send![scroll, initWithFrame: scroll_frame];
    let _: () = msg_send![scroll, setHasVerticalScroller: YES];
    let _: () = msg_send![scroll, setHasHorizontalScroller: YES];
    let _: () =
        msg_send![scroll, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE];

    let text_view: id = msg_send![class!(NSTextView), alloc];
    let text_view: id = msg_send![text_view, initWithFrame: scroll_frame];
    let _: () = msg_send![text_view, setMinSize: NSSize::new(0., 0.)];
    let _: () = msg_send![text_view, setMaxSize: NSSize::new(f64::MAX, f64::MAX)];
    let _: () = msg_send![text_view, setVerticallyResizable: YES];
    let _: () = msg_send![text_view, setHorizontallyResizable: YES];
    let _: () =
        msg_send![text_view, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE];
    let font: id = msg_send![class!(NSFont), userFixedPitchFontOfSize: 13.0f64];
    let _: () = msg_send![text_view, setFont: font];
    let _: () = msg_send![scroll, setDocumentView: text_view];
    content.addSubview_(scroll);

    let status_field: id = msg_send![class!(NSTextField), alloc];
    let status_field: id = msg_send![status_field, initWithFrame: status_frame];
    let _: () = msg_send![status_field, setEditable: NO];
    let _: () = msg_send![status_field, setSelectable: NO];
    let _: () = msg_send![status_field, setBordered: NO];
    let _: () = msg_send![status_field, setDrawsBackground: NO];
    let _: () =
        msg_send![status_field, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_MIN_Y_MARGIN];
    content.addSubview_(status_field);

    (window, text_view, status_field)
}

unsafe fn build_menu(app: id, delegate: id) {
    let menubar = NSMenu::new(nil).autorelease();
    app.setMainMenu_(menubar);

    let app_menu_item = NSMenuItem::new(nil).autorelease();
    menubar.addItem_(app_menu_item);
    let app_menu = NSMenu::new(nil).autorelease();
    app_menu_item.setSubmenu_(app_menu);
    app_menu.addItem_(menu_item("Quit FastPad", "q", sel!(terminate:), nil));

    let file_menu = add_menu(menubar, "File");
    file_menu.addItem_(menu_item("New", "n", sel!(newDocument:), delegate));
    file_menu.addItem_(menu_item("Open...", "o", sel!(openDocument:), delegate));
    file_menu.addItem_(disabled_menu_item("Open Containing Folder"));
    file_menu.addItem_(disabled_menu_item("Open Folder as Workspace"));
    file_menu.addItem_(disabled_menu_item("Reload from Disk"));
    file_menu.addItem_(separator_item());
    file_menu.addItem_(menu_item("Save", "s", sel!(saveDocument:), delegate));
    file_menu.addItem_(disabled_menu_item("Save As..."));
    file_menu.addItem_(disabled_menu_item("Save a Copy As..."));
    file_menu.addItem_(disabled_menu_item("Save All"));
    file_menu.addItem_(disabled_menu_item("Rename..."));
    file_menu.addItem_(separator_item());
    file_menu.addItem_(disabled_menu_item("Close"));
    file_menu.addItem_(disabled_menu_item("Close All"));
    file_menu.addItem_(disabled_menu_item("Close All But Current"));
    file_menu.addItem_(disabled_menu_item("Delete from Disk"));
    file_menu.addItem_(separator_item());
    file_menu.addItem_(disabled_menu_item("Load Session..."));
    file_menu.addItem_(disabled_menu_item("Save Session..."));
    file_menu.addItem_(disabled_menu_item("Print..."));

    let edit_menu = add_menu(menubar, "Edit");
    edit_menu.addItem_(menu_item("Undo", "z", sel!(undo:), nil));
    edit_menu.addItem_(menu_item("Redo", "Z", sel!(redo:), nil));
    edit_menu.addItem_(separator_item());
    edit_menu.addItem_(menu_item("Cut", "x", sel!(cut:), nil));
    edit_menu.addItem_(menu_item("Copy", "c", sel!(copy:), nil));
    edit_menu.addItem_(menu_item("Paste", "v", sel!(paste:), nil));
    edit_menu.addItem_(menu_item("Delete", "", sel!(delete:), nil));
    edit_menu.addItem_(menu_item("Select All", "a", sel!(selectAll:), nil));
    edit_menu.addItem_(separator_item());
    edit_menu.addItem_(disabled_menu_item("Begin/End Select"));
    edit_menu.addItem_(disabled_menu_item("Column Mode"));
    edit_menu.addItem_(disabled_menu_item("Multi-Editing"));
    edit_menu.addItem_(disabled_menu_item("Line Operations"));
    edit_menu.addItem_(disabled_menu_item("Blank Operations"));
    edit_menu.addItem_(disabled_menu_item("Case Conversion"));
    edit_menu.addItem_(disabled_menu_item("Comment/Uncomment"));
    edit_menu.addItem_(disabled_menu_item("Auto-completion"));
    edit_menu.addItem_(disabled_menu_item("Parameter Hint"));

    let search_menu = add_menu(menubar, "Search");
    search_menu.addItem_(placeholder_menu_item("Find...", "f", delegate));
    search_menu.addItem_(placeholder_menu_item("Find Next", "g", delegate));
    search_menu.addItem_(placeholder_menu_item("Find Previous", "G", delegate));
    search_menu.addItem_(placeholder_menu_item("Replace...", "h", delegate));
    search_menu.addItem_(disabled_menu_item("Find in Files..."));
    search_menu.addItem_(disabled_menu_item("Find in Projects..."));
    search_menu.addItem_(disabled_menu_item("Incremental Search"));
    search_menu.addItem_(disabled_menu_item("Mark..."));
    search_menu.addItem_(disabled_menu_item("Bookmark"));
    search_menu.addItem_(disabled_menu_item("Go To..."));
    search_menu.addItem_(disabled_menu_item("Search Results Window"));

    let view_menu = add_menu(menubar, "View");
    view_menu.addItem_(disabled_menu_item("Always on Top"));
    view_menu.addItem_(disabled_menu_item("Word Wrap"));
    view_menu.addItem_(disabled_menu_item("Show Symbol"));
    view_menu.addItem_(disabled_menu_item("Zoom"));
    view_menu.addItem_(separator_item());
    view_menu.addItem_(menu_item(
        "Page Down",
        " ",
        sel!(pageDownDocument:),
        delegate,
    ));
    view_menu.addItem_(disabled_menu_item("Move/Clone Current Document"));
    view_menu.addItem_(disabled_menu_item("Tab Bar"));
    view_menu.addItem_(disabled_menu_item("Status Bar"));
    view_menu.addItem_(disabled_menu_item("Toolbar"));
    view_menu.addItem_(disabled_menu_item("Document Map"));
    view_menu.addItem_(disabled_menu_item("Function List"));
    view_menu.addItem_(disabled_menu_item("Folder as Workspace"));
    view_menu.addItem_(disabled_menu_item("Project Panels"));
    view_menu.addItem_(disabled_menu_item("Monitoring"));

    let encoding_menu = add_menu(menubar, "Encoding");
    for item in [
        "ANSI",
        "UTF-8",
        "UTF-8 BOM",
        "UTF-16 LE",
        "UTF-16 BE",
        "Character Sets",
        "Convert Encoding",
    ] {
        encoding_menu.addItem_(disabled_menu_item(item));
    }

    let language_menu = add_menu(menubar, "Language");
    add_language_items(language_menu);

    let settings_menu = add_menu(menubar, "Settings");
    for item in [
        "Preferences...",
        "Style Configurator...",
        "Shortcut Mapper...",
        "Import...",
        "Export...",
        "Cloud Settings",
    ] {
        settings_menu.addItem_(disabled_menu_item(item));
    }

    let tools_menu = add_menu(menubar, "Tools");
    for item in [
        "Macros",
        "Run Command...",
        "Plugin Admin...",
        "Plugins",
        "MD5",
        "SHA Tools",
        "Compare",
        "XML Tools",
        "JSON Tools",
    ] {
        tools_menu.addItem_(disabled_menu_item(item));
    }

    let window_menu = add_menu(menubar, "Window");
    window_menu.addItem_(disabled_menu_item("New Window"));
    window_menu.addItem_(disabled_menu_item("Next Document"));
    window_menu.addItem_(disabled_menu_item("Previous Document"));

    let help_menu = add_menu(menubar, "Help");
    help_menu.addItem_(placeholder_menu_item("About FastPad", "", delegate));
}

unsafe fn add_menu(menubar: id, title: &str) -> id {
    let menu_item = NSMenuItem::new(nil).autorelease();
    menubar.addItem_(menu_item);
    let menu = NSMenu::alloc(nil)
        .initWithTitle_(ns_string(title))
        .autorelease();
    menu_item.setSubmenu_(menu);
    menu
}

unsafe fn add_language_items(language_menu: id) {
    let languages = [
        "Plain Text",
        "ActionScript",
        "Ada",
        "ASN.1",
        "ASP",
        "Assembly",
        "AutoIt",
        "AviSynth",
        "BaanC",
        "Batch",
        "BlitzBasic",
        "C",
        "C#",
        "C++",
        "Caml",
        "CMake",
        "COBOL",
        "CoffeeScript",
        "Csound",
        "CSS",
        "D",
        "Diff",
        "Dockerfile",
        "Erlang",
        "Forth",
        "Fortran",
        "FreeBasic",
        "Go",
        "GraphQL",
        "Groovy",
        "Haskell",
        "HCL",
        "HTML",
        "INI",
        "Intel HEX",
        "Inno Setup",
        "Java",
        "JavaScript",
        "JSON",
        "JSON5",
        "JSP",
        "Kotlin",
        "LaTeX",
        "Lisp",
        "Lua",
        "Makefile",
        "Markdown",
        "MATLAB",
        "Nim",
        "NSIS",
        "Objective-C",
        "OCaml",
        "Pascal",
        "Perl",
        "PHP",
        "PostScript",
        "PowerShell",
        "Properties",
        "Protocol Buffers",
        "Python",
        "R",
        "Registry",
        "Resource Script",
        "Ruby",
        "Rust",
        "Scala",
        "Scheme",
        "Shell Script",
        "Smalltalk",
        "SPICE",
        "SQL",
        "Swift",
        "Tcl",
        "Terraform",
        "TeX",
        "TOML",
        "TypeScript",
        "Visual Basic",
        "Verilog",
        "VHDL",
        "Vue",
        "XML",
        "YAML",
        "Zig",
        "User Defined Language",
    ];

    for language in languages {
        language_menu.addItem_(disabled_menu_item(language));
    }
}

unsafe fn menu_item(title: &str, key: &str, action: Sel, target: id) -> id {
    let item = NSMenuItem::alloc(nil)
        .initWithTitle_action_keyEquivalent_(ns_string(title), action, ns_string(key))
        .autorelease();
    if target != nil {
        item.setTarget_(target);
    }
    item
}

unsafe fn placeholder_menu_item(title: &str, key: &str, target: id) -> id {
    menu_item(title, key, sel!(showNotImplemented:), target)
}

unsafe fn disabled_menu_item(title: &str) -> id {
    let item = menu_item(title, "", sel!(showNotImplemented:), nil);
    let _: () = msg_send![item, setEnabled: NO];
    item
}

unsafe fn separator_item() -> id {
    msg_send![class!(NSMenuItem), separatorItem]
}

unsafe fn set_text_view(text_view: id, text: &str, editable: bool) {
    let _: () = msg_send![text_view, setString: ns_string(text)];
    let _: () = msg_send![text_view, setEditable: if editable { YES } else { NO }];
}

unsafe fn set_status(status_field: id, text: &str) {
    let _: () = msg_send![status_field, setStringValue: ns_string(text)];
}

unsafe fn set_window_title(window: id, title: &str) {
    window.setTitle_(ns_string(title));
}

unsafe fn text_view_string(text_view: id) -> String {
    let ns_string_obj: id = msg_send![text_view, string];
    nsstring_to_string(ns_string_obj)
}

unsafe fn ns_string(text: &str) -> id {
    NSString::alloc(nil).init_str(text)
}

unsafe fn nsstring_to_string(value: id) -> String {
    if value == nil {
        return String::new();
    }
    let c_string: *const c_char = msg_send![value, UTF8String];
    if c_string.is_null() {
        String::new()
    } else {
        CStr::from_ptr(c_string).to_string_lossy().into_owned()
    }
}

fn app_delegate_class() -> *const Class {
    static REGISTER: Once = Once::new();
    static mut CLASS: *const Class = ptr::null();
    REGISTER.call_once(|| unsafe {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("FastPadAppDelegate", superclass).unwrap();
        decl.add_ivar::<*mut c_void>("state");
        decl.add_method(
            sel!(newDocument:),
            new_document as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(openDocument:),
            open_document as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(saveDocument:),
            save_document as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(pageDownDocument:),
            page_down_document as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(showNotImplemented:),
            show_not_implemented as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(applicationShouldTerminateAfterLastWindowClosed:),
            should_terminate_after_last_window_closed as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        CLASS = decl.register();
    });
    unsafe { CLASS }
}

extern "C" fn new_document(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.new_document();
        }
    }
}

extern "C" fn open_document(this: &Object, _: Sel, _: id) {
    unsafe {
        let Some(state) = state_from_delegate(this) else {
            return;
        };
        let panel: id = msg_send![class!(NSOpenPanel), openPanel];
        let _: () = msg_send![panel, setCanChooseFiles: YES];
        let _: () = msg_send![panel, setCanChooseDirectories: NO];
        let response: i64 = msg_send![panel, runModal];
        if response != NS_MODAL_RESPONSE_OK {
            return;
        }
        let url: id = msg_send![panel, URL];
        let path: id = msg_send![url, path];
        let rust_path = nsstring_to_string(path);
        state.open_path(Path::new(&rust_path));
    }
}

extern "C" fn save_document(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.save_active();
        }
    }
}

extern "C" fn page_down_document(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.page_down();
        }
    }
}

extern "C" fn show_not_implemented(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.show_not_implemented();
        }
    }
}

extern "C" fn should_terminate_after_last_window_closed(_: &Object, _: Sel, _: id) -> BOOL {
    YES
}

unsafe fn state_from_delegate<'a>(delegate: &Object) -> Option<&'a mut AppState> {
    let state: *mut c_void = *delegate.get_ivar("state");
    (state as *mut AppState).as_mut()
}
