//! Application services over `tt-storage`: timer, catalog, entries and settings.

pub mod catalog;
pub mod clock;
pub mod entries;
pub mod error;
pub mod export;
pub mod reports;
pub mod settings;
pub mod timer;
mod tx;

#[cfg(any(test, feature = "test-util"))]
pub use clock::FixedClock;

pub use catalog::{CatalogService, NewProject};
pub use clock::{Clock, SystemClock};
pub use entries::{EntryService, default_billable};

pub use error::{Result, ServiceError};
pub use export::{ExportLabels, Lookup, export_entries_csv, export_report_csv, export_report_xlsx};
pub use reports::{Dashboard, DashboardService, ReportService};
pub use settings::{Settings, SettingsService, Theme};
pub use timer::{TimerService, TimerStatus};
