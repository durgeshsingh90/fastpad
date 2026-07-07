#![cfg(target_os = "macos")]
#![allow(unexpected_cfgs)]

use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicyRegular, NSBackingStoreBuffered, NSMenu,
    NSMenuItem, NSView, NSWindow, NSWindowStyleMask,
};
use cocoa::base::{id, nil, BOOL, NO, YES};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
use fastpad_core::{
    AppSettings, Document, DocumentManager, EditorMode, OpenIntent, OpenTabRequest, TabId,
    TabSummary,
};
use fastpad_render::{RenderOptions, RenderPlan, DEFAULT_MAX_COLUMNS, DEFAULT_OVERSCAN_LINES};
use fastpad_tasks::{TaskHandle, TaskProgress};
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
const VIRTUAL_FONT_SIZE: f64 = 13.0;
const VIRTUAL_LINE_HEIGHT: f64 = 18.0;
const VIRTUAL_TOP_PADDING: f64 = 6.0;
const VIRTUAL_GUTTER_PADDING: f64 = 8.0;
const VIRTUAL_GUTTER_CHAR_WIDTH: f64 = 8.0;
const VIRTUAL_TEXT_PADDING: f64 = 10.0;
const VIRTUAL_CHAR_WIDTH: f64 = 8.0;
const TAB_BAR_HEIGHT: f64 = 30.0;
const TAB_ITEM_HEIGHT: f64 = 22.0;
const TAB_ITEM_TOP: f64 = 4.0;
const TAB_ITEM_GAP: f64 = 6.0;
const TAB_ITEM_PADDING_X: f64 = 10.0;
const TAB_ITEM_MIN_WIDTH: f64 = 104.0;
const TAB_ITEM_MAX_WIDTH: f64 = 280.0;
const TAB_ITEM_CHAR_WIDTH: f64 = 8.0;

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSFontAttributeName: id;
    static NSForegroundColorAttributeName: id;
    fn NSRectFill(rect: NSRect);
}

struct AppState {
    manager: DocumentManager,
    window: id,
    tab_bar: id,
    scroll_view: id,
    text_view: id,
    virtual_view: id,
    status_field: id,
    last_presented_text: String,
    open_tasks: Vec<TaskHandle<anyhow::Result<Document>>>,
}

impl AppState {
    unsafe fn new(
        window: id,
        tab_bar: id,
        scroll_view: id,
        text_view: id,
        virtual_view: id,
        status_field: id,
    ) -> Self {
        Self {
            manager: DocumentManager::new(AppSettings::default()),
            window,
            tab_bar,
            scroll_view,
            text_view,
            virtual_view,
            status_field,
            last_presented_text: String::new(),
            open_tasks: Vec::new(),
        }
    }

    unsafe fn present_text(&mut self, text: String, editable: bool) {
        set_scroll_document_view(self.scroll_view, self.text_view);
        set_text_view(self.text_view, &text, editable);
        self.last_presented_text = text;
    }

    unsafe fn present_render_plan(&mut self, plan: RenderPlan) {
        let fallback_text = plan.to_plain_text();
        set_virtual_render_plan(self.virtual_view, plan);
        set_scroll_document_view(self.scroll_view, self.virtual_view);
        self.last_presented_text = fallback_text;
    }

    unsafe fn open_path(&mut self, path: &Path) {
        match self.manager.begin_open_tab(path, OpenIntent::default()) {
            Ok(OpenTabRequest::Existing(_)) => self.render_active_tab(),
            Ok(OpenTabRequest::Pending(pending)) => {
                let path = pending.path().to_path_buf();
                let label = display_path(&path);
                let task = TaskHandle::spawn(format!("open {label}"), move |token, progress| {
                    let _ = progress.send(TaskProgress {
                        name: "Open".into(),
                        processed_bytes: 0,
                        total_bytes: None,
                        message: Some(format!("Opening {label}")),
                    });
                    token.throw_if_cancelled().map_err(anyhow::Error::from)?;
                    let document = pending.open();
                    token.throw_if_cancelled().map_err(anyhow::Error::from)?;
                    document
                });
                set_status(
                    self.status_field,
                    &format!("Opening {}...", display_path(&path)),
                );
                if self.manager.active().is_none() {
                    self.present_text(format!("Opening {}...", display_path(&path)), false);
                }
                self.open_tasks.push(task);
            }
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
        let mut render_plan = None;
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
                max_lines: analysis_viewport_line_budget(&settings),
                max_bytes: settings.initial_viewport_bytes,
            }) {
                Ok(viewport) => {
                    let plan = RenderPlan::from_viewport_with_options(
                        &viewport,
                        analysis_render_options(&settings),
                    );
                    rendered_anchor = ViewAnchor::Byte(viewport.start);
                    next_anchor = plan
                        .next_anchor_byte
                        .and_then(|byte| {
                            viewport
                                .lines
                                .iter()
                                .find(|line| line.end.0 == byte)
                                .map(|line| ViewAnchor::Byte(line.end))
                        })
                        .or_else(|| Some(viewport.next_anchor()));
                    render_plan = Some(plan);
                    String::new()
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

        if let Some(plan) = render_plan {
            self.present_render_plan(plan);
        } else {
            self.present_text(text, mode == EditorMode::Edit);
        }
        let tabs = self.manager.tab_summaries();
        set_window_title(self.window, &window_title(&title, &tabs));
        set_status(self.status_field, &status);
        self.set_tab_bar_from_summaries(&tabs);
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
            max_lines: analysis_viewport_line_budget(&settings),
            max_bytes: settings.initial_viewport_bytes,
        }) {
            Ok(viewport) => {
                let plan = RenderPlan::from_viewport_with_options(
                    &viewport,
                    analysis_render_options(&settings),
                );
                let rendered_anchor = ViewAnchor::Byte(viewport.start);
                let next_anchor = plan
                    .next_anchor_byte
                    .and_then(|byte| {
                        viewport
                            .lines
                            .iter()
                            .find(|line| line.end.0 == byte)
                            .map(|line| ViewAnchor::Byte(line.end))
                    })
                    .unwrap_or_else(|| viewport.next_anchor());
                self.present_render_plan(plan);
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

    unsafe fn activate_tab_by_id(&mut self, tab_id: TabId) {
        if !self.sync_active_edit_buffer() {
            return;
        }
        if self.manager.set_active_tab(tab_id) {
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
        self.set_tab_bar_from_summaries(&self.manager.tab_summaries());
    }

    unsafe fn set_tab_bar_from_summaries(&self, tabs: &[TabSummary]) {
        set_tab_bar(self.tab_bar, tabs);
    }

    unsafe fn poll_background_tasks(&mut self) {
        let mut index = 0usize;
        while index < self.open_tasks.len() {
            while let Ok(progress) = self.open_tasks[index].progress().try_recv() {
                if let Some(message) = progress.message {
                    set_status(self.status_field, &message);
                }
            }

            if !self.open_tasks[index].is_finished() {
                index += 1;
                continue;
            }

            let task = self.open_tasks.swap_remove(index);
            match task.join() {
                Ok(Ok(document)) => {
                    let title = document.title().to_string();
                    self.manager.finish_open_tab(document);
                    self.render_active_tab();
                    set_status(self.status_field, &format!("Opened {title}"));
                }
                Ok(Err(error)) => self.show_error(&format!("Open failed: {error:#}")),
                Err(error) => self.show_error(&format!("Open task failed: {error}")),
            }
        }

        let _ = self.manager.maintain_resource_policy();
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

        let (window, tab_bar, scroll_view, text_view, virtual_view, status_field) =
            create_main_window();
        let state = Box::into_raw(Box::new(AppState::new(
            window,
            tab_bar,
            scroll_view,
            text_view,
            virtual_view,
            status_field,
        )));
        (*delegate).set_ivar("state", state as *mut c_void);
        set_tab_bar_app_state(tab_bar, state as *mut c_void);
        app.setDelegate_(delegate);

        build_menu(app, delegate);
        install_background_task_timer(delegate);
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
            set_tab_bar(tab_bar, &[]);
        } else {
            (*state).open_paths(paths);
        }

        app.run();
    }
}

unsafe fn create_main_window() -> (id, id, id, id, id, id) {
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
    let tab_height = TAB_BAR_HEIGHT;
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

    let tab_scroll: id = msg_send![class!(NSScrollView), alloc];
    let tab_scroll: id = msg_send![tab_scroll, initWithFrame: tab_frame];
    let _: () = msg_send![tab_scroll, setHasHorizontalScroller: YES];
    let _: () = msg_send![tab_scroll, setHasVerticalScroller: NO];
    let _: () = msg_send![tab_scroll, setBorderType: 0u64];
    let _: () = msg_send![tab_scroll, setDrawsBackground: NO];
    let _: () =
        msg_send![tab_scroll, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_MIN_Y_MARGIN];

    let tab_bar = create_tab_bar_view(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(tab_frame.size.width, tab_frame.size.height),
    ));
    let _: () =
        msg_send![tab_bar, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_MIN_Y_MARGIN];
    let _: () = msg_send![tab_scroll, setDocumentView: tab_bar];
    content.addSubview_(tab_scroll);

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

    let virtual_view = create_virtual_text_view(scroll_frame);

    let status_field: id = msg_send![class!(NSTextField), alloc];
    let status_field: id = msg_send![status_field, initWithFrame: status_frame];
    let _: () = msg_send![status_field, setEditable: NO];
    let _: () = msg_send![status_field, setSelectable: NO];
    let _: () = msg_send![status_field, setBordered: NO];
    let _: () = msg_send![status_field, setDrawsBackground: NO];
    let _: () =
        msg_send![status_field, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_MIN_Y_MARGIN];
    content.addSubview_(status_field);

    (
        window,
        tab_bar,
        scroll,
        text_view,
        virtual_view,
        status_field,
    )
}

unsafe fn install_background_task_timer(delegate: id) {
    let _: id = msg_send![
        class!(NSTimer),
        scheduledTimerWithTimeInterval: 0.05f64
        target: delegate
        selector: sel!(pollBackgroundTasks:)
        userInfo: nil
        repeats: YES
    ];
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

fn analysis_viewport_line_budget(settings: &AppSettings) -> usize {
    settings
        .initial_viewport_lines
        .saturating_add(DEFAULT_OVERSCAN_LINES)
        .max(1)
}

fn analysis_render_options(settings: &AppSettings) -> RenderOptions {
    RenderOptions {
        visible_line_count: settings.initial_viewport_lines.max(1),
        overscan_lines: DEFAULT_OVERSCAN_LINES,
        first_column: 0,
        max_columns: DEFAULT_MAX_COLUMNS,
        ..RenderOptions::default()
    }
}

unsafe fn set_text_view(text_view: id, text: &str, editable: bool) {
    let _: () = msg_send![text_view, setString: ns_string(text)];
    let _: () = msg_send![text_view, setEditable: if editable { YES } else { NO }];
}

#[derive(Clone, Debug)]
struct TabBarItem {
    id: TabId,
    label: String,
    active: bool,
    x: f64,
    width: f64,
}

#[derive(Default)]
struct TabBarViewState {
    app_state: *mut c_void,
    items: Vec<TabBarItem>,
}

unsafe fn create_tab_bar_view(frame: NSRect) -> id {
    let view_class = tab_bar_view_class();
    let view: id = msg_send![view_class, alloc];
    let view: id = msg_send![view, initWithFrame: frame];
    let state = Box::into_raw(Box::new(TabBarViewState::default()));
    (*view).set_ivar("state", state as *mut c_void);
    view
}

unsafe fn set_tab_bar_app_state(tab_bar: id, app_state: *mut c_void) {
    let state_ptr = *(*tab_bar).get_ivar::<*mut c_void>("state");
    if state_ptr.is_null() {
        return;
    }
    let state = &mut *(state_ptr as *mut TabBarViewState);
    state.app_state = app_state;
}

unsafe fn set_tab_bar(tab_bar: id, tabs: &[TabSummary]) {
    let state_ptr = *(*tab_bar).get_ivar::<*mut c_void>("state");
    if state_ptr.is_null() {
        return;
    }

    let state = &mut *(state_ptr as *mut TabBarViewState);
    state.items = tab_bar_items(tabs);
    let width = tab_bar_content_width(&state.items);
    let _: () = msg_send![tab_bar, setFrameSize: NSSize::new(width, TAB_BAR_HEIGHT)];
    let _: () = msg_send![tab_bar, setNeedsDisplay: YES];
}

fn tab_bar_items(tabs: &[TabSummary]) -> Vec<TabBarItem> {
    let mut x = TAB_ITEM_GAP;
    tabs.iter()
        .enumerate()
        .map(|(index, tab)| {
            let label = tab_label(index + 1, tab);
            let width = tab_item_width(&label);
            let item = TabBarItem {
                id: tab.id,
                label,
                active: tab.active,
                x,
                width,
            };
            x += width + TAB_ITEM_GAP;
            item
        })
        .collect()
}

fn tab_item_width(label: &str) -> f64 {
    ((label.chars().count() as f64 * TAB_ITEM_CHAR_WIDTH) + (TAB_ITEM_PADDING_X * 2.0))
        .clamp(TAB_ITEM_MIN_WIDTH, TAB_ITEM_MAX_WIDTH)
}

fn tab_bar_content_width(items: &[TabBarItem]) -> f64 {
    items
        .last()
        .map(|item| item.x + item.width + TAB_ITEM_GAP)
        .unwrap_or(TAB_ITEM_MIN_WIDTH)
}

fn tab_bar_view_class() -> *const Class {
    static REGISTER: Once = Once::new();
    static mut CLASS: *const Class = ptr::null();
    REGISTER.call_once(|| unsafe {
        let superclass = class!(NSView);
        let mut decl = ClassDecl::new("FastPadTabBarView", superclass).unwrap();
        decl.add_ivar::<*mut c_void>("state");
        decl.add_method(
            sel!(isFlipped),
            tab_bar_view_is_flipped as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(drawRect:),
            tab_bar_view_draw_rect as extern "C" fn(&Object, Sel, NSRect),
        );
        decl.add_method(
            sel!(mouseDown:),
            tab_bar_view_mouse_down as extern "C" fn(&Object, Sel, id),
        );
        CLASS = decl.register();
    });
    unsafe { CLASS }
}

extern "C" fn tab_bar_view_is_flipped(_: &Object, _: Sel) -> BOOL {
    YES
}

extern "C" fn tab_bar_view_draw_rect(this: &Object, _: Sel, dirty_rect: NSRect) {
    unsafe {
        let background: id = msg_send![class!(NSColor), controlBackgroundColor];
        let _: () = msg_send![background, setFill];
        NSRectFill(dirty_rect);

        let state_ptr = *this.get_ivar::<*mut c_void>("state");
        if state_ptr.is_null() {
            return;
        }
        let state = &*(state_ptr as *const TabBarViewState);

        if state.items.is_empty() {
            let attrs = text_attributes(
                msg_send![class!(NSFont), systemFontOfSize: 12.0f64],
                msg_send![class!(NSColor), secondaryLabelColor],
            );
            draw_string("No tabs", NSPoint::new(TAB_ITEM_PADDING_X, 7.0), attrs);
            return;
        }

        for item in &state.items {
            draw_tab_bar_item(item);
        }
    }
}

unsafe fn draw_tab_bar_item(item: &TabBarItem) {
    let rect = NSRect::new(
        NSPoint::new(item.x, TAB_ITEM_TOP),
        NSSize::new(item.width, TAB_ITEM_HEIGHT),
    );
    let fill: id = if item.active {
        msg_send![
            class!(NSColor),
            colorWithCalibratedRed: 0.10f64
            green: 0.36f64
            blue: 0.82f64
            alpha: 1.0f64
        ]
    } else {
        msg_send![class!(NSColor), windowBackgroundColor]
    };
    let _: () = msg_send![fill, setFill];
    NSRectFill(rect);

    let border: id = if item.active {
        msg_send![class!(NSColor), keyboardFocusIndicatorColor]
    } else {
        msg_send![class!(NSColor), separatorColor]
    };
    let _: () = msg_send![border, setFill];
    NSRectFill(NSRect::new(
        NSPoint::new(item.x, TAB_ITEM_TOP + TAB_ITEM_HEIGHT - 1.0),
        NSSize::new(item.width, 1.0),
    ));
    if item.active {
        let accent: id = msg_send![class!(NSColor), whiteColor];
        let _: () = msg_send![accent, setFill];
        NSRectFill(NSRect::new(
            NSPoint::new(item.x + 1.0, TAB_ITEM_TOP),
            NSSize::new(item.width - 2.0, 2.0),
        ));
    }

    let font: id = if item.active {
        msg_send![class!(NSFont), boldSystemFontOfSize: 12.0f64]
    } else {
        msg_send![class!(NSFont), systemFontOfSize: 12.0f64]
    };
    let color: id = if item.active {
        msg_send![class!(NSColor), whiteColor]
    } else {
        msg_send![class!(NSColor), labelColor]
    };
    let attrs = text_attributes(font, color);
    let text_rect = NSRect::new(
        NSPoint::new(item.x + TAB_ITEM_PADDING_X, TAB_ITEM_TOP + 4.0),
        NSSize::new(
            (item.width - (TAB_ITEM_PADDING_X * 2.0)).max(1.0),
            TAB_ITEM_HEIGHT - 4.0,
        ),
    );
    let _: () = msg_send![ns_string(&item.label), drawInRect: text_rect withAttributes: attrs];
}

extern "C" fn tab_bar_view_mouse_down(this: &Object, _: Sel, event: id) {
    unsafe {
        let event_location: NSPoint = msg_send![event, locationInWindow];
        let point: NSPoint = msg_send![this, convertPoint: event_location fromView: nil];
        let click_count: usize = msg_send![event, clickCount];
        let state_ptr = *this.get_ivar::<*mut c_void>("state");
        if state_ptr.is_null() {
            return;
        }
        let state = &*(state_ptr as *const TabBarViewState);

        let hit_tab = state
            .items
            .iter()
            .find(|item| point.x >= item.x && point.x <= item.x + item.width)
            .map(|item| item.id);

        let Some(app_state) = (state.app_state as *mut AppState).as_mut() else {
            return;
        };

        if let Some(tab_id) = hit_tab {
            app_state.activate_tab_by_id(tab_id);
        } else if click_count >= 2 {
            app_state.new_document();
        }
    }
}

unsafe fn set_scroll_document_view(scroll_view: id, document_view: id) {
    let current: id = msg_send![scroll_view, documentView];
    if current != document_view {
        let _: () = msg_send![scroll_view, setDocumentView: document_view];
    }
}

#[derive(Debug, Default)]
struct VirtualTextViewState {
    plan: RenderPlan,
}

unsafe fn create_virtual_text_view(frame: NSRect) -> id {
    let view_class = virtual_text_view_class();
    let view: id = msg_send![view_class, alloc];
    let view: id = msg_send![view, initWithFrame: frame];
    let state = Box::into_raw(Box::new(VirtualTextViewState::default()));
    (*view).set_ivar("state", state as *mut c_void);
    let _: () =
        msg_send![view, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE];
    view
}

unsafe fn set_virtual_render_plan(view: id, plan: RenderPlan) {
    let state_ptr = *(*view).get_ivar::<*mut c_void>("state");
    if state_ptr.is_null() {
        return;
    }

    let state = &mut *(state_ptr as *mut VirtualTextViewState);
    state.plan = plan;
    let _: () = msg_send![view, setFrameSize: virtual_content_size(&state.plan)];
    let _: () = msg_send![view, setNeedsDisplay: YES];
}

fn virtual_content_size(plan: &RenderPlan) -> NSSize {
    let width = (plan.estimated_content_width_columns.max(80) as f64 * VIRTUAL_CHAR_WIDTH)
        + VIRTUAL_GUTTER_PADDING
        + VIRTUAL_TEXT_PADDING
        + 24.0;
    let height =
        (plan.lines.len().max(1) as f64 * VIRTUAL_LINE_HEIGHT) + (VIRTUAL_TOP_PADDING * 2.0);
    NSSize::new(width, height)
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

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
struct TabBarRender {
    text: String,
    active_range_utf16: Option<(usize, usize)>,
}

#[cfg(test)]
fn tab_bar_render(tabs: &[TabSummary]) -> TabBarRender {
    if tabs.is_empty() {
        return TabBarRender {
            text: "No tabs".to_string(),
            active_range_utf16: None,
        };
    }

    let mut text = String::new();
    let mut active_range_utf16 = None;

    for (index, tab) in tabs.iter().enumerate() {
        if index > 0 {
            text.push_str("  |  ");
        }

        let label = tab_label(index + 1, tab);
        let start = text.encode_utf16().count();
        text.push_str(&label);

        if tab.active {
            active_range_utf16 = Some((start, label.encode_utf16().count()));
        }
    }

    TabBarRender {
        text,
        active_range_utf16,
    }
}

fn tab_label(tab_number: usize, tab: &TabSummary) -> String {
    let mut flags = String::new();
    if tab.view_analysis {
        flags.push_str(" 👁");
    }
    if tab.dirty {
        flags.push_str(" *");
    }
    if tab.read_only {
        flags.push_str(" 🔒");
    }
    if tab.external_modified {
        flags.push_str(" ⚠");
    }
    let pin = if tab.pinned { "📌 " } else { "" };
    format!("{tab_number}. {pin}{}{flags}", tab.title)
}

fn window_title(title: &str, tabs: &[TabSummary]) -> String {
    let count = tabs.len();
    let Some(active_index) = tabs
        .iter()
        .position(|tab| tab.active)
        .map(|index| index + 1)
    else {
        return format!("{title} - FastPad");
    };
    format!("{active_index}/{count} {title} - FastPad")
}

fn display_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

fn virtual_text_view_class() -> *const Class {
    static REGISTER: Once = Once::new();
    static mut CLASS: *const Class = ptr::null();
    REGISTER.call_once(|| unsafe {
        let superclass = class!(NSView);
        let mut decl = ClassDecl::new("FastPadVirtualTextView", superclass).unwrap();
        decl.add_ivar::<*mut c_void>("state");
        decl.add_method(
            sel!(drawRect:),
            virtual_text_view_draw_rect as extern "C" fn(&Object, Sel, NSRect),
        );
        decl.add_method(
            sel!(isFlipped),
            virtual_text_view_is_flipped as extern "C" fn(&Object, Sel) -> BOOL,
        );
        CLASS = decl.register();
    });
    unsafe { CLASS }
}

extern "C" fn virtual_text_view_is_flipped(_this: &Object, _sel: Sel) -> BOOL {
    YES
}

extern "C" fn virtual_text_view_draw_rect(this: &Object, _sel: Sel, dirty_rect: NSRect) {
    unsafe {
        let state_ptr = *this.get_ivar::<*mut c_void>("state");
        if state_ptr.is_null() {
            return;
        }
        let state = &*(state_ptr as *const VirtualTextViewState);

        let background: id = msg_send![class!(NSColor), textBackgroundColor];
        let _: () = msg_send![background, setFill];
        NSRectFill(dirty_rect);

        let gutter_width = state.plan.gutter_width_columns as f64 * VIRTUAL_GUTTER_CHAR_WIDTH;
        if gutter_width > 0.0 {
            let gutter_background: id = msg_send![class!(NSColor), controlBackgroundColor];
            let _: () = msg_send![gutter_background, setFill];
            NSRectFill(NSRect::new(
                NSPoint::new(0.0, dirty_rect.origin.y),
                NSSize::new(
                    VIRTUAL_GUTTER_PADDING + gutter_width + VIRTUAL_TEXT_PADDING * 0.5,
                    dirty_rect.size.height,
                ),
            ));

            let separator: id = msg_send![class!(NSColor), separatorColor];
            let _: () = msg_send![separator, setFill];
            NSRectFill(NSRect::new(
                NSPoint::new(
                    VIRTUAL_GUTTER_PADDING + gutter_width + VIRTUAL_TEXT_PADDING * 0.5,
                    dirty_rect.origin.y,
                ),
                NSSize::new(1.0, dirty_rect.size.height),
            ));
        }

        let font: id = msg_send![class!(NSFont), userFixedPitchFontOfSize: VIRTUAL_FONT_SIZE];
        let text_color: id = msg_send![class!(NSColor), labelColor];
        let gutter_color: id = msg_send![class!(NSColor), secondaryLabelColor];
        let text_attrs = text_attributes(font, text_color);
        let gutter_attrs = text_attributes(font, gutter_color);

        let visible_start = dirty_rect.origin.y.max(0.0);
        let visible_end = (dirty_rect.origin.y + dirty_rect.size.height).max(visible_start);
        let first_line =
            ((visible_start - VIRTUAL_TOP_PADDING).max(0.0) / VIRTUAL_LINE_HEIGHT).floor() as usize;
        let last_line =
            (((visible_end - VIRTUAL_TOP_PADDING).max(0.0) / VIRTUAL_LINE_HEIGHT).ceil() as usize)
                .saturating_add(1)
                .min(state.plan.lines.len());

        for line_index in first_line..last_line {
            let Some(line) = state.plan.lines.get(line_index) else {
                continue;
            };
            let y = VIRTUAL_TOP_PADDING + line_index as f64 * VIRTUAL_LINE_HEIGHT;

            if state.plan.gutter_width_columns > 0 {
                let number = line
                    .display_line_number
                    .map(|number| number.to_string())
                    .unwrap_or_default();
                let gutter = format!("{number:>width$}", width = state.plan.gutter_width_columns);
                draw_string(
                    &gutter,
                    NSPoint::new(VIRTUAL_GUTTER_PADDING, y),
                    gutter_attrs,
                );
            }

            let mut text = String::new();
            if line.continued_left {
                text.push_str("...");
            }
            text.push_str(&line.visible_text);
            if line.continued_right {
                text.push_str("...");
            }

            let text_x = VIRTUAL_GUTTER_PADDING
                + gutter_width
                + if gutter_width > 0.0 {
                    VIRTUAL_TEXT_PADDING
                } else {
                    0.0
                };
            draw_string(&text, NSPoint::new(text_x, y), text_attrs);
        }
    }
}

unsafe fn text_attributes(font: id, color: id) -> id {
    let attrs: id = msg_send![class!(NSMutableDictionary), dictionary];
    let _: () = msg_send![attrs, setObject: font forKey: NSFontAttributeName];
    let _: () = msg_send![attrs, setObject: color forKey: NSForegroundColorAttributeName];
    attrs
}

unsafe fn draw_string(text: &str, point: NSPoint, attributes: id) {
    let value = ns_string(text);
    let _: () = msg_send![value, drawAtPoint: point withAttributes: attributes];
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
            sel!(pollBackgroundTasks:),
            poll_background_tasks as extern "C" fn(&Object, Sel, id),
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

extern "C" fn poll_background_tasks(this: &Object, _: Sel, _: id) {
    unsafe {
        if let Some(state) = state_from_delegate(this) {
            state.poll_background_tasks();
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

#[cfg(test)]
mod tests {
    use super::*;
    use fastpad_core::{DocumentId, TabId, ViewId};

    fn summary(tab_number: u64, title: &str, active: bool) -> TabSummary {
        TabSummary {
            id: TabId(tab_number),
            view_id: ViewId(tab_number),
            document_id: DocumentId(tab_number),
            title: title.to_string(),
            dirty: false,
            read_only: false,
            view_analysis: false,
            external_modified: false,
            pinned: false,
            preview: false,
            active,
        }
    }

    #[test]
    fn tab_bar_numbers_tabs_and_marks_active_front_tab() {
        let tabs = vec![
            summary(1, "first.txt", false),
            summary(2, "second.txt", true),
            summary(3, "third.txt", false),
        ];

        let render = tab_bar_render(&tabs);
        let active_label = "2. second.txt";
        let active_start = render.text.find(active_label).unwrap();

        assert!(render.text.contains("1. first.txt"));
        assert!(render.text.contains(active_label));
        assert!(render.text.contains("3. third.txt"));
        assert!(!render.text.contains("📄"));
        assert!(!render.text.contains("📝"));
        assert_eq!(
            render.active_range_utf16,
            Some((
                render.text[..active_start].encode_utf16().count(),
                active_label.encode_utf16().count()
            ))
        );
    }

    #[test]
    fn tab_labels_use_visible_state_markers_without_file_icon() {
        let mut tab = summary(1, "dump.sql", true);
        tab.view_analysis = true;
        tab.dirty = true;
        tab.read_only = true;
        tab.external_modified = true;
        tab.pinned = true;

        let label = tab_label(1, &tab);

        assert!(label.starts_with("1. 📌 dump.sql"));
        assert!(label.contains("📌"));
        assert!(label.contains("👁"));
        assert!(label.contains("*"));
        assert!(label.contains("🔒"));
        assert!(label.contains("⚠"));
        assert!(!label.contains("📄"));
        assert!(!label.contains("🧾"));
    }

    #[test]
    fn window_title_includes_active_tab_position() {
        let tabs = vec![
            summary(1, "first.txt", false),
            summary(2, "second.txt", true),
        ];

        assert_eq!(
            window_title("second.txt", &tabs),
            "2/2 second.txt - FastPad"
        );
    }
}
