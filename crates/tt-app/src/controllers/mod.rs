//! Controllers connect the UI to the services.
//!
//! Pattern: each UI area has one `*Bridge` global in `ui/` (e.g. `TimerBridge`) with
//! `in` properties holding view structs and callbacks for user actions. Its controller
//! module (`controllers/timer.rs`, …) provides `install(ui, ctx: &Ctx)` that
//!
//! 1. registers the bridge callbacks; each handler borrows the [`Ctx`](crate::app::Ctx)
//!    briefly, calls a service, and never holds the borrow across UI calls;
//! 2. maps results to view structs with pure `vm::*` functions (unit-tested without a
//!    window) and pushes them into the bridge properties;
//! 3. reports errors through `Tr` strings (toast), logging only ids and error codes.
//!
//! Globals are per window instance, so state shared with other windows (the mini timer)
//! is pushed to each of them by the same helper.
