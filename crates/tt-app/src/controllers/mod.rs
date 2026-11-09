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

pub mod calendar;
pub mod entries;
pub mod pickers;
pub mod timer;

use slint::{Model, SharedString, VecModel};
use tt_services::ServiceError;

use crate::Tr;
use crate::vm::error::{ErrorText, error_text};

/// Localized message for a service error.
pub fn error_message(tr: &Tr<'_>, e: &ServiceError) -> SharedString {
    match error_text(e) {
        ErrorText::InUseProjects => tr.get_err_in_use_projects(),
        ErrorText::InUseTasks => tr.get_err_in_use_tasks(),
        ErrorText::InUseEntries => tr.get_err_in_use_entries(),
        ErrorText::Overlap => tr.get_err_overlap(),
        ErrorText::InvalidName => tr.get_err_invalid_name(),
        ErrorText::InvalidColor => tr.get_err_invalid_color(),
        ErrorText::InvalidRange => tr.get_err_invalid_range(),
        ErrorText::Backup => tr.get_err_backup(),
        ErrorText::Generic => tr.get_err_generic(),
    }
}

/// Makes `model` hold `items`, notifying only rows that changed, appended or removed (a
/// list view keeps its instances and scroll position).
pub fn sync_vec<T: Clone + PartialEq + 'static>(model: &VecModel<T>, items: Vec<T>) {
    while model.row_count() > items.len() {
        model.remove(model.row_count() - 1);
    }
    for (i, item) in items.into_iter().enumerate() {
        match model.row_data(i) {
            Some(old) if old == item => {}
            Some(_) => model.set_row_data(i, item),
            None => model.push(item),
        }
    }
}

#[cfg(test)]
mod tests {
    use slint::{Model, VecModel};

    use super::sync_vec;

    #[test]
    fn sync_vec_grows_updates_and_shrinks_in_place() {
        let model = VecModel::from(vec![1, 2, 3]);
        sync_vec(&model, vec![1, 2, 3, 4, 5]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        sync_vec(&model, vec![1, 9, 3, 4, 5]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [1, 9, 3, 4, 5]);
        sync_vec(&model, vec![1, 9]);
        assert_eq!(model.iter().collect::<Vec<_>>(), [1, 9]);
        sync_vec(&model, vec![]);
        assert_eq!(model.row_count(), 0);
    }
}
