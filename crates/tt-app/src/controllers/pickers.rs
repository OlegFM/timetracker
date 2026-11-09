//! Implements the `Pickers` helper callbacks (search, "can create", tag toggling) with
//! the pure functions of [`vm::pickers`](crate::vm::pickers).

use slint::{ComponentHandle, Global, Model, ModelRc, VecModel};

use crate::Pickers;
use crate::vm::pickers as vm;

/// Registers the helpers in `ui` (needed in every window that shows a picker).
pub fn install<C>(ui: &C)
where
    C: ComponentHandle,
    for<'a> Pickers<'a>: Global<'a, C>,
{
    let p = ui.global::<Pickers>();
    p.on_filter_options(|items, query| model(vm::filter_options(&rows(&items), &query)));
    p.on_option_index(|items, id| vm::option_index(&rows(&items), &id));
    p.on_option_creatable(|items, query| vm::option_creatable(&rows(&items), &query));
    p.on_filter_tags(|items, query| model(vm::filter_tags(&rows(&items), &query)));
    p.on_tag_creatable(|items, query| vm::tag_creatable(&rows(&items), &query));
    p.on_toggle_tag(|items, id| model(vm::toggle_tag(&rows(&items), &id)));
    p.on_select_tags(|items, ids| model(vm::select_tags(&rows(&items), &rows(&ids))));
    p.on_selected_names(|items, ids| vm::selected_names(&rows(&items), &rows(&ids)).into());
}

fn rows<T: Clone + 'static>(m: &ModelRc<T>) -> Vec<T> {
    m.iter().collect()
}

pub(crate) fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}
