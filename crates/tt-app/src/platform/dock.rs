//! The Dock icon and the "reopen" request on macOS (spec section 5.3); on Windows and Linux
//! these functions do nothing.
//!
//! The Dock icon is shown only while the main window is: [`set_visible`] switches the app
//! between the `Regular` and `Accessory` activation policies, so the mini timer alone puts
//! no icon in the Dock and no entry in `Cmd+Tab` (as on Windows it has no taskbar button).
//!
//! Opening the app again from Finder, Launchpad or Spotlight does not start a second
//! process: macOS sends the running one a "reopen" Apple Event. [`on_reopen`] handles it,
//! so the main window comes back even while it is hidden in the menu bar.
//!
//! The handler goes through `NSAppleEventManager`, not an `NSApplicationDelegate`: winit
//! 0.30 registers its own application delegate in `EventLoop::new` and panics when it finds
//! another one there (`ApplicationDelegate::get`), whatever its `platform/macos.rs` says.

/// Shows (`true`) or hides the Dock icon; call with `true` before showing the main window
/// and with `false` after hiding it.
pub fn set_visible(on: bool) {
    #[cfg(target_os = "macos")]
    macos::set_visible(on);
    #[cfg(not(target_os = "macos"))]
    let _ = on;
}

/// Runs `f` on the UI thread when the app is opened again while it runs. Call once, on the
/// main thread, after the event loop has started: `NSApplication` installs its own "reopen"
/// handler while it launches and would replace an earlier one.
pub fn on_reopen(f: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    macos::on_reopen(f);
    #[cfg(not(target_os = "macos"))]
    let _ = f;
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::sync::Arc;

    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    use super::handler::ReopenHandler;

    thread_local! {
        /// `NSAppleEventManager` does not retain its handlers: this one lives as long as
        /// the main thread.
        static HANDLER: RefCell<Option<Retained<ReopenHandler>>> = const { RefCell::new(None) };
    }

    pub fn set_visible(on: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("Dock icon not changed: not on the main thread");
            return;
        };
        let policy = if on {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        };
        if !NSApplication::sharedApplication(mtm).setActivationPolicy(policy) {
            log::warn!("Dock icon not changed");
        }
    }

    pub fn on_reopen(f: impl Fn() + Send + Sync + 'static) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("reopen handler not set: not on the main thread");
            return;
        };
        // The event arrives outside Slint's callbacks: go through the event loop, as the
        // tray and single-instance events do.
        let f = Arc::new(f);
        let post = move || {
            let f = f.clone();
            if let Err(e) = slint::invoke_from_event_loop(move || f()) {
                log::warn!("reopen dropped: {e}");
            }
        };
        let handler = ReopenHandler::new(mtm, Box::new(post));
        if handler.install() {
            HANDLER.with(|h| h.replace(Some(handler)));
        } else {
            log::warn!("reopen handler not set: no Apple Event manager");
        }
    }
}

// The only unsafe code here: an Objective-C class and two messages to the Apple Event
// manager (no safe binding without another `objc2-*` crate).
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod handler {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send, sel,
    };

    /// `kCoreEventClass` and `kAEReopenApplication` (four-character codes).
    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");

    pub struct Ivars {
        on_reopen: Box<dyn Fn()>,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and `ReopenHandler` does not
        // implement `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "TimeTrackerReopenHandler"]
        #[ivars = Ivars]
        pub struct ReopenHandler;

        impl ReopenHandler {
            // SAFETY: the signature of an Apple Event handler method:
            // `- (void)handleEvent:(NSAppleEventDescriptor *)e withReplyEvent:(NSAppleEventDescriptor *)r`.
            #[unsafe(method(handleReopen:withReplyEvent:))]
            fn handle_reopen(&self, _event: &AnyObject, _reply: &AnyObject) {
                (self.ivars().on_reopen)();
            }
        }

        // SAFETY: `NSObjectProtocol` has no safety requirements.
        unsafe impl NSObjectProtocol for ReopenHandler {}
    );

    impl ReopenHandler {
        pub fn new(mtm: MainThreadMarker, on_reopen: Box<dyn Fn()>) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(Ivars { on_reopen });
            // SAFETY: `NSObject`'s `init` takes no arguments and returns the initialized object.
            unsafe { msg_send![super(this), init] }
        }

        /// Makes this object the handler of the "reopen" Apple Event; `false` when there is
        /// no Apple Event manager. The caller keeps the object alive.
        pub fn install(&self) -> bool {
            // SAFETY: `+[NSAppleEventManager sharedAppleEventManager]` takes no arguments and
            // returns an object (or nil).
            let manager: Option<Retained<AnyObject>> =
                unsafe { msg_send![class!(NSAppleEventManager), sharedAppleEventManager] };
            let Some(manager) = manager else {
                return false;
            };
            // SAFETY: `setEventHandler:andSelector:forEventClass:andEventID:` takes an object,
            // the selector of a method of it with the handler signature (defined above) and
            // two `FourCharCode`s (`UInt32`), and returns nothing.
            unsafe {
                let _: () = msg_send![
                    &*manager,
                    setEventHandler: self,
                    andSelector: sel!(handleReopen:withReplyEvent:),
                    forEventClass: CORE_EVENT_CLASS,
                    andEventID: REOPEN_APPLICATION
                ];
            }
            true
        }
    }
}
