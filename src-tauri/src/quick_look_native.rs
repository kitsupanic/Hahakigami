//! Main-thread-only Quick Look ownership. Framework callbacks own panel configuration.
use super::{Check, Registry, Status};
use objc2::{
    define_class, msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, ProtocolObject},
    sel, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSEvent, NSEventType, NSResponder, NSWindow, NSWindowDelegate, NSWindowWillCloseNotification,
};
use objc2_foundation::{
    NSInteger, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSRunLoop,
    NSRunLoopCommonModes, NSTimer, NSURL,
};
use objc2_quick_look_ui::{
    QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource, QLPreviewPanelDelegate,
};
use std::{
    cell::{Cell, RefCell},
    ffi::CString,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    ptr::NonNull,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) type Emit = Arc<dyn Fn(Status) + Send + Sync>;
struct ItemState {
    url: RefCell<Option<Retained<NSURL>>>,
    registry: Arc<Registry>,
    revision: u64,
    check: RefCell<Option<Check>>,
    enabled: Cell<bool>,
}
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "HahakigamiQuickLookItem"]
    #[ivars = ItemState]
    struct Item;
    unsafe impl NSObjectProtocol for Item {}
    unsafe impl QLPreviewItem for Item {
        #[unsafe(method_id(previewItemURL))]
        fn preview_url(&self) -> Option<Retained<NSURL>> {
            self.checked_url()
        }
    }
);
impl Item {
    fn checked_url(&self) -> Option<Retained<NSURL>> {
        let state = self.ivars();
        let check = state.check.borrow().clone()?;
        if !state.enabled.get() || !state.registry.is_current(state.revision) || check().is_err() {
            return None;
        }
        state.url.borrow().clone()
    }
    fn disable(&self) {
        let state = self.ivars();
        state.enabled.set(false);
        state.check.borrow_mut().take();
        state.url.borrow_mut().take();
    }
}
struct ControllerState {
    registry: Arc<Registry>,
    revision: u64,
    scan_id: String,
    node_id: u32,
    path: PathBuf,
    item: Retained<Item>,
    anchor: Weak<NSResponder>,
    original: Option<Retained<NSResponder>>,
    active: Cell<bool>,
    closing_panel: RefCell<Option<Retained<QLPreviewPanel>>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    fullscreen_exit_issued: Cell<bool>,
    last_windowed_close: Cell<Option<Instant>>,
    emit: Emit,
}
define_class!(
    // NSResponder has no additional subclass initialization requirements.
    #[unsafe(super = NSResponder)]
    #[thread_kind = MainThreadOnly]
    #[name = "HahakigamiQuickLookController"]
    #[ivars = ControllerState]
    struct Controller;
    unsafe impl NSObjectProtocol for Controller {}
    impl Controller {
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts(&self, _panel: &QLPreviewPanel) -> bool {
            (self.ivars().active.get() && self.ivars().registry.is_current(self.ivars().revision)) || self.ivars().closing_panel.borrow().is_some()
        }
        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_control(&self, panel: &QLPreviewPanel) {
            if !self.owns(panel) { return; }
            if self.ivars().closing_panel.borrow().is_some() { unsafe {panel.setDataSource(None);panel.setDelegate(None);} return; }
            if !self.ivars().active.get() {return;}
            // Framework invokes this callback; threadlocal retains self until teardown.
            unsafe { panel.setDataSource(Some(ProtocolObject::from_ref(self))); panel.setDelegate(Some(self)); }
        }
        #[unsafe(method(endPreviewPanelControl:))]
        fn end_control(&self, panel: &QLPreviewPanel) {
            if self.owns(panel) {
                unsafe { panel.setDataSource(None); panel.setDelegate(None); }
            }
            if self.ivars().closing_panel.borrow().is_some() { self.revoke(); } else { self.finish(); }
        }
        #[unsafe(method(cancelOperation:))]
        fn cancel_preview(&self, _sender: Option<&AnyObject>) { self.escape(); }
        #[unsafe(method(pollPreviewVisibility:))]
        fn poll_visibility(&self, _timer: &NSTimer) {
            if !self.ivars().active.get() { return; }
            if let Some(panel) = unsafe { QLPreviewPanel::sharedPreviewPanel(self.mtm()) } {
                if !panel.isVisible() && !unsafe { panel.isInFullScreenMode() } {
                    let _ = self.dispose();
                }
            }
        }
        #[unsafe(method(hostWindowWillClose:))]
        fn host_closed(&self, _notification: &NSNotification) {
            if self.ivars().registry.is_current(self.ivars().revision) {
                let status = self.ivars().registry.invalidate();
                (self.ivars().emit)(status);
            }
            let _ = self.dispose();
        }
    }
    unsafe impl QLPreviewPanelDataSource for Controller {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, panel: Option<&QLPreviewPanel>) -> NSInteger {
            if panel.is_some_and(|p|self.owns(p)) && self.ivars().active.get() && self.ivars().registry.is_current(self.ivars().revision) { 1 } else { 0 }
        }
        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item(&self, panel: Option<&QLPreviewPanel>, index: NSInteger) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
            self.checked_item(panel,index)
        }
    }
    unsafe impl NSWindowDelegate for Controller {
        #[unsafe(method(windowWillClose:))]
        fn preview_closed(&self, _notification: &NSNotification) { self.finish(); }
    }
    unsafe impl QLPreviewPanelDelegate for Controller {
        #[unsafe(method(previewPanel:handleEvent:))]
        fn handle_event(&self, panel: &QLPreviewPanel, event: &NSEvent) -> bool {
            if self.owns(panel) && event.r#type() == NSEventType::KeyDown && event.keyCode() == 53 {
                self.escape();
                true
            } else { false }
        }
    }
);
thread_local! { static CONTROLLER: RefCell<Option<Retained<Controller>>> = const { RefCell::new(None) }; }
impl Controller {
    fn escape(&self) {
        if let Some(panel) = unsafe { QLPreviewPanel::sharedPreviewPanel(self.mtm()) } {
            if self.owns(&panel) && unsafe { panel.isInFullScreenMode() } {
                unsafe {
                    panel.exitFullScreenModeWithOptions(None);
                }
                return;
            }
        }
        let _ = self.dispose();
    }
    fn checked_item(
        &self,
        panel: Option<&QLPreviewPanel>,
        index: NSInteger,
    ) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
        if index != 0 || !panel.is_some_and(|p| self.owns(p)) || !self.ivars().active.get() {
            return None;
        }
        self.ivars().item.checked_url()?;
        Some(ProtocolObject::from_retained(self.ivars().item.clone()))
    }
    fn owns(&self, panel: &QLPreviewPanel) -> bool {
        unsafe { panel.currentController() }
            .is_some_and(|object| std::ptr::eq(&*object, self as *const Self as *const AnyObject))
    }
    fn revoke(&self) {
        self.ivars().item.disable();
        if let Some(timer) = self.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
        if self.ivars().active.replace(false) {
            if let Ok(status) = self.ivars().registry.publish(
                self.ivars().revision,
                self.ivars().scan_id.clone(),
                self.ivars().node_id,
                false,
            ) {
                (self.ivars().emit)(status);
            }
        }
    }
    fn finish(&self) {
        self.revoke();
        self.restore();
    }
    fn restore(&self) {
        if let Some(anchor) = self.ivars().anchor.load() {
            let ours = unsafe { anchor.nextResponder() }
                .is_some_and(|next| std::ptr::eq(&*next, self as &NSResponder));
            if ours {
                unsafe {
                    anchor.setNextResponder(self.ivars().original.as_deref());
                }
            }
        }
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(self);
        }
    }
    fn dispose(&self) -> bool {
        let mtm = self.mtm();
        let retained = self.ivars().closing_panel.borrow().clone();
        let panel = retained.or_else(|| {
            if unsafe { QLPreviewPanel::sharedPreviewPanelExists(mtm) } {
                unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }.filter(|panel| self.owns(panel))
            } else {
                None
            }
        });
        let Some(panel) = panel else {
            self.finish();
            return true;
        };
        *self.ivars().closing_panel.borrow_mut() = Some(panel.clone());
        // Closing-only control exposes no items or URLs but keeps the responder
        // available for AppKit's asynchronous fullscreen transition.
        self.revoke();
        if self.owns(&panel) {
            unsafe {
                panel.setDataSource(None);
                panel.setDelegate(None);
            }
            if unsafe { panel.isInFullScreenMode() } {
                if !self.ivars().fullscreen_exit_issued.replace(true) {
                    self.ivars().last_windowed_close.set(None);
                    unsafe {
                        panel.exitFullScreenModeWithOptions(None);
                    }
                }
            } else if panel.isVisible()
                && self
                    .ivars()
                    .last_windowed_close
                    .get()
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
            {
                // An initial close can arrive during the exit animation. Retry
                // only while owning the panel, at most once per second.
                self.ivars().last_windowed_close.set(Some(Instant::now()));
                panel.close();
            }
        }
        if panel.isVisible() || unsafe { panel.isInFullScreenMode() } {
            return false;
        }
        self.finish();
        if self.owns(&panel) {
            unsafe {
                panel.updateController();
            }
        }
        true
    }
}

/// `window` is the live native NSWindow handle obtained on this same main thread.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn present(
    window: *mut std::ffi::c_void,
    registry: Arc<Registry>,
    revision: u64,
    scan_id: String,
    node_id: u32,
    path: PathBuf,
    check: Check,
    emit: Emit,
) -> Result<Option<Status>, String> {
    let mtm = MainThreadMarker::new().ok_or("Quick Look requires the main thread")?;
    registry.mark_native_attempted();
    let window =
        unsafe { (window as *mut NSWindow).as_ref() }.ok_or("Native window is unavailable")?;
    if !registry.is_current(revision) {
        return Err("Quick Look request is stale".into());
    }
    let previous = CONTROLLER.with(|slot| slot.borrow().clone());
    if let Some(previous) = &previous {
        let state = previous.ivars();
        let same = Arc::ptr_eq(&registry, &state.registry)
            && state.scan_id == scan_id
            && state.node_id == node_id
            && state.path.as_os_str().as_bytes() == path.as_os_str().as_bytes();
        let visible = unsafe { QLPreviewPanel::sharedPreviewPanelExists(mtm) }
            && unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }
                .is_some_and(|p| previous.owns(&p) && p.isVisible());
        if same && visible && state.active.get() {
            previous.dispose();
            let status = registry.publish(revision, scan_id, node_id, false)?;
            emit(status.clone());
            return Ok(Some(status));
        }
        if !previous.dispose() {
            return Ok(None);
        }
    }
    check()?;
    if !registry.is_current(revision) {
        return Err("Quick Look request is stale".into());
    }
    let bytes = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "Preview path contains a null byte")?;
    let url = unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
            NonNull::new(bytes.as_ptr().cast_mut()).expect("CString pointer"),
            false,
            None,
        )
    };
    let item = Item::alloc(mtm).set_ivars(ItemState {
        url: RefCell::new(Some(url)),
        registry: registry.clone(),
        revision,
        check: RefCell::new(Some(check)),
        enabled: Cell::new(true),
    });
    let item: Retained<Item> = unsafe { msg_send![super(item), init] };
    let anchor = window.firstResponder().unwrap_or_else(|| unsafe {
        Retained::retain((window as &NSResponder as *const NSResponder).cast_mut())
            .expect("live native window")
    });
    let controller = Controller::alloc(mtm).set_ivars(ControllerState {
        registry: registry.clone(),
        revision,
        scan_id: scan_id.clone(),
        node_id,
        path,
        item,
        anchor: Weak::new(&anchor),
        original: unsafe { anchor.nextResponder() },
        active: Cell::new(true),
        closing_panel: RefCell::new(None),
        timer: RefCell::new(None),
        fullscreen_exit_issued: Cell::new(false),
        last_windowed_close: Cell::new(None),
        emit: emit.clone(),
    });
    let controller: Retained<Controller> = unsafe { msg_send![super(controller), init] };
    unsafe {
        controller.setNextResponder(controller.ivars().original.as_deref());
        anchor.setNextResponder(Some(&controller));
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &controller,
            sel!(hostWindowWillClose:),
            Some(NSWindowWillCloseNotification),
            Some(window),
        );
    }
    CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
    window.makeKeyAndOrderFront(None);
    let panel = match unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) } {
        Some(panel) => panel,
        None => {
            let _ = controller.dispose();
            return Err("Quick Look panel is unavailable".into());
        }
    };
    panel.makeKeyAndOrderFront(None);
    unsafe {
        panel.updateController();
    }
    if !controller.owns(&panel) {
        let _ = controller.dispose();
        return Err("Quick Look could not acquire preview control".into());
    }
    unsafe {
        panel.reloadData();
    }
    panel.makeKeyAndOrderFront(None);
    if !registry.is_current(revision) {
        let _ = controller.dispose();
        return Err("Quick Look request is stale".into());
    }
    if controller.ivars().item.checked_url().is_none() {
        let _ = controller.dispose();
        return Err("Preview authority changed before presentation".into());
    }
    let status = match registry.publish(revision, scan_id, node_id, true) {
        Ok(status) => status,
        Err(error) => {
            let _ = controller.dispose();
            return Err(error);
        }
    };
    // Quick Look may order out on Escape without NSWindowWillClose. A common-mode
    // main-loop timer observes visibility without touching files. Invalidating it
    // in finish breaks the timer-target/controller retention cycle.
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            0.05,
            &controller,
            sel!(pollPreviewVisibility:),
            None,
            true,
        )
    };
    unsafe {
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
    *controller.ivars().timer.borrow_mut() = Some(timer);
    emit(status.clone());
    Ok(Some(status))
}
pub(super) fn close(registry: &Arc<Registry>, revision: u64) -> Result<bool, String> {
    let _mtm = MainThreadMarker::new().ok_or("Quick Look requires the main thread")?;
    let controller = CONTROLLER.with(|slot| slot.borrow().clone());
    if let Some(controller) = controller {
        if Arc::ptr_eq(registry, &controller.ivars().registry) {
            if controller.ivars().revision > revision && controller.ivars().active.get() {
                return Err("Quick Look close was superseded by a newer preview".into());
            }
            if !controller.dispose() {
                return Ok(false);
            }
            CONTROLLER.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    Ok(true)
}
