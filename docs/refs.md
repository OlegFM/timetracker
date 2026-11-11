# References

## Windows code signing (looked up 2026-11-10)

- [Microsoft: code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
  - Store MSIX packages are re-signed by Microsoft: no SmartScreen warning, no certificate needed.
  - A Win32 MSI/EXE installer in the Store must be signed by the publisher.
  - Outside the Store, a new app warns on first download until it has a download history, however it is signed.
- [Free Store registration for individual developers](https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/)
  (September 2025): sign in with a personal Microsoft account, no fee.
- [Azure Artifact Signing (Trusted Signing) quickstart](https://learn.microsoft.com/azure/trusted-signing/quickstart)
  and [pricing](https://azure.microsoft.com/pricing/details/artifact-signing/).
  - Individual developers only in the USA and Canada.
  - From USD 9.99/month.
- [SignPath Foundation terms](https://signpath.org/terms)
  - Free signing for open-source projects: OSI license, public repository, MFA.
  - The publisher shown is "SignPath Foundation".
- [Certum code signing (SimplySign, cloud key), reseller listing](https://sslpoint.com/code-signing-certificates)
  - Individuals allowed, roughly EUR 109–169 a year.
  - Prices unverified on certum.eu.

## Linux support (looked up 2026-11-11)

- [tray-icon](https://docs.rs/tray-icon) and its Linux backend [ksni](https://docs.rs/ksni)
  - The tray goes through D-Bus, with no GTK and no AppIndicator library.
- [StatusNotifierItem specification](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/)
  - The protocol behind the tray icon; GNOME needs the AppIndicator / KStatusNotifierItem extension to host it.
- [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry-spec/latest/)
  - The format of `timetracker.desktop`.
- [Desktop Application Autostart Specification](https://specifications.freedesktop.org/autostart-spec/latest/)
  - `~/.config/autostart/*.desktop`.
- [appimagetool 1.9.1](https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage)
  - Pinned in the build script. sha256 `ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0` (from the GitHub API asset digest).
- [type2-runtime 20251108, `runtime-x86_64`](https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64)
  - Embedded into the AppImage via `--runtime-file`; sha256 `2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d` (from the GitHub release asset digest).
- [cargo-deb 3.8.0](https://github.com/kornelski/cargo-deb)
  - Builds the `.deb` from the deb metadata in `crates/tt-app/Cargo.toml`.

## macOS support (looked up 2026-11-11)

- [`applicationShouldHandleReopen:hasVisibleWindows:`](https://developer.apple.com/documentation/appkit/nsapplicationdelegate/applicationshouldhandlereopen(_:hasvisiblewindows:))
  - Sent when the user opens an already running app again (Finder, Launchpad, Spotlight); we show the main window.
- [`NSApplication.setActivationPolicy(_:)`](https://developer.apple.com/documentation/appkit/nsapplication/setactivationpolicy(_:))
  - Regular (Dock icon) while the main window is shown, Accessory (no Dock icon, not in Cmd+Tab) otherwise.
- [App Translocation (Gatekeeper path randomization)](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web)
  - A quarantined app run from Downloads or a disk image runs from a read-only temporary copy, so an autostart path to it would break.
- [Apple Icon Image format (ICNS)](https://en.wikipedia.org/wiki/Apple_Icon_Image_format)
  - The layout of `app.icns`, produced by `gen_icon.py`.
- [`hdiutil` man page](https://ss64.com/mac/hdiutil.html)
  - `hdiutil create` packs the `.app` folder into the compressed `.dmg`.
- [Safely open apps on your Mac](https://support.apple.com/en-us/102445)
  - Gatekeeper first-start steps: "Open Anyway" in Privacy & Security (macOS 15), right-click → Open (macOS 11-14).
- [Information property list keys](https://developer.apple.com/documentation/bundleresources/information-property-list)
  - `Info.plist` keys of the bundle, such as `LSMinimumSystemVersion`.
- [GitHub-hosted runners](https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners)
  - `macos-14` was retired; CI uses `macos-15`.
