//! Implements the `Calendar` global (month grids and date labels for date pickers) with
//! the user's week start, date format and time zone.

use slint::{ComponentHandle, Global};
use tt_domain::{format_date, local_date};
use tt_services::Clock;

use crate::Calendar;
use crate::app::Ctx;
use crate::vm::dates::{calendar_month, parse_iso};

/// Registers the callbacks in `ui` (needed in every window that shows a date picker).
pub fn install<C>(ui: &C, ctx: &Ctx)
where
    C: ComponentHandle,
    for<'a> Calendar<'a>: Global<'a, C>,
{
    let cal = ui.global::<Calendar>();
    let c = ctx.clone();
    cal.on_month(move |date, delta| {
        let (week_start, today) = {
            let c = c.borrow();
            let f = &c.settings.format;
            (f.week_start, local_date(c.timer.clock().now(), f.tz))
        };
        calendar_month(parse_iso(&date), delta, week_start, today)
    });
    let c = ctx.clone();
    cal.on_format(move |date| {
        let f = c.borrow().settings.format.date;
        parse_iso(&date)
            .map(|d| format_date(d, f).into())
            .unwrap_or_default()
    });
}
