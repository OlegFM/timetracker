# Idea: Microsoft Store (MSIX) release

Status: idea, after v0.1.0. Not designed yet. Recorded 2026-11-10.

## Why

The installer is unsigned, so users see a SmartScreen warning. A Store MSIX package is
re-signed by Microsoft: no warning, no certificate to buy. Store registration is free for
individual developers. Sources are in [../refs.md](../refs.md).

Comparison (2026-11-10):

- **Microsoft Store, MSIX.** Free and no warning, but the app has to be packaged as MSIX.
- **SignPath Foundation.** Free, but needs a public repository, and SignPath Foundation is
  shown as the publisher.
- **Own certificate** (for example Certum, about EUR 109–169 a year). New apps still warn
  until they have a download history.
- **Azure Artifact Signing.** Individuals only in the USA and Canada.

## What has to change

- **Packaging.** Build an MSIX (app manifest, identity from Partner Center, logos) in CI.
- **Autostart.** A packaged app should use the `StartupTask` extension instead of the
  registry `Run` value.
- **Data folder.** Writes to `%APPDATA%` from a packaged app are redirected to a
  per-package folder.
  - Decide whether that is fine.
  - Check how existing data from the NSIS build is found.
- **Updates.** The Store updates the app, so the built-in update check stays off in the
  Store build.
- **Keep the NSIS installer and the portable zip** on GitHub as the unsigned alternative.

## Open questions

- Same identity for NSIS and Store installs on one machine, or treat them as separate apps?
- Store listing texts and screenshots in en, ru, es, zh.
