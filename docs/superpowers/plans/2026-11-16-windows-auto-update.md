# Windows Auto-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The installed Windows build finds the latest stable GitHub release, verifies a signed manifest, installs it silently on one click and restarts as the new version. CI prepares that release as a draft.

**Architecture:**
- `tt-services::update` owns the manifest format and does all checks. The format is a JSON file plus a detached ed25519 signature over its exact bytes. The module also does version comparison with pre-releases and the network calls.
- A thin bin `tt-release` in `tt-services` reuses that code to sign manifests and check tag versions in CI.
- The app side moves into a new `controllers/updates.rs`. The NSIS installer gets a `/UPDATE` flag that relaunches the app after a silent install.

**Tech Stack:** Rust 1.95, Slint, ureq 3, ed25519-dalek 3, sha2, serde/serde_json, windows-sys 0.61, NSIS 3, GitHub Actions + `gh`.

**Spec:** [docs/superpowers/specs/2026-11-15-windows-auto-update-design.md](../specs/2026-11-15-windows-auto-update-design.md)

## Global Constraints

- **Workflow and style:**
  - Work on branch `feature/windows-auto-update`.
  - Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - KISS: the spec is the scope.
- **Dependencies:** no new crates. Enabling a feature of an existing crate (windows-sys) is allowed.
- **Gates for every task:**
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test --workspace`
- **Platforms:** updates stay Windows-only, so `update_config()` is `None` when `cfg!(not(windows))`. The workspace must still build and test on Linux and macOS.
- **Manifest address:** `https://github.com/OlegFM/timetracker/releases/latest/download/manifest.json`. The signature sits at the same URL plus `.sig`.
- **Manifest fields:** exactly `version`, `url`, `sha256` (64 lowercase hex), `page`. No `notes`.
- **Signature:** base64 (standard) of an ed25519 signature over the exact bytes of `manifest.json`, checked with `verify_strict` before parsing.
- **Size limits:**
  - manifest 64 KiB;
  - signature file 1 KiB;
  - installer 512 MiB (existing).
- **https only:**
  - `url` and `page` must start with `https://`;
  - the ureq agents stay `https_only`, and the timeouts stay as they are.
- **Secrets:**
  - The private key comes only from env `TT_UPDATE_SIGNING_KEY` (64 hex = 32-byte seed), never from argv. It is never logged.
  - Repo variables are `TT_UPDATE_URL` and `TT_UPDATE_PUBKEY`.
- **Version and temp file:**
  - The current version is `option_env!("TT_BUILD_VERSION")`, falling back to `env!("CARGO_PKG_VERSION")`.
  - Temp installer: `%TEMP%\TimeTracker-update.exe`, started with the arguments `/S /UPDATE`.
- **Logging:** log only error codes (`vm::error::error_code`), never URLs or bodies, as now.
- **UI strings:** every new `@tr` string goes into `translations/tt-app.pot` and the ru, zh_CN and es `.po` files (`docs/translating.md`).

## Review Focus

1. **Release without a manifest.** GitHub `latest` points to a release that has no `manifest.json` or `.sig` (404), such as the first stable release published before the keys exist. Expected: a quiet start-up check shows nothing; "Check now" shows "Couldn't check for updates." Test: `verify_manifest_rejects_missing_or_garbage_signature` (Task 2).
2. **Newest stable is older than the running version.** The user runs an rc ahead of the last stable, or a stable release was pulled from `latest`. Expected: nothing is offered. Test: the `is_newer` table rows `0.4.0-rc.1` vs `0.3.0` and `0.4.1` vs `0.4.0` (Task 1).
3. **Leftovers in temp.** The app was closed during a download, or the installer still holds the file. A leftover `TimeTracker-update.exe.part` or `.exe` must be removed on the next start without an error. Test: `cleanup_removes_installer_and_part` (Task 4).
4. **Double click on Install or Check now.** A double click, or a click while the 24 h check runs, must not start a second download or check. Test: `busy_states_block_new_work` (Task 4).
5. **Signed manifest with an unsafe address.** A manifest whose `url` or `page` is `http:` or `file:` must never be opened or downloaded. Test: `verify_manifest_rejects_insecure_addresses` (Task 2).

---

### Task 1: Versions with pre-releases

**Files:**
- Modify: `crates/tt-services/src/update.rs` (`components`, `is_newer`, tests)

**Interfaces:**
- Produces:
  - `pub fn is_newer(current: &str, latest: &str) -> bool`, with the same signature but new rules.
  - `pub fn current_version() -> &'static str`
  - `pub fn tag_matches_version(tag: &str, cargo_version: &str) -> bool`

- [ ] **Step 1: Write the failing tests** in `update.rs` `mod tests`

```rust
#[test]
fn is_newer_with_pre_releases() {
    let ordered = ["0.3.0", "0.4.0-rc.1", "0.4.0-rc.2", "0.4.0-rc.10", "0.4.0", "0.4.1"];
    for w in ordered.windows(2) {
        assert!(is_newer(w[0], w[1]), "{} < {}", w[0], w[1]);
        assert!(!is_newer(w[1], w[0]), "{} > {}", w[1], w[0]);
    }
    assert!(!is_newer("0.4.0", "0.4.0"));
    assert!(!is_newer("0.4.0-rc.1", "0.4.0-rc.1"));
    assert!(is_newer("v1.0", "1.0.1"));
    assert!(is_newer("1.2.0", "1.10.0"));
    assert!(is_newer("1.0.0-beta.2", "1.0.0-rc.1"));   // strings compare as text
    assert!(is_newer("1.0.0-rc.1", "1.0.0-rc.a"));     // a number part is lower than a text part
    for bad in ["x", "", "1.0.x", "1..0", "1.0.0-", "1.0.0-rc..1"] {
        assert!(!is_newer(bad, "9.9.9"), "{bad}");
        assert!(!is_newer("0.0.1", bad), "{bad}");
    }
}

#[test]
fn tag_must_match_the_cargo_version() {
    assert!(tag_matches_version("v0.4.0", "0.4.0"));
    assert!(tag_matches_version("v0.4.0-rc.1", "0.4.0"));
    assert!(!tag_matches_version("v0.4.1", "0.4.0"));
    assert!(!tag_matches_version("0.4.0", "0.4.0"));     // the tag needs the leading v
    assert!(!tag_matches_version("v0.4.0-", "0.4.0"));
}

#[test]
fn current_version_defaults_to_the_crate_version() {
    if option_env!("TT_BUILD_VERSION").is_none() {
        assert_eq!(current_version(), env!("CARGO_PKG_VERSION"));
    }
}
```

Remove the old `is_newer_cases` test. Its row `!is_newer("1.0.0", "1.0.0-beta")` still holds under the new rules and is covered by the table above.

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p tt-services update::`
Expected: FAIL. `tag_matches_version` and `current_version` are not defined.

- [ ] **Step 3: Implement**
  - `is_newer` follows spec §5.3. Parse `[v]X(.Y)*[-pre]`: the core is dot-separated `u64` with missing parts as 0, and `pre` is a list of non-empty dot-separated identifiers. Any parse failure means `false`.
  - Ordering: compare the core first. With an equal core, no `pre` beats a `pre`. Two `pre` lists compare item by item: numeric against numeric as numbers, text against text as strings, numeric < text. A shorter list is lower when it is a prefix of the longer one.
  - `current_version()`: `option_env!("TT_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))`.
  - `tag_matches_version`: the tag must start with `v`. The rest must parse as a version, and its core (the part before `-`) must equal `cargo_version`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -p tt-services update::`
Expected: PASS

- [ ] **Step 5: Commit**: `feat(update): compare pre-release versions and read the build version`

---

### Task 2: Signed manifest format and the network check

**Files:**
- Modify: `crates/tt-services/src/update.rs` (serde `derive` is already on in the workspace)

**Interfaces:**
- Consumes: `is_newer`, `current_version` (Task 1)
- Produces:
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct Manifest { pub version: String, pub url: String, pub sha256: String, pub page: String }`
  - `pub fn verify_manifest(json: &[u8], sig_b64: &str, pubkey: &[u8; 32]) -> Result<Manifest>`
  - `pub fn sign_manifest(seed: &[u8; 32], json: &[u8]) -> String`, which returns the base64 signature.
  - `pub fn public_key_hex(seed: &[u8; 32]) -> String`
  - `pub fn manifest_json(m: &Manifest) -> Vec<u8>`, which is `serde_json::to_vec_pretty`.
  - `pub fn check_for_update(cfg: &UpdateConfig, current: &str) -> Result<Option<Manifest>>`, with the same signature as before.
  - `pub fn download_and_verify(m: &Manifest, dest: &Path) -> Result<()>`. The `cfg` parameter is dropped, because only sha256 is checked now.
  - `pub fn is_installed_copy(exe: &Path) -> bool`, which checks for `exe.parent()/uninstall.exe`.

- [ ] **Step 1: Write the failing tests** (replace `verify_rejects_tampered_bytes`, `signed_manifest`, `verify_download_cases` and `rejects_insecure_urls`)

```rust
const SEED: [u8; 32] = [7u8; 32];

fn sample() -> Manifest {
    Manifest {
        version: "0.4.0".into(),
        url: "https://x/TimeTracker-0.4.0-setup.exe".into(),
        sha256: hex::encode(Sha256::digest(b"installer")),
        page: "https://x/releases/tag/v0.4.0".into(),
    }
}

fn pk() -> [u8; 32] { hex::decode(public_key_hex(&SEED)).unwrap().try_into().unwrap() }

#[test]
fn signed_manifest_round_trips() {
    let json = manifest_json(&sample());
    let sig = sign_manifest(&SEED, &json);
    assert_eq!(verify_manifest(&json, &sig, &pk()).unwrap(), sample());
}

#[test]
fn verify_manifest_checks_the_signature_before_parsing() {
    let json = manifest_json(&sample());
    let sig = sign_manifest(&SEED, &json);
    let mut tampered = json.clone();
    let i = tampered.iter().position(|&b| b == b'4').unwrap();
    tampered[i] = b'9';                                   // 0.4.0 -> 0.9.0
    assert!(matches!(verify_manifest(&tampered, &sig, &pk()),
        Err(ServiceError::Update(s)) if s == "signature"));
    let other = sign_manifest(&[8u8; 32], &json);
    assert!(matches!(verify_manifest(&json, &other, &pk()),
        Err(ServiceError::Update(s)) if s == "signature"));
    // Signed but not a manifest: the signature passes, the parse fails.
    let junk = b"not json".to_vec();
    assert!(verify_manifest(&junk, &sign_manifest(&SEED, &junk), &pk()).is_err());
}

#[test]
fn verify_manifest_rejects_missing_or_garbage_signature() {
    let json = manifest_json(&sample());
    for sig in ["", "!!notbase64", "AAAA", "<html>Not Found</html>"] {
        assert!(matches!(verify_manifest(&json, sig, &pk()),
            Err(ServiceError::Update(s)) if s == "signature"), "{sig:?}");
    }
}

#[test]
fn verify_manifest_rejects_insecure_addresses() {
    for m in [
        Manifest { url: "http://x/i.exe".into(), ..sample() },
        Manifest { page: "file:///C:/x".into(), ..sample() },
        Manifest { sha256: "XYZ".into(), ..sample() },
    ] {
        let json = manifest_json(&m);
        assert!(verify_manifest(&json, &sign_manifest(&SEED, &json), &pk()).is_err());
    }
}

#[test]
fn download_rejects_insecure_url_without_creating_the_file() {
    let m = Manifest { url: "http://x/i.exe".into(), ..sample() };
    let dest = tempfile::tempdir().unwrap().path().join("u.exe");
    assert!(matches!(download_and_verify(&m, &dest),
        Err(ServiceError::Update(s)) if s == "insecure url"));
    assert!(!dest.exists());
}

#[test]
fn checksum_must_match() {
    assert!(check_sha256(b"installer", &sample().sha256).is_ok());
    assert!(matches!(check_sha256(b"other", &sample().sha256),
        Err(ServiceError::Update(s)) if s == "checksum"));
}

#[test]
fn installed_copy_has_the_uninstaller_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("tt-app.exe");
    assert!(!is_installed_copy(&exe));
    std::fs::write(dir.path().join("uninstall.exe"), b"").unwrap();
    assert!(is_installed_copy(&exe));
}
```

Keep `read_capped_enforces_limit`, `agents_are_https_only…`, `parse_config_cases` and the platform tests.

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p tt-services update::`
Expected: FAIL. The new functions are missing and `Manifest` has no `page`.

- [ ] **Step 3: Implement**
  - `verify_manifest`:
    1. Verify the signature over `json` (`verify_strict`). Any failure is `Update("signature")`.
    2. Parse with `serde_json::from_slice`. A failure is `Update("manifest")`.
    3. Validate: `url` and `page` start with `https://`, and `sha256` is 64 hex digits. A failure is `Update("manifest")`.
  - `check_sha256(bytes, expected) -> Result<()>` is private and replaces `verify_download`.
  - `check_for_update`:
    1. GET `cfg.manifest_url`, read with `read_capped(…, 64 * 1024)`.
    2. GET `format!("{}.sig", cfg.manifest_url)`, read with `read_capped(…, 1024)`, decode as UTF-8 (a failure is `Update("signature")`).
    3. Call `verify_manifest`.
    4. Return `is_newer(current, &m.version).then_some(m)`.
  - `download_and_verify`: the same flow as today's `download`, but it calls `check_sha256` instead of `verify_download`.
  - Delete `verify_bytes` and `verify_download`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -p tt-services update::`, then `cargo build -p tt-app`
Expected: the tests PASS. The `tt-app` build fails only at the `download_and_verify(&cfg, …)` call in `controllers/settings.rs`; change that call to `download_and_verify(&manifest, &dest)` so it builds.

- [ ] **Step 5: Commit**: `feat(update): sign the whole manifest; verify before parsing`

---

### Task 3: `tt-release` signing tool

**Files:**
- Create: `crates/tt-services/src/bin/tt-release.rs`

**Interfaces:**
- Consumes: `tag_matches_version` (Task 1); `Manifest`, `manifest_json`, `sign_manifest`, `public_key_hex` (Task 2)
- Produces: the CLI that CI calls in Task 6. Every failure prints a reason to stderr and exits with code 1.
  - `tt-release check-version <tag>`: exit 0 when `tag_matches_version(tag, env!("CARGO_PKG_VERSION"))` holds. On success it prints the version without the leading `v`.
  - `tt-release pubkey`: prints `public_key_hex(seed)`.
  - `tt-release manifest <version> <installer.exe> <url> <page> <out-dir>`: writes `<out-dir>/manifest.json` (the `manifest_json` bytes) and `<out-dir>/manifest.json.sig` (the base64 signature text). The sha256 is computed from the installer file.
  - The seed comes from env `TT_UPDATE_SIGNING_KEY`, and only for `pubkey` and `manifest`. Missing or invalid input is an error: "TT_UPDATE_SIGNING_KEY: expected 64 hex digits". The value is never echoed.

- [ ] **Step 1: Write the failing test.** Add a lib test in `update.rs` for the seed parser that the bin uses: `pub fn parse_seed(hex: &str) -> Option<[u8; 32]>`. It trims whitespace, so that a value pasted with a newline still works.

```rust
#[test]
fn seed_is_64_hex_digits() {
    assert_eq!(parse_seed(&"07".repeat(32)), Some(SEED));
    assert_eq!(parse_seed(&format!(" {}\n", "07".repeat(32))), Some(SEED));
    assert!(parse_seed("07").is_none());
    assert!(parse_seed(&"zz".repeat(32)).is_none());
}
```

- [ ] **Step 2: Run it and see it fail**: `cargo test -p tt-services seed_is` → FAIL (`parse_seed` is missing).

- [ ] **Step 3: Implement** `parse_seed`, then the bin.
  - Parse arguments by hand with `std::env::args` (no new crates).
  - `main` returns `std::process::ExitCode`.

- [ ] **Step 4: Verify**
  - Run `cargo test -p tt-services` → PASS.
  - Smoke test in PowerShell:
    ```powershell
    $env:TT_UPDATE_SIGNING_KEY = "07" * 32
    cargo run -q -p tt-services --bin tt-release -- check-version v0.3.0-rc.9   # prints 0.3.0-rc.9
    cargo run -q -p tt-services --bin tt-release -- check-version v9.9.9        # exit 1
    cargo run -q -p tt-services --bin tt-release -- pubkey                      # 64 hex
    cargo run -q -p tt-services --bin tt-release -- manifest 0.3.0 Cargo.toml https://x/i.exe https://x/p $env:TEMP
    ```
  - Then remove the env var.
  - Expected: `manifest.json` and `manifest.json.sig` exist in `%TEMP%`.

- [ ] **Step 5: Commit**: `feat(update): add tt-release tool for signing manifests in CI`

---

### Task 4: Updates controller (split, guard, cleanup, 24 h check, silent install)

**Files:**
- Create: `crates/tt-app/src/controllers/updates.rs`
- Modify:
  - `crates/tt-app/src/controllers/mod.rs` (`pub mod updates;`)
  - `crates/tt-app/src/controllers/settings.rs`: remove the `UPDATE_*` constants, the `update` field, `set_update_state`, `check_updates`, `install_update`, the update callbacks, the auto-check at start and the update part of the module doc. `on_set_update_auto_check` stays, because it is a setting. `set_app_version` uses `current_version()`.
  - `crates/tt-app/src/main.rs` (`let _updates = updates::install(&ui, &ctx);` after settings)

**Interfaces:**
- Consumes: `check_for_update`, `download_and_verify`, `current_version`, `update_config`, `Manifest` (Tasks 1–2)
- Produces:
  - `pub fn install(ui: &MainWindow, ctx: &Ctx) -> Rc<UpdatesController>`, which owns `on_check_updates` and `on_install_update`.
  - `pub fn busy(state: i32) -> bool`
  - `pub fn cleanup_installer(path: &Path)`
  - `pub fn installer_path() -> PathBuf`, which returns `std::env::temp_dir().join("TimeTracker-update.exe")`.
  - Constant `RECHECK: Duration = Duration::from_secs(24 * 60 * 60)`.

- [ ] **Step 1: Write the failing tests** in `updates.rs`

```rust
#[test]
fn busy_states_block_new_work() {
    assert!(busy(UPDATE_CHECKING) && busy(UPDATE_DOWNLOADING));
    for s in [UPDATE_IDLE, UPDATE_LATEST, UPDATE_AVAILABLE, UPDATE_FAILED] {
        assert!(!busy(s));
    }
}

#[test]
fn cleanup_removes_installer_and_part() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("TimeTracker-update.exe");
    std::fs::write(&exe, b"x").unwrap();
    std::fs::write(dir.path().join("TimeTracker-update.exe.part"), b"x").unwrap();
    cleanup_installer(&exe);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    cleanup_installer(&exe); // nothing there: no panic
}
```

`tempfile` is already a dev-dependency of tt-app.

- [ ] **Step 2: Run the tests and see them fail**: `cargo test -p tt-app updates::` → FAIL (module missing).

- [ ] **Step 3: Implement** `UpdatesController`, moving the existing code and changing it as follows:
  - `install`:
    1. Call `cleanup_installer(&installer_path())`.
    2. Return early (bridge left as is) when `update_config()` is `None`.
    3. Register the callbacks.
    4. Run a quiet check at start if `settings.update_auto_check` is set.
    5. Start a `slint::Timer` (`TimerMode::Repeated`, `RECHECK`) kept in the controller. On each tick it runs a quiet check if `update_auto_check` is still set at that moment.
  - `check_updates(quiet)` and `install_update()` first read `SettingsBridge.update-state` and return if `busy(..)`.
  - `check_for_update(&cfg, current_version())`.
  - The installer starts as `Command::new(&dest).args(["/S", "/UPDATE"]).spawn()`, then `slint::quit_event_loop()` as now.
  - `cleanup_installer(path)`: `remove_file` on `path` and on `path` + `.part`, ignoring errors.

- [ ] **Step 4: Verify**
  - Run `cargo test -p tt-app` → PASS.
  - Run `cargo clippy --all-targets -- -D warnings` → clean.
  - Run `wc -l crates/tt-app/src/controllers/settings.rs` → under 600.

- [ ] **Step 5: Commit**: `refactor(app): move updates into their own controller; guard, clean up, recheck daily`

---

### Task 5: UI — portable link, "What's new", installer relaunch

**Files:**
- Modify:
  - `crates/tt-app/ui/bridges/settings.slint`
  - `crates/tt-app/ui/screens/settings.slint` (Updates card)
  - `crates/tt-app/src/controllers/updates.rs`
  - `crates/tt-app/Cargo.toml` (windows-sys feature `Win32_UI_Shell`)
  - `crates/tt-app/src/platform/` (new `open_url` in a fitting module, Windows-only)
  - `crates/tt-app/installer/timetracker.nsi`
  - `translations/tt-app.pot` and the three `.po` files

**Interfaces:**
- Consumes: `is_installed_copy` (Task 2); `UpdatesController` (Task 4)
- Produces:
  - Bridge properties: `in property <bool> update-installable;` and `in property <string> update-page;`.
  - Bridge callback: `callback open-update-page();`.
  - `pub fn open_url(url: &str) -> std::io::Result<()>`: `ShellExecuteW` "open" on Windows; on other platforms it returns `Err(Unsupported)`. It is never called there, because updates are off outside Windows.

- [ ] **Step 1: Write the failing test.** Add the new English strings to the Updates card:
  - the Button "Open download page", shown when state is 3 and `!update-installable`;
  - a link-style text "What's new", shown when state is 3, which calls `open-update-page`.
  - "Download and install" is shown only when `update-installable` is set.
  - Then run `cargo test -p tt-app --test translations`.
  - Expected: FAIL. The strings are missing from the `.pot` and `.po` files.

- [ ] **Step 2: Add the translations** (ru «Открыть страницу загрузки» / «Что нового», zh_CN «打开下载页面» / «更新内容», es «Abrir la página de descarga» / «Novedades»). Re-run the test → PASS.

- [ ] **Step 3: Wire the controller.**
  - Set `update-installable` from `is_installed_copy(&std::env::current_exe()?)`. If `current_exe` fails, treat the copy as not installed.
  - When a check finds a manifest, set `update-page` to `m.page`.
  - `open-update-page` calls `open_url` with the stored manifest's `page`. The page passed `verify_manifest`, so it is https. On an error, show the toast `err_generic`.

- [ ] **Step 4: Change the NSIS installer.**
  - Add `!include "FileFunc.nsh"`.
  - At the end of `Section "Install"`, relaunch the app when the `/UPDATE` option was passed:
    ```nsis
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "/UPDATE" $R1
    ${IfNot} ${Errors}
      Exec '"$INSTDIR\${EXE}"'
    ${EndIf}
    ```
  - Update the header comment of the `.nsi` to describe `/UPDATE`.

- [ ] **Step 5: Verify**
  - Run `cargo test --workspace` → PASS.
  - If `makensis` is available locally: `cargo build --release -p tt-app` then `makensis /V2 /DVERSION=0.0.1 crates\tt-app\installer\timetracker.nsi` → it builds without warnings-as-errors. Otherwise the CI tag build covers it; note this in the report.

- [ ] **Step 6: Commit**: `feat(app): portable download link, release notes link, relaunch after silent update`

---

### Task 6: CI — version check, signed manifest, draft release

**Files:**
- Modify: `.github/workflows/ci.yml` (jobs `installer` and `prerelease` → `release`)
- Modify: `docs/development.md` (sections "Installer" → "Publishing", and "Updates")

**Interfaces:**
- Consumes: the `tt-release` CLI (Task 3)

- [ ] **Step 1: Job `installer`**, before `cargo build`, add the step "Version":
  ```pwsh
  $v = cargo run -q -p tt-services --bin tt-release -- check-version "${{ github.ref_name }}"
  if ($LASTEXITCODE -ne 0) { exit 1 }
  "TT_BUILD_VERSION=$v" >> $env:GITHUB_ENV
  ```

- [ ] **Step 2: Job `installer`**, after the "Installer" step, add the step "Update manifest":
  - It runs with `if: vars.TT_UPDATE_PUBKEY != ''` and `env: TT_UPDATE_SIGNING_KEY: ${{ secrets.TT_UPDATE_SIGNING_KEY }}`. The secret is set on this step only.
  - Run `tt-release pubkey`. If it fails or its output is not equal to `$env:TT_UPDATE_PUBKEY`, exit 1 with the message "TT_UPDATE_SIGNING_KEY does not match TT_UPDATE_PUBKEY".
  - Run `tt-release manifest $env:TT_BUILD_VERSION target/installer/TimeTracker-$env:TT_BUILD_VERSION-setup.exe https://github.com/$env:GITHUB_REPOSITORY/releases/download/$env:GITHUB_REF_NAME/TimeTracker-$env:TT_BUILD_VERSION-setup.exe https://github.com/$env:GITHUB_REPOSITORY/releases/tag/$env:GITHUB_REF_NAME target/installer`.
  - The existing upload of `target/installer/*` then carries both files.

- [ ] **Step 3: Rename job `prerelease` to `release`.**
  - `if: startsWith(github.ref, 'refs/tags/v')`.
  - Tag contains `-` → `gh release create … --prerelease`, with the current notes.
  - Otherwise → `gh release create … --draft`, with notes for a final release: an install link plus the macOS first-start lines (builder writes the text, spec §8).
  - Update the comment above the job: "Release candidates are published as prereleases; final releases are created as drafts and published by hand."

- [ ] **Step 4: Validate the workflow syntax.** If `actionlint` is available, run `actionlint .github/workflows/ci.yml`. Otherwise run `python -c "import yaml,sys;yaml.safe_load(open('.github/workflows/ci.yml'))"`.
  - Expected: no errors.

- [ ] **Step 5: Update `docs/development.md`.**
  - **Publishing:** a tag goes to the CI draft. Check the draft, then Publish. Publishing rolls out the update. Rollback means a fixed release, or marking the bad release as pre-release.
  - **Updates:**
    - Make the key once with PowerShell 7:
      ```powershell
      $seed = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLower()
      $env:TT_UPDATE_SIGNING_KEY = $seed; cargo run -q -p tt-services --bin tt-release -- pubkey
      ```
    - Set the secret `TT_UPDATE_SIGNING_KEY` and the variables `TT_UPDATE_PUBKEY` and `TT_UPDATE_URL = https://github.com/OlegFM/timetracker/releases/latest/download/manifest.json`.
    - Keep the seed in a password manager as well. If it is lost, shipped builds cannot update.
    - Describe the manifest format and `TT_BUILD_VERSION`.

- [ ] **Step 6: Commit**: `ci: sign the update manifest and create final releases as drafts`

---

### Task 7: User docs, known issues, QA, changelog

**Files:**
- Modify:
  - `docs/known-issues.md`: drop the closed items from "Must be done before updates are enabled" and keep only "split backup/restore out of `controllers/settings.rs`". Rename the section to fit.
  - `docs/qa-checklist.md`: a new section "Updates (Windows)" with the manual scenario from spec §6:
    1. Build a local `0.0.1` with `TT_UPDATE_URL` set to a specific rc manifest.
    2. Check, then Install.
    3. Expect: the app closes, updates silently and restarts as the new version; the timer is kept.
    4. Portable: the release page opens.
  - `README.md`: an "Updates" paragraph. It covers: Windows only; a check at start and daily, which can be turned off in Settings; one request to GitHub; nothing installs without a click; builds before the first updates-enabled release need one manual update.
  - `CHANGELOG.md`: under `[Unreleased]` → `### Added`, the Windows auto-update.
  - `docs/superpowers/specs/2026-11-15-windows-auto-update-design.md`: status "реализовано".

- [ ] **Step 1: Write the docs above.**

- [ ] **Step 2: Verify** that `cargo test --workspace` passes (the translations test reads the UI files) and that the doc links work: `grep -n "known-issues" README.md docs/*.md`.

- [ ] **Step 3: Commit**: `docs: auto-update for Windows`
