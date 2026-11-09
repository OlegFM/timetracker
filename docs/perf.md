# Performance: memory, CPU, size

Targets (spec §2.6): ≤ 50 MB private working set idle with the main window in the tray
and the mini timer shown, CPU ≈ 0 % idle, cold start ≤ 1 s, installer ≤ 20 MB.

**Result (v0.1.0):** Slint software renderer only, hidden windows destroyed,
`opt-level = "s"`: **4.4–6.1 MB private working set** with the timer running in the tray
(target 50 MB), **0.14–0.18 % of one core** while the timer ticks and 0 when stopped,
200 ms cold start, 21.7 MB executable, 6.6 MB installer.

## How it is measured

`crates/tt-app/tools/measure_idle.ps1` starts a release build on a seed database
(`cargo run -p tt-app --example seed -- <db> [--running]`, opened with `--db <db>`),
waits 20 s, then samples the process every 10 s for 5 minutes:

- **Private WS** — private working set (`WorkingSetPrivate`, the "Memory" column of Task
  Manager). This is the number the target is about.
- **Private bytes** — `PrivateMemorySize64` (committed private memory, includes pages
  that are not resident).
- **Working set** — `WorkingSet64`, includes shared pages of system DLLs.
- **CPU** — process CPU time over the sampling window, in % of one core.

Scenarios: **tray** — started with `--minimized` (as autostart does): the main window is
never created, only the mini timer is shown; **closed** — the main window is shown
(dashboard), then closed with ✕ (close to tray), the mini timer stays. Timer running
unless noted. Machine: Windows 11, 32 logical cores, 64 GB RAM, 100 % display scale; a
renderer was chosen at run time with `SLINT_BACKEND=winit-femtovg|winit-software` where
both were compiled in.

## Measurements (averages over 5 min, MB)

| # | Build | Renderer | Scenario | Private WS | Private bytes | Working set | CPU % of 1 core |
|---|---|---|---|---|---|---|---|
| 1 | O3, Slint default features | femtovg (OpenGL) | tray | 38.4 | 99.1 | 79.4 | 0.38 |
| 2 | O3, Slint default features | femtovg (OpenGL) | closed | 59.4 | 128.7 | 101.6 | 1.54 |
| 3 | O3, Slint default features | femtovg (OpenGL) | closed, stopped | 48.6 | 114.8 | 90.7 | 0 |
| 4 | O3, Slint default features | software | tray | 4.9 | 5.8 | 31.9 | 0.16 |
| 5 | O3, Slint default features | software | closed | 10.1 | 11.3 | 38.3 | 0.63 |
| 6 | O3, Slint default features | software | closed, stopped | 9.9 | 10.9 | 37.9 | 0 |
| 7 | O3, software only | software | tray | 4.4 | 5.2 | 42.9 | 0.13 |
| 8 | O3, software only | software | closed | 9.7 | 10.9 | 37.4 | 0.69 |
| 9 | Os, software only | software | tray | 4.6 | 5.5 | 41.7 | 0.18 |
| 10 | Os, software only | software | closed | 9.6 | 10.8 | 36.3 | 0.93 |
| 11 | **Os, software only, hidden windows destroyed (shipped)** | software | tray | **4.4** | 5.3 | 30.3 | 0.14 |
| 12 | **same** | software | closed | **6.1** | 7.3 | 33.0 | 0.18 |
| 13 | **same** | software | closed, stopped | **5.3** | 6.2 | 32.1 | 0.01 |

A first 5-minute run of rows 11–13 gave 10.2 MB / 3.8 % CPU (closed) and 8.4 MB / 1.1 %
(stopped, memory peaking at 10.8 MB): the numbers of a visible main window. It was
disturbed — the window was most likely opened from the tray during the run; a direct
check of the same build right after (main window gone after ✕, 6.8 MB private bytes, no
CPU) and the repeated run (rows 11–13) agree. Since then `measure_idle.ps1` reports
whether there was keyboard or mouse input while it sampled.

Short checks (30 s) with femtovg and `SLINT_DESTROY_WINDOW_ON_HIDE=1`: closed
53.6 MB private WS / 102.8 MB private bytes — destroying the main window does not bring
the OpenGL renderer under the target, because the GL driver alone costs ~35 MB per
process (row 1: the main window was never created).

## Decisions

1. **Renderer: software.** The femtovg (OpenGL) renderer loads the GPU driver into the
   process: 38 MB private WS with only the 240×88 mini timer, 59 MB after the main window
   was shown once. The software renderer needs 5–10 MB in the same scenarios. Text is
   rendered the same way (grayscale anti-aliasing, same glyph outlines; compared on
   enlarged screenshots in en/ru/zh_CN, CJK fallback works). Slint picks the renderer per
   backend, not per window, so "software only for the mini timer" is not possible; it is
   used for all windows.
   - Trade-offs of the software renderer (Slint 1.18): no drop shadows (popups, dialogs
     and toasts keep their 1 px border, so they stay distinct), no rounded clipping, and
     a gradient `Rectangle` ignores `border-radius` — the sidebar logo is therefore an
     image (`assets/logo.png`, from `tools/gen_icon.py`). Every frame is drawn by the
     CPU; for this UI (no animations, 1 s tick) that is not measurable idle.
   - Skia was not tried: its build downloads prebuilt Skia binaries (not allowed here)
     and it adds ~10 MB to the executable.
2. **Slint features trimmed** to `std, compat-1-2, accessibility, backend-winit,
   renderer-software, software-renderer-path, unstable-winit-030` (no femtovg/OpenGL
   stack, no Slint system tray — the app uses `tray-icon`). Memory is the same as with
   the software renderer chosen at run time (rows 4–5 vs 7–8); the executable shrinks
   (O3: 30.1 → 29.4 MB) and the OpenGL code is gone from the process.
3. **Hidden windows are destroyed** (`SLINT_DESTROY_WINDOW_ON_HIDE`, set first thing in
   `main`). A window hidden with `hide()` keeps its native window and frame buffer and —
   found by these measurements — is still repainted on every 1 s timer tick (rows 5, 8,
   10: 4–5× the CPU of the tray scenario). Spec §9.1 says a hidden window is not
   rendered, so the main window (and the mini timer, when hidden) is now destroyed on
   hide and created again on `show()` (Slint keeps its size and position).
   Window attributes that Slint caches are **lost on re-create**: Slint applies the
   window level and the icon only when they differ from its cached value, which is not
   reset when the window is destroyed. So they are set in `when_created` after every
   `show()` — `window_ext::keep_on_top` for the mini timer, `window_ext::set_app_icon`
   for the main window (not Slint's `always-on-top` / `icon` alone).
4. **`opt-level = "s"`** (with `lto = "thin"`, `codegen-units = 1`, `strip = true`):
   21.7 MB executable instead of 29.4 MB with `opt-level = 3`, same memory (rows 7–10).
   The extra CPU of Os in row 10 came from repainting the hidden window, which decision 3
   removed.

## Size

| Artifact | Size |
|---|---|
| `tt-app.exe`, O3, Slint default features | 30.1 MB |
| `tt-app.exe`, O3, software only | 29.4 MB |
| `tt-app.exe`, Os, software only (shipped) | 21.7 MB |
| NSIS installer (LZMA, solid) | 6.6 MB |
| Portable zip (`tt-app.exe` only) | 9.0 MB |

Cold start (launch to visible main window, seed database, warm file cache): 200 ms.
Showing the main window again after it was destroyed in the tray: ~40 ms.

## Re-measuring

```powershell
cargo build --release -p tt-app
cargo run -p tt-app --example seed -- $env:TEMP\tt-running.db --running
pwsh crates/tt-app/tools/measure_idle.ps1 -Exe target\release\tt-app.exe `
    -Database $env:TEMP\tt-running.db -Scenario closed
```

Close a running TimeTracker first (single instance). `--db` leaves the autostart entry
alone.
