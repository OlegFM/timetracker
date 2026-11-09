# Translating TimeTracker

The UI uses Slint's `@tr()` with gettext catalogs that are compiled into the binary.

- Source language: English (the strings in `crates/tt-app/ui/**/*.slint`).
- Template: `translations/tt-app.pot`.
- Catalogs: `translations/<lang>/LC_MESSAGES/tt-app.po` (gettext domain `tt-app` =
  crate name). Shipped: `ru`, `zh_CN`, `es`.
- Strings carry no implicit context. A context is used only where one English word
  needs different translations, written as `@tr("export" => "Start")` (`msgctxt "export"`).
- Plurals: `@tr("{n} hour" | "{n} hours" % n)` → `msgid` / `msgid_plural` / `msgstr[i]`;
  each `.po` declares its `Plural-Forms`.

## Fix or complete a translation

Edit the language's `.po` in Poedit (or any text editor) and rebuild. Keep the
`msgid`s unchanged.

## Add a string (developers)

1. Use `@tr("…")` in a `.slint` file. Strings needed by Rust code (tray, export headers,
   errors) go into the `Tr` global in `crates/tt-app/ui/strings.slint`.
2. Add the entry to `tt-app.pot` and, translated, to every `.po`.
3. `cargo test -p tt-app --test translations` checks that every `@tr` string is in the
   template and translated in every catalog, and that no catalog has stale entries.

## Add a language

1. Copy `translations/tt-app.pot` to `translations/<lang>/LC_MESSAGES/tt-app.po`
   (e.g. `de`, `pt_BR`), set the `Language` and `Plural-Forms` headers, translate all
   entries (Poedit: "Create new translation" from the `.pot`).
2. Add the code to `SUPPORTED` and `pick_language` in `crates/tt-app/src/vm/lang.rs`
   and to `LANGS` in `crates/tt-app/tests/translations.rs`.
3. Run `cargo test -p tt-app` and open a pull request.
