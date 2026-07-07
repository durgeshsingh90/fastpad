#![cfg(target_os = "macos")]
#![allow(unexpected_cfgs)]

use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicyRegular, NSBackingStoreBuffered, NSMenu,
    NSMenuItem, NSView, NSWindow, NSWindowStyleMask,
};
use cocoa::base::{id, nil, BOOL, NO, YES};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
use fastpad_core::{AppSettings, DocumentManager, EditorMode, OpenIntent, TabSummary};
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
const NS_ALERT_FIRST_BUTTON_RETURN: i64 = 1000;
const NS_ALERT_SECOND_BUTTON_RETURN: i64 = 1001;
const NS_ALERT_THIRD_BUTTON_RETURN: i64 = 1002;
const NS_TERMINATE_CANCEL: i64 = 0;
const NS_TERMINATE_NOW: i64 = 1;
const NS_VIEW_WIDTH_SIZABLE: u64 = 2;
const NS_VIEW_HEIGHT_SIZABLE: u64 = 16;
const NS_VIEW_MIN_Y_MARGIN: u64 = 8;

struct AppState {
    manager: DocumentManager,
    window: id,
    tab_bar: id,
    text_view: id,
    status_field: id,
    last_presented_text: String,
}

impl AppState {
    unsafe fn new(window: id, tab_bar: id, text_view: id, status_field: id) -> Self {
        Self {
            manager: DocumentManager::new(AppSettings::default()),
            window,
            tab_bar,
            text_view,
            status_field,
            last_presented_text: String::new(),
        }
    }

    unsafe fn present_text(&mut self, text: String, editable: bool) {
        set_text_view(self.text_view, &text, editable);
        self.last_presented_text = text;
    }

    unsafe fn open_path(&mut self, path: &Path) {
        match self.manager.open_tab(path, OpenIntent::default()) {
            Ok(_) => self.render_active_tab(),
            Err(error) => self.show_error(&format!("Open failed: {error:#}")),
        }
    }

    unsafe fn open_paths<I>(&mut self, paths: I)
    where
        I: IntoIterator,
        I::Item: AsRef<Path>,
    {
        for path in paths {
            self.open_path(path.as_ref());
        }
    }

    unsafe fn new_document(&mut self) {
        self.manager.new_untitled_tab();
        self.render_active_tab();
    }

    unsafe fn render_active_tab(&mut self) {
        let Some(doc) = self.manager.active() else {
            self.present_text(String::new(), false);
            set_window_title(self.window, "FastPad");
            set_status(self.status_field, "No document open");
            self.refresh_tab_bar();
            return;
        };

        let settings = self.manager.settings().clone();
        let view = self.manager.active_view_state().unwrap_or_default();
        let mut next_anchor = None;
        let mut rendered_anchor = view.anchor;
        let mut doc = doc.write();
        let mode = doc.mode();
        let title = doc.title().to_string();
        let status = doc.status_line();

        let text = if mode == EditorMode::Edit {
            match doc.full_text_for_editing() {
                Ok(text) => text,
                Err(error) => {
                    self.show_error(&format!("Render failed: {error:#}"));
                    return;
                }
            }
        } else {
            match doc.viewport(ViewportRequest {
                anchor: view.anchor,
                max_lines: settings.initial_viewport_lines,
                max_bytes: settings.initial_viewport_bytes,
            }) {
                Ok(viewport) => {
                    rendered_anchor = ViewAnchor::Byte(viewport.start);
                    next_anchor = Some(viewport.next_anchor());
                    viewport.text()
                }
                Err(error) => {
                    self.show_error(&format!("Render failed: {error:#}"));
                    return;
                }
            }
        };
        drop(doc);

        if mode == EditorMode::ViewAnalysis {
            self.manager.update_active_view_state(|view| {
                view.anchor = rendered_anchor;
                view.next_anchor = next_anchor;
            });
        }

        self.present_text(text, mode == EditorMode::Edit);
        set_window_title(self.window, &format!("{title} - FastPad"));
        set_status(self.status_field, &status);
        self.refresh_tab_bar();
    }

    unsafe fn page_down(&mut self) {
        let view = self.manager.active_view_state().unwrap_or_default();
        let Some(anchor) = view.next_anchor else {
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
                let rendered_anchor = ViewAnchor::Byte(viewport.start);
                let next_anchor = viewport.next_anchor();
                self.present_text(viewport.text(), false);
                set_status(self.status_field, &doc.status_line());
                drop(doc);
                self.manager.update_active_view_state(|view| {
                    view.anchor = rendered_anchor;
                    view.next_anchor = Some(next_anchor);
                });
            }
            Err(error) => self.show_error(&format!("Page failed: {error:#}")),
        }
    }

    unsafe fn sync_active_edit_buffer(&mut self) -> bool {
        let Some(doc) = self.manager.active() else {
            return true;
        };
        let mut doc = doc.write();
        if doc.mode() != EditorMode::Edit {
            return true;
        }
        let ui_text = text_view_string(self.text_view);
        if let Err(error) = doc.set_edit_text(&ui_text) {
            self.show_error(&format!("Sync failed: {error:#}"));
            return false;
        }
        true
    }

    unsafe fn active_document_has_unsaved_changes(&self) -> bool {
        let Some(doc) = self.manager.active() else {
            return self.manager.has_dirty_documents();
        };
        let doc = doc.read();
        if doc.mode() != EditorMode::Edit {
            drop(doc);
            return self.manager.has_dirty_documents();
        }
        let active_dirty =
            doc.is_dirty() || text_view_string(self.text_view) != self.last_presented_text;
        drop(doc);
        active_dirty || self.manager.has_dirty_documents()
    }

    unsafe fn save_active(&mut self) -> bool {
        let Some(doc) = self.manager.active() else {
            return true;
        };
        {
            let doc = doc.read();
            if doc.mode() != EditorMode::Edit {
                self.show_error("Save is disabled in View/Analysis Mode.");
                return false;
            }
        }

        if !self.sync_active_edit_buffer() {
            return false;
        }

        let needs_save_as = {
            let doc = doc.read();
            !doc.has_save_path()
        };
        if needs_save_as {
            return self.save_active_as();
        }

        let mut doc = doc.write();
        match doc.save() {
            Ok(()) => {
                self.last_presented_text = text_view_string(self.text_view);
                set_status(self.status_field, &doc.status_line());
                drop(doc);
                self.refresh_tab_bar();
                true
            }
            Err(error) => {
                self.show_error(&format!("Save failed: {error:#}"));
                false
            }
        }
    }

    unsafe fn save_active_as(&mut self) -> bool {
        let Some(doc) = self.manager.active() else {
            return true;
        };
        {
            let doc = doc.read();
            if doc.mode() != EditorMode::Edit {
                self.show_error("Save As is disabled in View/Analysis Mode.");
                return false;
            }
        }

        if !self.sync_active_edit_buffer() {
            return false;
        }

        let Some(path) = save_panel_path("Save As", doc.read().title()) else {
            return false;
        };

        let mut doc = doc.write();
        match doc.save_as(&path) {
            Ok(()) => {
                self.last_presented_text = text_view_string(self.text_view);
                set_window_title(self.window, &format!("{} - FastPad", doc.title()));
                set_status(self.status_field, &doc.status_line());
                drop(doc);
                self.refresh_tab_bar();
                true
            }
            Err(error) => {
                self.show_error(&format!("Save As failed: {error:#}"));
                false
            }
        }
    }

    unsafe fn activate_next_tab(&mut self) {
        if !self.sync_active_edit_buffer() {
            return;
        }
        if self.manager.activate_next_tab() {
            self.render_active_tab();
        }
    }

    unsafe fn activate_previous_tab(&mut self) {
        if !self.sync_active_edit_buffer() {
            return;
        }
        if self.manager.activate_previous_tab() {
            self.render_active_tab();
        }
    }

    unsafe fn duplicate_active_tab(&mut self) {
        if !self.sync_active_edit_buffer() {
            return;
        }
        if self.manager.duplicate_active_tab().is_some() {
            self.render_active_tab();
        }
    }

    unsafe fn toggle_pin_active_tab(&mut self) {
        self.manager.toggle_active_tab_pin();
        self.refresh_tab_bar();
    }

    unsafe fn refresh_tab_bar(&self) {
        set_tab_bar(self.tab_bar, &tab_bar_text(&self.manager.tab_summaries()));
    }

    unsafe fn save_copy_as(&mut self) -> bool {
        let Some(doc) = self.manager.active() else {
            return true;
        };
        {
            let doc = doc.read();
            if doc.mode() != EditorMode::Edit {
                self.show_error("Save a Copy As is disabled in View/Analysis Mode.");
                return false;
            }
        }

        if !self.sync_active_edit_buffer() {
            return false;
        }

        let Some(path) = save_panel_path("Save a Copy As", doc.read().title()) else {
            return false;
        };

        let doc = doc.read();
        match doc.save_copy_as(&path) {
            Ok(()) => {
                set_status(self.status_field, "Saved copy.");
                true
            }
            Err(error) => {
                self.show_error(&format!("Save copy failed: {error:#}"));
                false
            }
        }
    }

    unsafe fn confirm_terminate(&mut self) -> bool {
        if !self.active_document_has_unsaved_changes() {
            return true;
        }

        let alert: id = msg_send![class!(NSAlert), new];
        let _: () = msg_send![alert, setMessageText: ns_string("Save changes before quitting?")];
        let _: () = msg_send![
            alert,
            setInformativeText: ns_string("One or more open tabs have unsaved changes.")
        ];
        let _: id = msg_send![alert, addButtonWithTitle: ns_string("Save")];
        let _: id = msg_send![alert, addButtonWithTitle: ns_string("Cancel")];
        let _: id = msg_send![alert, addButtonWithTitle: ns_string("Quit Without Saving")];
        let response: i64 = msg_send![alert, runModal];

        match response {
            NS_ALERT_FIRST_BUTTON_RETURN => self.save_active(),
            NS_ALERT_SECOND_BUTTON_RETURN => false,
            NS_ALERT_THIRD_BUTTON_RETURN => true,
            _ => false,
        }
    }

    unsafe fn show_not_implemented(&self) {
        set_status(
            self.status_field,
            "This Notepad++-style command is visible for parity but is not implemented yet.",
        );
    }

    unsafe fn show_error(&mut self, message: &str) {
        set_status(self.status_field, message);
        self.present_text(message.to_string(), false);
    }
}

fn main() {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyRegular);

        let delegate_class = app_delegate_class();
        let delegate: id = msg_send![delegate_class, new];

        let (window, tab_bar, text_view, status_field) = create_main_window();
        let state = Box::into_raw(Box::new(AppState::new(
            window,
            tab_bar,
            text_view,
            status_field,
        )));
        (*delegate).set_ivar("state", state as *mut c_void);
        app.setDelegate_(delegate);

        build_menu(app, delegate);
        window.makeKeyAndOrderFront_(nil);
        app.activateIgnoringOtherApps_(YES);

        let paths = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if paths.is_empty() {
            set_text_view(
                text_view,
                "FastPad\n\nUse File > Open... to inspect a text file.",
                false,
            );
            set_status(
                status_field,
                "No document open - View/Analysis Mode opens huge files read-only",
            );
            set_tab_bar(tab_bar, "No tabs");
        } else {
            (*state).open_paths(paths);
        }

        app.run();
    }
}

unsafe fn create_main_window() -> (id, id, id, id) {
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
    let tab_height = 30.;
    let status_height = 28.;
    let tab_frame = NSRect::new(
        NSPoint::new(10., bounds.size.height - tab_height + 4.),
        NSSize::new(bounds.size.width - 20., tab_height - 8.),
    );
    let scroll_frame = NSRect::new(
        NSPoint::new(0., status_height),
        NSSize::new(
            bounds.size.width,
            bounds.size.height - status_height - tab_height,
        ),
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

    let tab_bar: id = msg_send![class!(NSTextField), alloc];
    let tab_bar: id = msg_send![tab_bar, initWithFrame: tab_frame];
    let _: () = msg_send![tab_bar, setEditable: NO];
    let _: () = msg_send![tab_bar, setSelectable: NO];
    let _: () = msg_send![tab_bar, setBordered: NO];
    let _: () = msg_send![tab_bar, setDrawsBackground: YES];
    let tab_font: id = msg_send![class!(NSFont), systemFontOfSize: 12.0f64];
    let _: () = msg_send![tab_bar, setFont: tab_font];
    let _: () =
        msg_send![tab_bar, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_MIN_Y_MARGIN];
    content.addSubview_(tab_bar);

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

    (window, tab_bar, text_view, status_field)
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
    file_menu.addItem_(menu_item(
        "Save As...",
        "S",
        sel!(saveDocumentAs:),
        delegate,
    ));
    file_menu.addItem_(menu_item(
        "Save a Copy As...",
        "",
        sel!(saveCopyAs:),
        delegate,
    ));
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
    file_menu.addItem_(separator_item());
    file_menu.addItem_(menu_item("Exit", "", sel!(terminate:), nil));

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
    search_menu.addItem_(find_menu_item("Find...", "f", 1));
    search_menu.addItem_(find_menu_item("Find Next", "g", 2));
    search_menu.addItem_(find_menu_item("Find Previous", "G", 3));
    search_menu.addItem_(disabled_menu_item("Replace..."));
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
        "SHA tools via plugins",
        "Compare via plugin",
        "XML tools via plugin",
        "JSON tools via plugin",
    ] {
        tools_menu.addItem_(disabled_menu_item(item));
    }

    let macro_menu = add_menu(menubar, "Macro");
    for item in [
        "Start Recording",
        "Stop Recording",
        "Playback",
        "Save Current Recorded Macro...",
        "Run a Macro Multiple Times...",
        "Modify Shortcut/Delete Macro...",
    ] {
        macro_menu.addItem_(disabled_menu_item(item));
    }

    let run_menu = add_menu(menubar, "Run");
    for item in [
        "Run...",
        "Launch in Browser",
        "Get PHP Help",
        "Wikipedia Search",
    ] {
        run_menu.addItem_(disabled_menu_item(item));
    }

    let plugins_menu = add_menu(menubar, "Plugins");
    for item in [
        "Plugin Admin...",
        "Open Plugins Folder",
        "MIME Tools",
        "Converter",
        "NppExport",
        "Compare",
        "XML Tools",
        "JSON Tools",
    ] {
        plugins_menu.addItem_(disabled_menu_item(item));
    }

    let tab_menu = add_menu(menubar, "Tab");
    tab_menu.addItem_(menu_item("Next Tab", "]", sel!(nextTab:), delegate));
    tab_menu.addItem_(menu_item("Previous Tab", "[", sel!(previousTab:), delegate));
    tab_menu.addItem_(separator_item());
    tab_menu.addItem_(menu_item(
        "Duplicate Tab",
        "",
        sel!(duplicateTab:),
        delegate,
    ));
    tab_menu.addItem_(menu_item(
        "Pin/Unpin Tab",
        "",
        sel!(togglePinTab:),
        delegate,
    ));
    tab_menu.addItem_(separator_item());
    for item in [
        "Close Current Tab",
        "Close All Tabs",
        "Close Other Tabs",
        "Close Tabs to the Right",
        "Reopen Recently Closed Tab",
        "Move Tab to New Window",
        "Split Tab Vertically",
        "Split Tab Horizontally",
        "Clone View of Same Document",
        "Preview Tab",
        "Tab Search",
    ] {
        tab_menu.addItem_(disabled_menu_item(item));
    }

    let window_menu = add_menu(menubar, "Window");
    window_menu.addItem_(disabled_menu_item("New Window"));
    window_menu.addItem_(menu_item("Next Document", "", sel!(nextTab:), delegate));
    window_menu.addItem_(menu_item(
        "Previous Document",
        "",
        sel!(previousTab:),
        delegate,
    ));

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

unsafe fn find_menu_item(title: &str, key: &str, tag: i64) -> id {
    let item = menu_item(title, key, sel!(performFindPanelAction:), nil);
    let _: () = msg_send![item, setTag: tag];
    item
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

unsafe fn set_tab_bar(tab_bar: id, text: &str) {
    let _: () = msg_send![tab_bar, setStringValue: ns_string(text)];
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

unsafe fn save_panel_path(title: &str, default_name: &str) -> Option<PathBuf> {
    let panel: id = msg_send![class!(NSSavePanel), savePanel];
    let _: () = msg_send![panel, setTitle: ns_string(title)];
    let _: () = msg_send![panel, setNameFieldStringValue: ns_string(default_name)];
    let response: i64 = msg_send![panel, runModal];
    if response != NS_MODAL_RESPONSE_OK {
        return None;
    }
    let url: id = msg_send![panel, URL];
    let path: id = msg_send![url, path];
    Some(PathBuf::from(nsstring_to_string(path)))
}

unsafe fn paths_from_url_array(urls: id) -> Vec<PathBuf> {
    let count: usize = msg_send![urls, count];
    let mut paths = Vec::with_capacity(count);
    for idx in 0..count {
        let url: id = msg_send![urls, objectAtIndex: idx];
        let path: id = msg_send![url, path];
        paths.push(PathBuf::from(nsstring_to_string(path)));
    }
    paths
}

unsafe fn paths_from_nsstring_array(values: id) -> Vec<PathBuf> {
    let count: usize = msg_send![values, count];
    let mut paths = Vec::with_capacity(count);
    for idx in 0..count {
        let path: id = msg_send![values, objectAtIndex: idx];
        paths.push(PathBuf::from(nsstring_to_string(path)));
    }
    paths
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

fn tab_bar_text(tabs: &[TabSummary]) -> String {
    if tabs.is_empty() {
        return "No tabs".to_string();
    }
    tabs.iter().map(tab_label).collect::<Vec<_>>().join("  |  ")
}

fn tab_label(tab: &TabSummary) -> String {
    let icon = if tab.pinned { "📌" } else { "📄" };
    let mut flags = String::new();
    if tab.view_analysis {
        flags.push_str(" 👁");
    }
    if tab.dirty {
        flags.push_str(" *");
    }
    if tab.read_only {
        flags.push_str(" RO");
    }
    if tab.external_modified {
        flags.push_str(" !");
    }
    let label = format!("{icon} {}{flags}", tab.title);
    if tab.active {
        format!("[{label}]")
    } else {
        label
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
            sel!(saveDocumentAs:),
            save_document_as as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(saveCopyAs:),
            save_copy_as as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(pageDownDocument:),
            page_down_document as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(sel!(nextTab:), next_tab as extern "C" fn(&Object, Sel, id));
        decl.add_method(
            sel!(previousTab:),
            previous_tab as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(duplicateTab:),
            duplicate_tab as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(togglePinTab:),
            toggle_pin_tab as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(showNotImplemented:),
            show_not_implemented as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(applicationShouldTerminate:),
            application_should_terminate as extern "C" fn(&Object, Sel, id) -> i64,
        );
        decl.add_method(
            sel!(applicationShouldTerminateAfterLastWindowClosed:),
            should_terminate_after_last_window_closed as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        decl.add_method(
            sel!(application:openFiles:),
            application_open_files as extern "C" fn(&Object, Sel, id, id),
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
        let _: () = msg_send![panel, setAllowsMultipleSelection: YES];
        let response: i64 = msg_send![panel, runModal];
        if response != NS_MODAL_RESPONSE_OK {
            return;
        }
        let urls: id = msg_send![panel, URLs];
        state.open_paths(paths_from_url_array(urls));
    }
}

extern "C" fn save_document(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.save_active();
        }
    }
}

extern "C" fn save_document_as(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.save_active_as();
        }
    }
}

extern "C" fn save_copy_as(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.save_copy_as();
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

extern "C" fn next_tab(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.activate_next_tab();
        }
    }
}

extern "C" fn previous_tab(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.activate_previous_tab();
        }
    }
}

extern "C" fn duplicate_tab(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.duplicate_active_tab();
        }
    }
}

extern "C" fn toggle_pin_tab(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.toggle_pin_active_tab();
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

extern "C" fn application_should_terminate(this: &Object, _: Sel, _: id) -> i64 {
    unsafe {
        let Some(state) = state_from_delegate(this) else {
            return NS_TERMINATE_NOW;
        };
        if state.confirm_terminate() {
            NS_TERMINATE_NOW
        } else {
            NS_TERMINATE_CANCEL
        }
    }
}

extern "C" fn should_terminate_after_last_window_closed(_: &Object, _: Sel, _: id) -> BOOL {
    YES
}

extern "C" fn application_open_files(this: &Object, _: Sel, app: id, files: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.open_paths(paths_from_nsstring_array(files));
        }
        let _: () = msg_send![app, replyToOpenOrPrint: 0i64];
    }
}

unsafe fn state_from_delegate<'a>(delegate: &Object) -> Option<&'a mut AppState> {
    let state: *mut c_void = *delegate.get_ivar("state");
    (state as *mut AppState).as_mut()
}
