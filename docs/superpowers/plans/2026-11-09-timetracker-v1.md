# TimeTracker v1 (ядро) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Нативное Windows-приложение учёта времени (ядро функциональности solidtime без аккаунтов): таймер с паузой, мини-таймер поверх окон, трей, записи, проекты/задачи/клиенты/теги, обзор, сводный отчёт с экспортом CSV/XLSX, 4 языка.

**Architecture:** Cargo workspace из 4 крейтов со строго нисходящими зависимостями: `tt-domain` (чистая логика, без I/O) ← `tt-storage` (SQLite) ← `tt-services` (сценарии, транзакции, экспорт, бэкап, обновления) ← `tt-app` (Slint UI, трей, окна). Вся логика тестируется без GUI; `tt-app` содержит только привязку к UI и тонкие view-model-функции, которые тоже тестируются.

**Tech Stack:** Rust 1.95 stable (MSVC), Slint 1.18 (`@tr` + bundled `.po`, feature `unstable-winit-030`), rusqlite 0.41 (bundled), chrono 0.4 + chrono-tz 0.10 + iana-time-zone, uuid 1 (v7, serde), serde + serde_json, rust_xlsxwriter 0.102, csv 1.4, tray-icon 0.26, windows-sys 0.61, auto-launch 0.6, rfd (диалоги файлов), ed25519-dalek 3, sha2, base64, hex, ureq 3, sys-locale 0.3, directories 6, log + simplelog, thiserror 2, anyhow 1, winresource (build), tempfile (dev).

**Spec:** `docs/superpowers/specs/2026-11-09-timetracker-design.md` — исполнитель читает спеку и этот план вместе.

## Global Constraints

- Только Windows-сборка тестируется и пакуется; код не использует платформенные API вне `tt-app/src/platform/`.
- Ориентир RAM ≤ 50 МБ private working set в простое (главное окно в трее, мини-таймер показан), CPU ≈ 0 % в простое, холодный старт ≤ 1 с, установщик ≤ 20 МБ.
- Никакого кода, строк и переводов из solidtime (AGPL). solidtime — образец поведения.
- Все пользовательские строки — через `@tr()` в `.slint`; строки, нужные Rust-коду (трей, заголовки экспорта, ошибки), — свойства глобала `Tr` в `ui/strings.slint`, читаются из Rust. Ни одной пользовательской строки литералом в Rust.
- Языки: `en` (исходный и fallback), `ru`, `zh_CN`, `es`; `.po` в `translations/<lang>/LC_MESSAGES/tt-app.po`.
- Время в БД — `INTEGER` unix-миллисекунды UTC; интервалы полуоткрытые `[start, end)`; идущая запись — `end IS NULL`.
- Запись никогда не пересекает локальную полночь (часовой пояс из настроек).
- ID — UUID v7 (`TEXT`, 36 символов). Деньги — `i64` центы; ставка — центы за час.
- Пауза не создаёт записей; типа «перерыв» нет; импорта нет; idle detection нет.
- Обновления: URL манифеста и публичный ключ задаются при сборке (`TT_UPDATE_URL`, `TT_UPDATE_PUBKEY`); без них проверка не выполняется и секция в настройках скрыта.
- Зависимости только MIT/Apache-2.0/BSD/ISC/Zlib/Unicode/MPL-2.0 (+ OFL для шрифта Inter, + Slint Royalty-free); `cargo deny check` в CI.
- Атрибуция Slint (`AboutSlint`) в «О программе» обязательна.
- Логи без пользовательского текста (описаний, имён проектов).
- `cargo clippy --all-targets -- -D warnings` и `cargo fmt --check` чистые на каждом коммите.

## Review Focus

1. **Сон/выключение через несколько полуночей при идущей записи** — после пробуждения запись нарезана на куски по каждому дню, без потерь и дублей (тест в Task 6: `rollover_after_three_days_asleep`).
2. **Полночь, которой нет из-за перехода на летнее время** (America/Sao_Paulo, 2018-11-04 00:00 → 01:00) — граница дня = первый существующий момент нового дня, нет паники (тест в Task 2: `split_at_dst_gap_midnight`).
3. **Краш с идущей записью** — после перезапуска таймер продолжает идти с исходного `start`, сессия в состоянии Running (тест в Task 6: `recover_running_session_after_restart`).
4. **Смена часового пояса в настройках при идущей записи** — уже закрытые записи не переписываются, новая нарезка идёт по новому поясу (тест в Task 6: `rollover_uses_current_timezone`).
5. **Тег-группировка в отчёте при записи с несколькими тегами** — запись попадает в каждую тег-группу, но общий итог не удваивается (тест в Task 4: `group_by_tag_does_not_double_total`).

---

## File Structure

```
Cargo.toml                       # workspace, общие версии зависимостей
deny.toml                        # лицензии
.github/workflows/ci.yml
.gitignore                       # target/, .superpowers/
crates/tt-domain/src/
  lib.rs  ids.rs  model.rs       # Id, Millis, сущности
  days.rs                        # local_date, next_local_midnight, split_at_local_midnights
  session.rs                     # EntryTemplate, session_total, today_total
  billing.rs                     # effective_rate, cost_cents
  rounding.rs                    # RoundingMode, rounded_duration
  report.rs                      # ReportFilter, GroupBy, Interval, aggregate
  format.rs                      # DateFormat…CurrencyFormat, FormatPrefs, format_*
crates/tt-storage/src/
  lib.rs  db.rs                  # Db::open/open_in_memory, migrations
  migrations/001_init.sql
  catalog.rs                     # clients/projects/tasks/tags repos
  entries.rs                     # time_entries + time_entry_tags + timer_sessions
  settings.rs                    # key/value
crates/tt-services/src/
  lib.rs  clock.rs               # trait Clock + SystemClock + FixedClock(test)
  error.rs                       # ServiceError
  timer.rs                       # TimerService
  catalog.rs                     # CatalogService
  entries.rs                     # EntryService
  reports.rs                     # ReportService, DashboardService
  export.rs                      # export_csv, export_xlsx
  settings.rs                    # SettingsService, Settings
  backup.rs                      # backup_to, restore_from
  update.rs                      # UpdateConfig, check_for_update, verify_installer
crates/tt-app/
  build.rs                       # slint_build с bundled translations
  src/main.rs  app.rs            # сборка AppContext, run_event_loop_until_quit
  src/vm/*.rs                    # маппинг domain → Slint-структуры (тестируемый)
  src/controllers/*.rs           # обработчики колбэков глобалов-мостов (*Bridge) → сервисы
  src/platform/{single_instance.rs, autostart.rs, tray.rs, window_ext.rs}
  ui/main.slint  ui/theme.slint  ui/strings.slint
  ui/components/*.slint          # Button, Input, ComboBox, TagSelect, DatePicker, DurationInput, Dialog, Toast, BarChart, PieChart, Heatmap…
  ui/screens/*.slint             # dashboard, time, projects, project, clients, tags, reports, settings
  ui/mini_timer.slint
  assets/fonts/Inter-*.ttf  assets/icons/*.svg  assets/tray/{idle,running,paused}.ico
translations/{ru,zh_CN,es}/LC_MESSAGES/tt-app.po   translations/tt-app.pot
docs/translating.md  README.md  CHANGELOG.md
```

---

## Phase A — логика (TDD)

### Task 1: Workspace, git, CI

**Files:**
- Create: `Cargo.toml`, `crates/{tt-domain,tt-storage,tt-services,tt-app}/Cargo.toml` + пустые `src/lib.rs` (`tt-app`: `src/main.rs` с `fn main() {}`), `.gitignore`, `deny.toml`, `.github/workflows/ci.yml`, `rust-toolchain.toml` (`channel = "1.95"`).

**Interfaces:**
- Produces: workspace с `[workspace.dependencies]` для всех крейтов из Tech Stack; `edition = "2024"`; `[workspace.lints.rust] unsafe_code = "deny"` (кроме `tt-app`, где `platform/` использует `unsafe` под `#[allow]` на модуле).

- [ ] **Step 1:** `git init`, `.gitignore` с `target/`, `.superpowers/`, `*.db`.
- [ ] **Step 2:** Создать workspace и крейты; `tt-domain` зависит ни от чего внутреннего, `tt-storage` → `tt-domain`, `tt-services` → оба, `tt-app` → все три.
- [ ] **Step 3:** `ci.yml`: job на `windows-latest`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check licenses`.
- [ ] **Step 4:** Run: `cargo build --workspace && cargo clippy --all-targets -- -D warnings` → Expected: успешно, 0 предупреждений.
- [ ] **Step 5:** Commit `chore: scaffold workspace, CI, license policy`.

### Task 2: Домен — модель и границы дней

**Files:**
- Create: `crates/tt-domain/src/{ids.rs, model.rs, days.rs}`
- Test: `crates/tt-domain/src/days.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub type Id = uuid::Uuid; pub fn new_id() -> Id` (v7); `pub type Millis = i64;`
  - `pub struct Client { id, name: String, archived_at: Option<Millis>, created_at: Millis, updated_at: Millis }`
  - `pub struct Project { id, name, color: String /* "#rrggbb" */, client_id: Option<Id>, is_billable: bool, billable_rate: Option<i64>, estimated_time: Option<i64> /* сек */, archived_at: Option<Millis>, created_at, updated_at }`
  - `pub struct Task { id, project_id: Id, name, estimated_time: Option<i64>, done_at: Option<Millis>, created_at, updated_at }`
  - `pub struct Tag { id, name, created_at, updated_at }`
  - `pub struct TimeEntry { id, session_id: Option<Id>, description: String, start: Millis, end: Option<Millis>, billable: bool, project_id: Option<Id>, task_id: Option<Id>, tag_ids: Vec<Id>, created_at, updated_at }` + `fn duration_ms(&self, now: Millis) -> i64`
  - `pub enum SessionState { Running, Paused, Stopped }`; `pub struct TimerSession { id, started_at: Millis, ended_at: Option<Millis>, state: SessionState }`
  - `days.rs`: `pub fn local_date(t: Millis, tz: Tz) -> NaiveDate`; `pub fn start_of_local_day(d: NaiveDate, tz: Tz) -> Millis` (первый существующий момент даты); `pub fn next_local_midnight(t: Millis, tz: Tz) -> Millis`; `pub fn split_at_local_midnights(start: Millis, end: Millis, tz: Tz) -> Vec<(Millis, Millis)>`.

- [ ] **Step 1: Тесты**
  - `split_within_one_day`: Europe/Moscow, 2026-11-09 10:00→12:00 → один кусок, равный входу.
  - `split_across_midnight`: Europe/Moscow, 2026-11-09 23:00 → 2026-11-10 01:00 → `[(23:00, 00:00), (00:00, 01:00)]`.
  - `split_across_three_midnights`: 2026-11-09 22:00 → 2026-11-12 02:00 → 4 куска; сумма длительностей = 52 ч; концы и начала соседних кусков совпадают.
  - `split_end_exactly_midnight`: 22:00 → 00:00 следующего дня → один кусок (полуоткрытый интервал).
  - `split_at_dst_gap_midnight`: America/Sao_Paulo, 2018-11-03 22:00 local → 2018-11-04 02:00 local → граница = 2018-11-04 01:00 local (03:00 UTC); 2 куска.
  - `new_id_is_v7_and_sortable`: два последовательных `new_id()` — версия 7, второй > первого.
- [ ] **Step 2:** Run `cargo test -p tt-domain` → FAIL (не определено).
- [ ] **Step 3:** Реализовать. `start_of_local_day`: `d.and_hms(0,0,0).and_local_timezone(tz).earliest()`, при `None` — шагать по 15 минут вперёд до первого существующего момента.
- [ ] **Step 4:** Run `cargo test -p tt-domain` → PASS.
- [ ] **Step 5:** Commit `feat(domain): entities and local-day boundaries`.

### Task 3: Домен — сессии, ставки, округление, форматирование

**Files:**
- Create: `crates/tt-domain/src/{session.rs, billing.rs, rounding.rs, format.rs}`

**Interfaces:**
- Produces:
  - `session.rs`: `pub struct EntryTemplate { description: String, project_id: Option<Id>, task_id: Option<Id>, tag_ids: Vec<Id>, billable: bool }`; `impl EntryTemplate { fn from_entry(&TimeEntry) -> Self }`; `pub fn session_total(entries: &[TimeEntry], now: Millis) -> i64` (мс); `pub fn day_total(entries: &[TimeEntry], day: NaiveDate, now: Millis, tz: Tz) -> i64`.
  - `billing.rs`: `pub fn effective_rate(entry_billable: bool, project_rate: Option<i64>, default_rate: Option<i64>) -> i64`; `pub fn cost_cents(duration_s: i64, rate_cents_per_hour: i64) -> i64` (округление до цента, half-up).
  - `rounding.rs`: `pub enum RoundingMode { Up, Down, Nearest }`; `pub fn rounded_duration_s(start: Millis, end: Millis, mode: RoundingMode, step_min: u32) -> i64`.
  - `format.rs`: `DateFormat { PointDMY, SlashMMDDYYYY, SlashDDMMYYYY, HyphenDDMMYYYY, HyphenMMDDYYYY, HyphenYYYYMMDD }`, `TimeFormat { H12, H24 }`, `IntervalFormat { Decimal, HoursMinutes, HoursMinutesColon, HoursMinutesSecondsColon }`, `NumberFormat { PointComma, CommaPoint, SpaceComma, SpacePoint, ApostrophePoint }`, `CurrencyFormat { IsoBeforeSpace, IsoAfterSpace, SymbolBefore, SymbolAfter, SymbolBeforeSpace, SymbolAfterSpace }` — все с `as_str()/from_str()` (kebab-case ключи как в solidtime: `point-separated-d-m-yyyy` … `symbol-after-with-space`); `pub struct FormatPrefs { date, time, interval, number, currency, currency_code: String, week_start: chrono::Weekday, tz: Tz, hour_suffix: String /* "h" */, minute_suffix: String /* "min" */ }` (суффиксы локализуемые — `tt-app` заполняет их из `Tr`); `fn format_date(NaiveDate, DateFormat) -> String`, `fn format_time(Millis, &FormatPrefs) -> String`, `fn format_duration(seconds: i64, &FormatPrefs) -> String`, `fn format_clock(seconds: i64) -> String` (`H:MM:SS`, для таймера), `fn format_number(f64, decimals: u8, NumberFormat) -> String`, `fn format_money(cents: i64, &FormatPrefs) -> String`, `fn currency_symbol(code: &str) -> &str` (USD `$`, EUR `€`, GBP `£`, RUB `₽`, CNY `¥`, JPY `¥`, иначе сам код).

- [ ] **Step 1: Тесты**
  - `effective_rate`: `(false, Some(5000), Some(3000)) → 0`; `(true, Some(5000), Some(3000)) → 5000`; `(true, None, Some(3000)) → 3000`; `(true, None, None) → 0`.
  - `cost_cents(5400, 5000) == 7500` (1.5 ч × 50.00).
  - Округление, шаг 15, начало 10:00:40 (усекается до 10:00): конец 10:20:00 → `Down` 900, `Up` 1800, `Nearest` 900; конец 10:23:00 → `Nearest` 1800; конец 10:30:00 → `Up` 1800 (на границе не увеличивается).
  - `session_total`: записи 10:00–10:30, 11:00–идёт, now=11:15 → 45 мин.
  - `day_total`: две записи сессии в разные дни → для каждого дня только свои.
  - `format_duration(53106 /*14:45:06*/)`: Decimal + CommaPoint → `"14.75 h"`; HoursMinutes → `"14h 45min"`; HoursMinutesColon → `"14:45"`; HoursMinutesSecondsColon → `"14:45:06"`.
  - `format_date(2026-11-09)`: PointDMY → `"9.11.2026"`; SlashMMDDYYYY → `"11/09/2026"`; HyphenYYYYMMDD → `"2026-11-09"`.
  - `format_number(1234567.891, 2, …)`: PointComma `"1.234.567,89"`, CommaPoint `"1,234,567.89"`, SpaceComma `"1 234 567,89"`, SpacePoint `"1 234 567.89"`, ApostrophePoint `"1'234'567.89"`.
  - `format_money(123450, EUR, SymbolAfterSpace, SpaceComma)` → `"1 234,50 €"`; `IsoBeforeSpace` USD CommaPoint → `"USD 1,234.50"`.
- [ ] **Step 2:** Run `cargo test -p tt-domain` → FAIL.
- [ ] **Step 3:** Реализовать. Округление: `s = floor_to_minute(start)`; `d = end - s`; `step = step_min*60_000`; Down `d/step*step`; Up `ceil`; Nearest `(d + step/2)/step*step`; результат в секундах.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(domain): sessions, billing, rounding, formatting`.

### Task 4: Домен — агрегация отчёта

**Files:**
- Create: `crates/tt-domain/src/report.rs`

**Interfaces:**
- Consumes: `TimeEntry`, `Project`, `Task`, `Client`, `Tag`, `rounded_duration_s`, `effective_rate`, `cost_cents`, `local_date`.
- Produces:
  - `pub enum TagMatch { Any, All }`
  - `pub struct ReportFilter { start: Millis, end: Millis, project_ids: Vec<Id>, task_ids: Vec<Id>, client_ids: Vec<Id>, tag_ids: Vec<Id>, tag_match: TagMatch, billable: Option<bool>, rounding: Option<(RoundingMode, u32)> }` (пустой вектор = без фильтра)
  - `pub enum GroupBy { Project, Task, Client, Tag, Billable, Description, Day }`
  - `pub enum Interval { Day, Week, Month }`
  - `pub enum GroupKey { None, Id(Id), Bool(bool), Text(String), Date(NaiveDate) }`
  - `pub struct GroupRow { key: GroupKey, seconds: i64, cost_cents: i64, children: Vec<GroupRow> }`
  - `pub struct Bucket { start: NaiveDate, seconds: i64 }`
  - `pub struct ReportResult { total_seconds: i64, total_cost_cents: i64, groups: Vec<GroupRow>, buckets: Vec<Bucket> }`
  - `pub struct ReportContext<'a> { projects: &'a HashMap<Id, Project>, tasks: &'a HashMap<Id, Task>, default_rate: Option<i64>, tz: Tz, week_start: Weekday, now: Millis }`
  - `pub fn aggregate(entries: &[TimeEntry], filter: &ReportFilter, g1: GroupBy, g2: Option<GroupBy>, interval: Interval, ctx: &ReportContext) -> ReportResult`

- [ ] **Step 1: Тесты** (фикстура: 2 проекта A(rate 5000, client C1), B(без ставки), default_rate 3000, теги T1,T2)
  - `filters_by_period_project_tag_billable` — каждая ось фильтра отбрасывает ожидаемые записи; `TagMatch::All` требует оба тега.
  - `group_project_then_task` — суммы по проектам и задачам; записи без проекта → `GroupKey::None`.
  - `group_by_tag_does_not_double_total` — запись 1 ч с T1+T2: группы T1=1ч, T2=1ч, `total_seconds`=3600.
  - `cost_uses_project_then_default_rate` — 1 ч billable в A → 5000; 1 ч billable в B → 3000; non-billable → 0.
  - `buckets_fill_gaps_week_interval` — период 3 недели, записи только в 1-й → 3 бакета, начала недель по `week_start=Monday`, пустые = 0.
  - `running_entry_counts_until_now`.
  - `rounding_applied_per_entry` — две записи по 7 мин, Up/15 → total 1800.
  - Сортировка групп: по убыванию секунд, `None` последним.
- [ ] **Step 2:** Run `cargo test -p tt-domain report` → FAIL.
- [ ] **Step 3:** Реализовать в памяти (объём личных данных мал): фильтр → длительность каждой записи (с округлением) → группировка `BTreeMap`/`HashMap` → сортировка. Клиент записи — `projects[project_id].client_id`.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(domain): report aggregation`.

### Task 5: Хранилище SQLite

**Files:**
- Create: `crates/tt-storage/src/{db.rs, catalog.rs, entries.rs, settings.rs}`, `crates/tt-storage/migrations/001_init.sql`

**Interfaces:**
- Produces:
  - `pub struct Db { conn: rusqlite::Connection }`; `Db::open(path: &Path) -> Result<Db>` (WAL, `foreign_keys=ON`, миграции по `PRAGMA user_version`); `Db::open_in_memory() -> Result<Db>`; `Db::transaction<T>(&mut self, f: impl FnOnce(&Tx) -> Result<T>) -> Result<T>`; `pub type Result<T> = std::result::Result<T, StorageError>`.
  - Репозитории — функции над `&rusqlite::Connection` (работают и в транзакции):
    `catalog`: `insert_client/update_client/delete_client/get_client/list_clients`, то же для `project`, `task (list_tasks(project_id))`, `tag`; `count_entries_for_project/task(id)`, `count_projects_for_client(id)`, `count_tasks_for_project(id)`.
    `entries`: `insert_entry(&TimeEntry)`, `update_entry(&TimeEntry)` (перезаписывает теги), `delete_entry(id)`, `get_entry(id)`, `running_entry() -> Option<TimeEntry>`, `entries_in_range(start, end) -> Vec<TimeEntry>` (пересечение с `[start,end)`, включая идущие), `entries_of_session(id)`, `recent_entries(limit)`, `overlapping(start, end, exclude: Option<Id>) -> bool`, `spent_ms_for_project(id, now)`, `spent_ms_for_task(id, now)`; `insert_session/update_session/active_session() -> Option<TimerSession>` (state ≠ Stopped), `set_session_template(id, &EntryTemplate)`, `session_template(id) -> Option<EntryTemplate>` (serde_json в `template_json`; `EntryTemplate` получает `#[derive(Serialize, Deserialize)]`).
    `settings`: `get(key) -> Option<String>`, `set(key, value)`.
- Схема `001_init.sql` — таблицы из спеки §9.2 (без `billable_rate` и `client_id` у записей); у `timer_sessions` дополнительно `template_json TEXT` (шаблон для следующего «Продолжить», см. Task 6); индексы `time_entries(start)`, `time_entries(session_id)`, `time_entries(project_id)`, `time_entries(task_id)`; не более одной идущей записи: `CREATE UNIQUE INDEX one_running ON time_entries("end" IS NULL) WHERE "end" IS NULL`. FK: `tasks.project_id ON DELETE RESTRICT`, `time_entry_tags ON DELETE CASCADE` по обеим сторонам, `time_entries.project_id/task_id ON DELETE RESTRICT`.

- [ ] **Step 1: Тесты** (на `open_in_memory`):
  - `migrations_idempotent` — повторный `open` на файле в `tempdir` не падает, `user_version == 1`.
  - CRUD round-trip для каждой сущности, включая теги записи.
  - `only_one_running_entry` — вставка второй записи с `end=None` → ошибка.
  - `entries_in_range_includes_overlapping_and_running`.
  - `overlapping_excludes_self_and_touching` — `[10,11)` и `[11,12)` не пересекаются.
  - `delete_project_with_entries_is_restricted`.
  - `recover_running_entry_from_file` — записать идущую запись в файл, закрыть `Db`, открыть заново → `running_entry()` возвращает её.
- [ ] **Step 2:** Run `cargo test -p tt-storage` → FAIL.
- [ ] **Step 3:** Реализовать (зависимость dev: `tempfile`).
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(storage): sqlite schema and repositories`.

### Task 6: TimerService — сессии, пауза, нарезка по полночи

**Files:**
- Create: `crates/tt-services/src/{clock.rs, error.rs, timer.rs, settings.rs}`

**Interfaces:**
- Consumes: `tt-storage` репозитории, `split_at_local_midnights`, `EntryTemplate`, `session_total`, `day_total`.
- Produces:
  - `pub trait Clock: Send + Sync { fn now(&self) -> Millis; }`, `SystemClock`, `#[cfg(any(test, feature="test-util"))] FixedClock { set(Millis), advance(ms) }`.
  - `pub enum ServiceError { Storage(StorageError), NotFound, InUse { what: &'static str }, Overlap, Invalid(&'static str), Io(std::io::Error), Update(String) }`.
  - `settings.rs`: `pub struct Settings { language: String, format: FormatPrefs, theme: Theme /* Dark|Light */, mini_timer_visible: bool, mini_timer_idle_opacity: f32 /* 0.3 */, mini_timer_pos: Option<(i32,i32)>, close_to_tray: bool /* true */, autostart: bool, prevent_overlap: bool /* false */, default_rate: Option<i64>, update_auto_check: bool }`; `SettingsService::load(&Db) -> Settings` (значения по умолчанию при отсутствии ключей), `save(&Db, &Settings)`.
  - `pub struct TimerStatus { session: Option<TimerSession>, current: Option<TimeEntry> /* идущая или последняя запись сессии */, template: Option<EntryTemplate>, session_ms: i64, today_ms: i64 }`
  - `pub struct TimerService<C: Clock> { … }` с методами (каждый — одна транзакция):
    - `status(&self, db: &Db, tz: Tz) -> Result<TimerStatus>`
    - `start(&self, db: &mut Db, t: EntryTemplate, tz) -> Result<TimerStatus>` — если есть активная сессия: остановить её; создать сессию Running и запись.
    - `pause(&self, db, tz)` — закрыть идущую запись (`end = now`, с нарезкой), state = Paused. Нет Running → `Invalid("not running")`.
    - `resume(&self, db, tz)` — новая запись по шаблону последней записи сессии, state = Running.
    - `stop(&self, db, tz)` — закрыть запись (если идёт), state = Stopped, `ended_at = now`.
    - `update_template(&self, db, t: EntryTemplate, tz)` — сохранить в `set_session_template` и применить к идущей записи (если есть); закрытые записи не трогаются. `start` записывает шаблон сессии, `resume` берёт `session_template`.
    - `rollover(&self, db, tz) -> Result<bool>` — если идущая запись начинается до `start_of_local_day(today)`: закрыть на каждой полуночи (куски — отдельные записи той же сессии с теми же атрибутами и тегами), текущий кусок остаётся идущим; вернуть `true`, если что-то изменилось. Вызывается из `status`-тика UI каждую секунду и при старте.
    - При `prevent_overlap` старт/продолжение не проверяет пересечения (идущая запись всегда начинается «сейчас»), проверка — только для ручных записей (Task 7).

- [ ] **Step 1: Тесты** (`FixedClock`, `Europe/Moscow`, `open_in_memory`)
  - `start_creates_running_session_and_entry`.
  - `pause_closes_entry_and_records_no_gap` — start 10:00, pause 10:30, resume 11:00, pause 11:15 → 2 записи (10:00–10:30, 11:00–11:15), `session_ms` = 45 мин, никакой записи на промежуток.
  - `stop_after_pause_ends_session`.
  - `start_other_task_stops_current_session` — и при Running, и при Paused.
  - `update_template_applies_to_running_and_future_not_past`.
  - `update_template_while_paused_used_on_resume`.
  - `pause_overnight_resume_next_day` — pause 2026-11-09 18:00, resume 2026-11-10 09:00 → новая запись 10-го; `today_ms` считает только 10-е, `session_ms` — оба дня.
  - `rollover_at_midnight` — start 23:30, now 00:10 → `rollover` → записи 23:30–00:00 и идущая с 00:00.
  - `rollover_after_three_days_asleep` — start 2026-11-09 22:00, now 2026-11-12 08:00 → 3 закрытых куска + идущий с 2026-11-12 00:00; суммарно 58 ч; теги скопированы во все куски.
  - `rollover_uses_current_timezone` — закрытые ранее куски не меняются при смене `tz`.
  - `recover_running_session_after_restart` — start, «перезапуск» (новый `TimerService`, тот же файл БД) → `status` Running, `start` исходный.
  - `pause_when_not_running_is_invalid`.
- [ ] **Step 2:** Run `cargo test -p tt-services timer` → FAIL.
- [ ] **Step 3:** Реализовать.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(services): timer sessions with pause and midnight rollover`.

### Task 7: CatalogService и EntryService

**Files:**
- Create: `crates/tt-services/src/{catalog.rs, entries.rs}`

**Interfaces:**
- Produces:
  - `CatalogService`: `create_client(name) -> Client`, `rename_client`, `set_client_archived(id, bool)`, `delete_client(id)` (есть проекты → `InUse{what:"projects"}`); `create_project(NewProject{ name, color, client_id, is_billable, billable_rate, estimated_time }) -> Project`, `update_project(&Project)`, `set_project_archived`, `delete_project(id)` (задачи → `InUse{"tasks"}`, записи → `InUse{"time_entries"}`); `create_task(project_id, name, estimated_time) -> Task`, `update_task`, `set_task_done(id, bool)`, `delete_task(id)` (записи → `InUse{"time_entries"}`); `create_tag(name) -> Tag`, `rename_tag`, `delete_tag(id)` (отвязывает от записей); `list_*`; `project_stats(id, now) -> (spent_ms: i64, estimated_s: Option<i64>)`.
    Валидация: имя после `trim` непусто и ≤ 255 символов (`Invalid("name")`); цвет `#rrggbb` (`Invalid("color")`); задача принадлежит проекту записи.
  - `EntryService`: `create_manual(t: EntryTemplate, start, end, tz, prevent_overlap) -> Vec<TimeEntry>` (нарезка по полночи; `end <= start` → `Invalid("range")`; при `prevent_overlap` и пересечении → `Overlap`); `update(entry: TimeEntry, tz, prevent_overlap) -> Vec<TimeEntry>` (если новый диапазон пересекает полночь — исходная запись становится первым куском, остальные — новые записи без `session_id`); `delete(id)` (идущую запись удалить нельзя → `Invalid("running")`); `duplicate(id) -> TimeEntry` (те же атрибуты и время, без сессии); `list_days(before: Option<NaiveDate>, days: u32, tz) -> Vec<(NaiveDate, Vec<TimeEntry>, total_ms)>` — пагинация по дням для экрана «Время»; `continue_entry(id)` — делегирует `TimerService::start(EntryTemplate::from_entry)`.
    Новая запись с проектом и `billable` не указан явно — UI подставляет `project.is_billable` (функция `default_billable(project: Option<&Project>) -> bool` здесь же).

- [ ] **Step 1: Тесты**
  - Удаление: клиент с проектами, проект с задачами/записями, задача с записями → `InUse`; тег удаляется и пропадает у записей.
  - `manual_entry_across_midnight_is_split`.
  - `manual_entry_overlap_rejected_only_when_setting_on`.
  - `update_entry_extending_past_midnight_splits`.
  - `cannot_delete_running_entry`.
  - `list_days_paginates_and_sums`.
  - `project_stats_counts_running_entry`.
  - Валидация имени/цвета.
- [ ] **Step 2:** Run `cargo test -p tt-services` → FAIL.
- [ ] **Step 3:** Реализовать.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(services): catalog and entry management`.

### Task 8: Отчёты, обзор, экспорт CSV/XLSX

**Files:**
- Create: `crates/tt-services/src/{reports.rs, export.rs}`

**Interfaces:**
- Consumes: `aggregate`, `ReportFilter`, `GroupBy`, `Interval`.
- Produces:
  - `ReportService::run(db, filter, g1, g2, interval, settings, now) -> ReportResult` (грузит `entries_in_range` + справочники, вызывает `aggregate`).
  - `pub struct Dashboard { recent: Vec<EntryTemplate> /* до 6 уникальных по (description, project, task), по убыванию последнего старта */, last7: Vec<(NaiveDate, i64)>, heatmap: Vec<(NaiveDate, i64)> /* 12 полных недель до текущей включительно, с week_start */, week_by_day: Vec<(NaiveDate, i64)>, week_by_project: Vec<(Option<Id>, i64, i64 /*cost*/)>, week_total_ms: i64, week_cost_cents: i64 }`; `DashboardService::load(db, settings, now) -> Dashboard`.
  - `export.rs`: `pub struct ExportLabels { duration, cost, total, no_project, no_task, no_client, no_tag, yes, no, description, project, task, client, tags, billable, start, end, date }` (заполняет `tt-app` из `Tr`); `export_report_csv(w: impl Write, r: &ReportResult, names: &dyn Fn(&GroupKey) -> String, labels, prefs) -> Result<()>`; `export_entries_csv(w, entries: &[TimeEntry], lookup: &Lookup, labels, prefs)`; `export_report_xlsx(path, r, names, entries, lookup, labels, prefs)` — книга с листами «Summary» (группы, 2 уровня с отступом, итог) и «Entries» (записи). Длительности в XLSX — числа (часы, формат `0.00`) + текстовая колонка по `IntervalFormat`; деньги — числа с 2 знаками.
  - `pub struct Lookup<'a> { projects, tasks, clients, tags: &'a HashMap<Id, _> }`.

- [ ] **Step 1: Тесты**
  - `dashboard_recent_unique_and_ordered`, `dashboard_heatmap_has_84_days_aligned_to_week_start`, `dashboard_week_totals`.
  - `csv_report_round_trip` — прочитать CSV `csv::Reader`ом: заголовки = labels, строки = группы, последняя — итог.
  - `csv_escapes_quotes_and_newlines_in_description`.
  - `csv_starts_with_bom` — первые байты `EF BB BF` (иначе Excel ломает кириллицу и китайский).
  - `xlsx_has_two_sheets` — файл создаётся в `tempdir`, размер > 0, открывается как zip и содержит `xl/worksheets/sheet1.xml` и `sheet2.xml`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Реализовать.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(services): reports, dashboard, csv/xlsx export`.

### Task 9: Бэкап и механизм обновлений (выключен)

**Files:**
- Create: `crates/tt-services/src/{backup.rs, update.rs}`

**Interfaces:**
- Produces:
  - `backup_to(db: &Db, dest: &Path) -> Result<()>` — `VACUUM INTO`; `restore_from(src: &Path, live_path: &Path) -> Result<()>` — проверить, что `src` — SQLite с `user_version ≤ текущей` и таблицей `time_entries`; иначе `Invalid("backup")`; скопировать во временный файл рядом и атомарно переименовать (`Db` перед этим закрывается вызывающим; приложение перезапускает своё `AppContext`).
  - `pub struct UpdateConfig { manifest_url: String, public_key: [u8; 32] }`; `pub fn update_config() -> Option<UpdateConfig>` — из `option_env!("TT_UPDATE_URL")` и `option_env!("TT_UPDATE_PUBKEY")` (hex); любой отсутствует → `None`.
  - `pub struct Manifest { version: semver-строка, url: String, sha256: String, signature: String /* base64 ed25519 над байтами установщика */, notes: Option<String> }` (serde_json).
  - `pub fn is_newer(current: &str, latest: &str) -> bool`; `pub fn check_for_update(cfg: &UpdateConfig, current: &str) -> Result<Option<Manifest>>` (ureq GET, таймаут 10 с); `pub fn download_and_verify(cfg, m: &Manifest, dest: &Path) -> Result<()>` (sha256 + ed25519, при несовпадении файл удаляется, `Update("signature")`).
- [ ] **Step 1: Тесты**
  - `backup_restore_round_trip` (файловая БД в `tempdir`).
  - `restore_rejects_non_sqlite_file` и `restore_rejects_newer_schema`.
  - `update_config_none_without_env` (в тестовой сборке переменные не заданы).
  - `is_newer`: `("1.0.0","1.0.1") → true`, `("1.2.0","1.10.0") → true`, `("1.0.0","1.0.0") → false`.
  - `verify_rejects_tampered_bytes` — подписать тестовым ключом, изменить байт → ошибка; оригинал → ok (функция `verify_bytes(pubkey, bytes, sig_b64) -> Result<()>` выделена для теста без сети).
- [ ] **Step 2:** Run → FAIL. **Step 3:** Реализовать. **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(services): backup/restore and disabled-by-default updater`.

---

## Phase B — приложение (Slint)

Общее для Phase B: UI-логика без I/O выносится в `tt-app/src/vm/*.rs` (маппинг `tt-domain` → Slint-структуры, форматирование строк) и покрывается unit-тестами. Сами экраны проверяются вручную по чек-листу в конце каждой задачи (запуск `cargo run -p tt-app`). Скриншоты чек-листа прикладываются к отчёту задачи.

### Task 10: Каркас приложения — окно, тема, навигация, переводы

**Files:**
- Create: `crates/tt-app/build.rs`, `src/{main.rs, app.rs, vm/mod.rs, vm/format.rs}`, `ui/{main.slint, theme.slint, strings.slint}`, `ui/components/{button.slint, input.slint, icon.slint, sidebar.slint, dialog.slint, toast.slint}`, `assets/fonts/Inter-{Regular,Medium,SemiBold}.ttf`, `assets/icons/*.svg` (Lucide, ISC), `translations/tt-app.pot`, `translations/{ru,zh_CN,es}/LC_MESSAGES/tt-app.po`, `docs/translating.md`.

**Interfaces:**
- Consumes: все сервисы.
- Produces:
  - `pub struct AppContext { db: Db, timer: TimerService<SystemClock>, settings: Settings, db_path: PathBuf }` (+ `fn open_default() -> anyhow::Result<AppContext>` — `%APPDATA%\TimeTracker\timetracker.db` через `directories`); владеет UI-поток, доступ через `Rc<RefCell<AppContext>>`.
  - `build.rs`: `slint_build::compile_with_config("ui/main.slint", CompilerConfiguration::new().with_bundled_translations("../../translations"))`.
  - `theme.slint`: `export global Theme { in-out property <bool> dark: true; out property <color> bg, panel, card, line, muted, text, accent, success, warning, danger; out property <length> radius-sm(6px), radius-md(8px), radius-lg(12px), gap-1(4px)…gap-6(24px); out property <length> font-sm(12px), font-md(13px), font-lg(15px), font-xl(20px); }` — тёмные значения: bg `#0e0f11`, panel `#16181c`, card `#1c1f24`, line `#2a2e35`, muted `#8b919c`, text `#e8eaed`, accent `#3b82f6`, success `#22c55e`, warning `#f59e0b`, danger `#ef4444`; светлые — подобрать в той же гамме.
  - `strings.slint`: `export global Tr { out property <string> tray-pause: @tr("Pause"); tray-resume, tray-stop, tray-show-mini, tray-hide-mini, tray-open, tray-quit, no-active-task, hour-suffix: @tr("h"), minute-suffix: @tr("min"), export-…(все поля ExportLabels), err-in-use-projects, err-in-use-tasks, err-in-use-entries, err-overlap, err-invalid-name, err-invalid-color, err-invalid-range, err-backup; }`.
  - `main.slint`: `export component MainWindow inherits Window` — `title: "TimeTracker"`, `min-width: 960px`, `min-height: 600px`, `default-font-family: "Inter"`; `in-out property <int> page` (0 Обзор, 1 Время, 2 Проекты, 3 Проект, 4 Клиенты, 5 Теги, 6 Отчёты, 7 Настройки); сайдбар с группами как в спеке §7.1; слот под строку таймера сверху (Task 11).
  - `vm/format.rs`: `fn prefs_from(settings: &Settings, tr: &Tr) -> FormatPrefs` (подставляет суффиксы).
  - Логи: `log` + `simplelog::WriteLogger` в `%LOCALAPPDATA%\TimeTracker\logs\tt.log` (уровень Warn; panic hook пишет туда же); в сообщения — только ID и коды ошибок, никаких описаний и имён.
  - Смена языка: `slint::select_bundled_translation(lang)`; при старте — `settings.language`, а если не задан — `sys-locale` → первый из `ru`/`zh_CN`/`es`/`en` по префиксу (`zh*` → `zh_CN`), иначе `en`; сохранить в настройки.
- [ ] **Step 1: Тест** `vm::lang::pick_language`: `"ru-RU"→"ru"`, `"zh-Hans-CN"→"zh_CN"`, `"es-MX"→"es"`, `"de-DE"→"en"`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Реализовать каркас и тему. **Step 4:** Run `cargo test -p tt-app` → PASS.
- [ ] **Step 5: Ручная проверка:** `cargo run -p tt-app` — окно с тёмной темой и сайдбаром, переход по пунктам меню; запуск с `settings.language=ru` показывает русские пункты меню (переводы ru/zh_CN/es для всех строк каркаса заполнены).
- [ ] **Step 6:** Commit `feat(app): window shell, theme, navigation, i18n`.

### Task 11: Строка таймера и компоненты выбора

**Files:**
- Create: `ui/components/{combobox.slint, tag_select.slint, timer_bar.slint, color_dot.slint}`, `src/vm/timer.rs`, `src/controllers/timer.rs`

**Interfaces:**
- Consumes: `TimerService`, `CatalogService::list_*`, `create_project/create_task/create_tag`.
- Produces:
  - Slint-структуры: `struct ProjectOption { id: string, label: string /* "Проект › Задача" или "Проект" */, color: color, project-id: string, task-id: string }`, `struct TagOption { id: string, name: string, selected: bool }`, `struct TimerView { state: int /* 0 stopped,1 running,2 paused */, description: string, project-label: string, project-color: color, tags-label: string, billable: bool, clock: string /* H:MM:SS */, today: string /* пусто, если сессия началась сегодня */ }`.
  - В выбор проекта/задачи не попадают архивные проекты и выполненные задачи.
  - `ComboBox` (поиск по подстроке без учёта регистра, клавиши ↑↓ Enter Esc, пункт «Создать "<текст>"» в конце, если точного совпадения нет); `TagSelect` (мультивыбор + создание).
  - Глобал `TimerBridge { in property <TimerView> view; callback start(); pause(); resume(); stop(); set-description(string); set-project(string /*project-id*/, string /*task-id*/); set-tags([string]); set-billable(bool); create-project(string) -> string; create-task(string /*project-id*/, string) -> string; create-tag(string) -> string; }`.
  - `vm::timer::timer_view(status: &TimerStatus, lookup, prefs, now) -> TimerView`.
  - Тик: `slint::Timer` 1 с вызывает `timer.rollover` и обновляет только `view` (не пересобирает модели). Когда сессии нет — тик не запускается.
- [ ] **Step 1: Тесты** `vm::timer`: состояние Stopped → `clock="0:00:00"`, Paused → `state=2`, `clock` = сумма сессии; `today` пуст, если сессия началась сегодня, и `"1:15:00"`-подобный (по `IntervalFormat`) иначе; `label` = «Проект › Задача».
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** старт с описанием и проектом; пауза → кнопка становится «Продолжить», время стоит; продолжить; стоп; создание проекта/задачи/тега прямо из комбобокса; правка описания во время сессии сохраняется в идущую запись.
- [ ] **Step 6:** Commit `feat(app): timer bar with project/task/tag pickers`.

### Task 12: Экран «Время» — список записей, правка, ручное добавление

**Files:**
- Create: `ui/screens/time.slint`, `ui/components/{entry_row.slint, entry_dialog.slint, date_picker.slint, duration_input.slint}`, `src/vm/entries.rs`, `src/controllers/entries.rs`

**Interfaces:**
- Consumes: `EntryService`, `TimerService::start`, `vm::timer`.
- Produces:
  - `struct EntryRowView { id: string, description: string, project-label: string, project-color: color, tags: string, billable: bool, range: string /* "09:30 – 10:15" */, duration: string, running: bool }`, `struct DayView { title: string /* «Сегодня»/«Вчера»/дата */, total: string, rows: [EntryRowView] }`.
  - Глобал `EntriesBridge { in property <[DayView]> days; callback load-more(); continue(string); edit(string); duplicate(string); delete(string); add-manual(); save(EntryDialogData) -> string /* пусто или текст ошибки */ }`; `struct EntryDialogData { id: string, description: string, project-id: string, task-id: string, tag-ids: [string], billable: bool, date: string, start: string, end: string }`.
  - `vm::entries::parse_time_input(s: &str) -> Option<(u32,u32)>` — принимает `"9:05"`, `"0905"`, `"905"`, `"9"`, `"21:30"`; `"9pm"` при 12-часовом формате.
  - `vm::entries::parse_duration_input(s) -> Option<i64>` — `"1:30"`, `"1.5"`, `"1,5"`, `"90m"`, `"1h 30m"` → 5400 с.
  - Список виртуализирован (`ListView`), подгрузка по 14 дней при прокрутке к концу.
- [ ] **Step 1: Тесты** `parse_time_input`, `parse_duration_input` (все примеры выше + мусор → `None`), `day_title` (сегодня/вчера/иначе формат даты), сумма дня с идущей записью.
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** группы по дням с суммами; ▶ продолжает задачу; правка времени через полночь создаёт две записи; дублирование; удаление с подтверждением; удаление идущей записи недоступно; ручное добавление с ошибкой пересечения при включённой настройке.
- [ ] **Step 6:** Commit `feat(app): time entries screen`.

### Task 13: Мини-таймер

**Files:**
- Create: `ui/mini_timer.slint`, `src/platform/window_ext.rs`, `src/controllers/mini.rs`

**Interfaces:**
- Consumes: `TimerBridge` (тот же глобал, тот же `TimerView`), `Settings.mini_timer_*`.
- Produces:
  - `export component MiniTimer inherits Window { no-frame: true; always-on-top: true; background: transparent; width: 240px; height: 88px; in property <float> idle-opacity; }` — корневой `Rectangle` с `opacity: area.has-hover ? 1.0 : root.idle-opacity; animate opacity { duration: 150ms; }`, ручка ⠿ (`TouchArea` → перетаскивание окна через `window_ext::start_drag`), клик по тексту → `open-main()`; при `state == 0` — «Нет активной задачи» + ▶ старт последней записи.
  - `window_ext.rs` (feature `slint/unstable-winit-030`): `fn hide_from_taskbar(w: &slint::Window)` (winit `WindowAttributesExtWindows::with_skip_taskbar(true)` через `BackendSelector::with_winit_window_attributes_hook` или `set_skip_taskbar`), `fn start_drag(w: &slint::Window)` (`winit::Window::drag_window()`), `fn position(w) -> (i32,i32)`, `fn set_position(w, (i32,i32))` с проверкой, что точка внутри какого-либо монитора, иначе — правый верхний угол основного монитора с отступом 24 px.
  - Позиция сохраняется в `settings.mini_timer_pos` при окончании перетаскивания.
- [ ] **Step 1: Тест** `controllers::mini::clamp_to_monitors(pos, monitors: &[Rect]) -> (i32,i32)`: точка вне всех мониторов → правый верхний угол первого монитора минус (240+24, −24).
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** окно без рамки поверх других программ, нет кнопки на панели задач; без мыши ~30 % видимости, при наведении 100 % плавно; перетаскивание и восстановление позиции после перезапуска; пауза/стоп синхронны с главным окном; скрытие/показ из главного окна.
- [ ] **Step 6:** Commit `feat(app): floating mini timer`.

### Task 14: Трей, одна копия, закрытие в трей, автозапуск

**Files:**
- Create: `src/platform/{tray.rs, single_instance.rs, autostart.rs}`, `assets/tray/{idle,running,paused}.ico`

**Interfaces:**
- Consumes: `TimerBridge`-обработчики, `Tr` строки.
- Produces:
  - `tray.rs`: `pub struct Tray { … }`; `Tray::new(strings: TrayStrings) -> Result<Tray>`; `fn set_state(&self, state: u8, tooltip: &str)` (иконка + тултип + текст первого неактивного пункта + «Пауза»/«Продолжить» + доступность «Стоп»); события меню через `MenuEvent::set_event_handler` → `slint::invoke_from_event_loop` → `AppAction { Pause, Resume, Stop, ToggleMini, OpenMain, Quit }`; левый клик по иконке (`TrayIconEvent::Click { button: Left, button_state: Up }`) → показать/скрыть главное окно. Обновлять тултип раз в секунду не нужно — раз в минуту и при смене состояния.
  - `single_instance.rs`: `pub fn acquire() -> Option<InstanceGuard>` — `CreateMutexW("Local\\TimeTracker.Instance")`; при `ERROR_ALREADY_EXISTS` → `signal_existing()` (`OpenEventW` + `SetEvent` на `"Local\\TimeTracker.Activate"`) и `None` → процесс завершается; `InstanceGuard::on_activate(f)` — поток `WaitForSingleObject` → `invoke_from_event_loop(f)`.
  - `autostart.rs`: `pub fn set_autostart(enabled: bool) -> Result<()>` (крейт `auto-launch`, HKCU Run, аргумент `--minimized`); `--minimized` при старте не показывает главное окно.
  - Главное окно: `on_close_requested` → при `close_to_tray` скрыть (`HideWindow`), иначе выход; цикл — `slint::run_event_loop_until_quit()`; «Выход» → `slint::quit_event_loop()` (идущая сессия не трогается).
- [ ] **Step 1: Тест** `tray::tooltip_text(state, label, clock)`: Stopped → строка `no-active-task`; Running → `"<label> — 1:24:07"`; label обрезан до 100 символов (лимит тултипа Windows 127).
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** ✕ прячет в трей; иконка меняется running/paused/idle; пункты меню работают; левый клик открывает окно; второй запуск exe активирует первое окно; автозапуск ставится и снимается (проверить ключ в `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`); «Выход» и повторный запуск — таймер продолжает идти.
- [ ] **Step 6:** Commit `feat(app): tray, single instance, close-to-tray, autostart`.

### Task 15: Экраны проектов, страницы проекта, клиентов, тегов

**Files:**
- Create: `ui/screens/{projects.slint, project.slint, clients.slint, tags.slint}`, `ui/components/{project_dialog.slint, color_picker.slint, progress.slint, confirm_dialog.slint}`, `src/vm/catalog.rs`, `src/controllers/catalog.rs`

**Interfaces:**
- Consumes: `CatalogService`, `TimerService::start`.
- Produces:
  - `struct ProjectRowView { id, name, color, client: string, spent: string, estimate: string, progress: float /* 0..1, -1 если оценки нет */, billable-rate: string, archived: bool }`, `struct TaskRowView { id, name, spent, estimate, progress, done: bool }`, `struct ClientRowView { id, name, projects: int, archived: bool }`, `struct TagRowView { id, name, entries: int }`.
  - Глобал `CatalogBridge { … callbacks: create/update/archive/delete для каждой сущности, open-project(string), start-task(string /*task-id*/), toggle-task-done(string); in property search: string; show-archived: bool }`; ошибки `InUse` показываются тостом с текстом из `Tr` и предложением архивировать.
  - Палитра цветов проекта — 12 фиксированных цветов + ввод hex.
  - `vm::catalog::progress(spent_ms, estimate_s) -> f32` (обрезается до 1.0 для полосы; текст показывает реальные проценты, >100 % подсвечивается `warning`).
- [ ] **Step 1: Тесты** `progress` (нет оценки → −1; 150 % → полоса 1.0, флаг превышения), фильтр поиска без учёта регистра по имени и клиенту.
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** CRUD всех сущностей; архив/разархив; удаление занятого проекта → тост; на странице проекта ▶ у задачи стартует таймер с этой задачей; «выполнено» скрывает задачу из комбобокса таймера.
- [ ] **Step 6:** Commit `feat(app): projects, tasks, clients, tags screens`.

### Task 16: Диаграммы и экран «Обзор»

**Files:**
- Create: `ui/components/{bar_chart.slint, pie_chart.slint, heatmap.slint, card.slint}`, `ui/screens/dashboard.slint`, `src/vm/charts.rs`, `src/controllers/dashboard.rs`

**Interfaces:**
- Consumes: `DashboardService::load`.
- Produces:
  - `struct BarView { label: string, value: float /* 0..1 от максимума */, caption: string }`, `BarChart { in property <[BarView]> bars; }` (прямоугольники + подписи, `TouchArea` с тултипом).
  - `struct PieSlice { start-angle: float, sweep: float /* градусы */, color: color, label: string, caption: string }`, `PieChart` — сегменты `Path` (дуги); `vm::charts::pie_slices(values: &[(String, i64, Color)]) -> Vec<PieSlice>` (сумма sweep = 360; один элемент → полный круг; нули пропускаются).
  - `struct HeatCell { level: int /* 0..4 */, caption: string }`, `Heatmap { in property <[HeatCell]> cells; }` (7 строк × 12 колонок); `vm::charts::heat_level(ms, max_ms) -> i32` (0 при 0; иначе 1..=4 по квартилям от max).
  - Обзор: карточки «Недавние задачи» (клик → старт), «Последние 7 дней», «Активность», «Эта неделя» (BarChart по дням + таблица по проектам с суммами), «По проектам» (PieChart). Обновление — при открытии экрана и при остановке/паузе таймера.
- [ ] **Step 1: Тесты** `pie_slices` (сумма 360, порядок сохраняется, нули пропущены), `heat_level` (0, max → 4, границы квартилей), `bars_normalized` (максимум → 1.0, все нули → 0 без деления на 0).
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** на тестовой базе (сгенерировать скриптом `cargo run -p tt-app --example seed` — пример добавить в эту задачу, 300 записей за 90 дней) все карточки заполнены, диаграммы читаемы в тёмной и светлой теме, «Недавние» стартуют таймер.
- [ ] **Step 6:** Commit `feat(app): dashboard with charts`.

### Task 17: Экран «Отчёты» и экспорт

**Files:**
- Create: `ui/screens/reports.slint`, `ui/components/{date_range_picker.slint, multi_select.slint, report_table.slint}`, `src/vm/report.rs`, `src/controllers/reports.rs`

**Interfaces:**
- Consumes: `ReportService::run`, `export_report_csv`, `export_report_xlsx`, `BarChart`, `PieChart`.
- Produces:
  - Пресеты периода: `vm::report::preset_range(p: Preset, today: NaiveDate, week_start) -> (NaiveDate, NaiveDate)` для `Today, Yesterday, ThisWeek, LastWeek, ThisMonth, LastMonth, ThisYear` (конец включительно; в `ReportFilter` переводится в `[start_of_day(from), start_of_day(to+1))`).
  - `struct ReportRowView { level: int, label: string, color: color, duration: string, cost: string }` (дерево сплющено в список с уровнем).
  - Глобал `ReportsBridge { in-out property filters…, group1: int, group2: int /* -1 = нет */, interval: int, rounding-mode: int /* -1 нет */, rounding-step: int; callback run(); export-csv(); export-xlsx(); }`; экспорт — системный диалог сохранения (`rfd`, MIT) с именем `timetracker-report-YYYY-MM-DD.csv|xlsx`.
  - Состояние фильтров отчёта запоминается в `settings` (`report.last`, JSON).
- [ ] **Step 1: Тесты** `preset_range` для всех пресетов при `week_start` Monday и Sunday, включая 1 января и 31 декабря; `flatten_rows` (порядок, уровни, итог последним).
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** фильтры по всем осям, группировки «проект → день» показывает время по задаче в каждый день; округление меняет итог; экспорт CSV открывается в Excel с правильной кодировкой (UTF-8 BOM) и XLSX открывается в Excel/LibreOffice.
- [ ] **Step 6:** Commit `feat(app): reports screen with csv/xlsx export`.

### Task 18: Экран «Настройки» и «О программе»

**Files:**
- Create: `ui/screens/settings.slint`, `src/controllers/settings.rs`

**Interfaces:**
- Consumes: `SettingsService`, `set_autostart`, `backup_to`, `restore_from`, `update_config`, `check_for_update`, `download_and_verify`.
- Produces:
  - Разделы: Язык (en/ru/中文/es — названия языков на самих языках, без перевода), Форматы (дата, время, интервал, числа, валюта + код валюты, первый день недели, часовой пояс — список `chrono_tz::TZ_VARIANTS` с поиском), Тема, Мини-таймер (показ, ползунок прозрачности в покое 0–90 %, по умолчанию 70 %), Поведение (закрывать в трей, автозапуск, запрещать пересечения), Оплата (ставка по умолчанию), Данные (резервная копия → диалог сохранения; восстановление → диалог открытия + подтверждение, затем переоткрытие `AppContext`), Обновления (виден только если `update_config().is_some()`), О программе (версия из `CARGO_PKG_VERSION`, `AboutSlint`).
  - Все изменения применяются сразу (язык — `select_bundled_translation`, тема — `Theme.dark`, форматы — пересборка видимых моделей).
  - Ползунок прозрачности показывает «прозрачность», хранится `mini_timer_idle_opacity = 1 − прозрачность`.
- [ ] **Step 1: Тест** `controllers::settings::opacity_from_transparency(70) == 0.3` и обратно; `language_label` не проходит через `@tr`.
- [ ] **Step 2:** FAIL → **Step 3:** реализовать → **Step 4:** PASS.
- [ ] **Step 5: Ручная проверка:** переключение языка мгновенно меняет все экраны, трей-меню и мини-таймер (4 языка); на `zh_CN` иероглифы отображаются (системный fallback-шрифт), а ввод описания через китайский IME работает; смена формата даты/интервала видна на «Времени» и «Отчётах»; бэкап и восстановление на копии БД; секция обновлений скрыта в обычной сборке и видна при сборке с `TT_UPDATE_URL`/`TT_UPDATE_PUBKEY`; «О программе» показывает атрибуцию Slint.
- [ ] **Step 6:** Commit `feat(app): settings and about`.

### Task 19: Производительность, установщик, документация

**Files:**
- Create: `crates/tt-app/Cargo.toml` (секция `[package.metadata.packager]` или `wix/main.wxs`), `README.md`, `CHANGELOG.md`, `docs/perf.md`; Modify: `.github/workflows/ci.yml` (job сборки установщика по тегу `v*`).

**Interfaces:**
- Consumes: всё.

- [ ] **Step 1: Замер рендереров.** Собрать `--release` с `slint` features `renderer-femtovg` и отдельно `renderer-software`; на сид-базе (Task 16) в состоянии «главное окно в трее, мини-таймер показан, таймер идёт» 5 минут — записать private working set (Диспетчер задач / `Get-Process tt-app | Select WorkingSet64, PrivateMemorySize64`) и средний CPU в `docs/perf.md`. Выбрать рендерер с меньшей памятью при приемлемом качестве текста; если > 50 МБ — включить уничтожение главного окна при уходе в трей (пересоздание при открытии) и замерить снова. Результаты и решение — в `docs/perf.md`.
- [ ] **Step 2:** `[profile.release] lto = "thin", codegen-units = 1, strip = true, opt-level = "s"`(сравнить с `3` по памяти и размеру; записать в `docs/perf.md`); `#![windows_subsystem = "windows"]` в `main.rs`; иконка exe через `winresource` в `build.rs`.
- [ ] **Step 3:** Установщик per-user (MSI через `cargo-wix` с `InstallScope="perUser"` либо NSIS через `cargo-packager`) — Run: команда сборки установщика → Expected: файл ≤ 20 МБ; установка без UAC; ярлык в меню «Пуск»; удаление через «Приложения» удаляет программу, но не `%APPDATA%\TimeTracker`.
- [ ] **Step 4:** `README.md` (что это, возможности v1, сборка, запуск тестов, где данные, как включить обновления при сборке), `CHANGELOG.md` (Keep a Changelog, `## [0.1.0]`), `docs/translating.md` уже есть — проверить актуальность.
- [ ] **Step 5:** Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --workspace && cargo deny check licenses` → Expected: всё зелёное.
- [ ] **Step 6:** Commit `chore: release profile, installer, docs, perf report`.
