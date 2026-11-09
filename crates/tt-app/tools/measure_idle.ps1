# Idle memory/CPU sampler for tt-app (docs/perf.md).
#
#   pwsh crates/tt-app/tools/measure_idle.ps1 -Exe target/release/tt-app.exe -Database <seed.db> `
#       [-Scenario tray|closed] [-Renderer software|femtovg] [-Seconds 300]
#
# Close any running TimeTracker first (single-instance). Scenarios:
#   tray   - started with `--minimized` (autostart): the main window is never created,
#            only the mini timer is shown;
#   closed - the main window is shown, then closed with its X button (close to tray),
#            the mini timer stays.
# After -Warmup seconds the process is sampled every -Interval seconds for -Seconds
# seconds. "Private WS" is the private working set (the "Memory" column of Task Manager);
# "Private bytes" is PrivateMemorySize64 (committed private memory); "Working set"
# includes shared pages (DLLs). Do not touch the mouse or keyboard while it runs: such
# input (hovering the mini timer, opening the window from the tray) changes the numbers,
# and the result says whether there was any.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Database,
    [ValidateSet("tray", "closed")] [string] $Scenario = "tray",
    [string] $Renderer = "",
    [int] $Seconds = 300,
    [int] $Interval = 10,
    [int] $Warmup = 20
)
$ErrorActionPreference = "Stop"

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class TtWin {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LASTINPUTINFO i);
    struct RECT { public int L, T, R, B; }
    struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }
    // Tick count of the last keyboard or mouse input in this session.
    public static uint LastInput() {
        var i = new LASTINPUTINFO { cbSize = 8 };
        GetLastInputInfo(ref i);
        return i.dwTime;
    }
    // Sends WM_CLOSE to the largest visible window of the process (the main window).
    public static bool CloseLargest(uint process) {
        IntPtr best = IntPtr.Zero; long area = 0;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            RECT r;
            if (pid == process && IsWindowVisible(h) && GetWindowRect(h, out r)) {
                long a = (long)(r.R - r.L) * (r.B - r.T);
                if (a > area) { area = a; best = h; }
            }
            return true;
        }, IntPtr.Zero);
        return best != IntPtr.Zero && PostMessage(best, 0x0010, IntPtr.Zero, IntPtr.Zero);
    }
}
"@

if (Get-Process tt-app -ErrorAction SilentlyContinue) {
    throw "tt-app is already running; close it first (single-instance mutex)."
}
$appArgs = @("--db", "`"$Database`"")
if ($Scenario -eq "tray") { $appArgs = @("--minimized") + $appArgs }
$env:SLINT_BACKEND = if ($Renderer) { "winit-$Renderer" } else { $null }
$p = Start-Process -FilePath $Exe -ArgumentList $appArgs -PassThru
$env:SLINT_BACKEND = $null
try {
    if ($Scenario -eq "closed") {
        Start-Sleep -Seconds 5
        if (-not [TtWin]::CloseLargest([uint32]$p.Id)) { throw "main window not found" }
    }
    Start-Sleep -Seconds $Warmup
    $p.Refresh()
    $cpu0 = $p.TotalProcessorTime
    $input0 = [TtWin]::LastInput()
    $t0 = Get-Date
    $samples = @()
    $deadline = $t0.AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        $p.Refresh()
        $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess = $($p.Id)"
        $samples += [pscustomobject]@{
            PrivateWS    = $perf.WorkingSetPrivate / 1MB
            WorkingSet   = $p.WorkingSet64 / 1MB
            PrivateBytes = $p.PrivateMemorySize64 / 1MB
        }
        Start-Sleep -Seconds $Interval
    }
    $p.Refresh()
    $elapsed = ((Get-Date) - $t0).TotalSeconds
    $cpu = ($p.TotalProcessorTime - $cpu0).TotalSeconds
    $avg = { param($n) [math]::Round(($samples | Measure-Object $n -Average).Average, 1) }
    $max = { param($n) [math]::Round(($samples | Measure-Object $n -Maximum).Maximum, 1) }
    [pscustomobject]@{
        Scenario          = "$Scenario, renderer '$Renderer', $([math]::Round($elapsed)) s"
        Samples           = $samples.Count
        "Private WS MB"   = "$(& $avg PrivateWS) (max $(& $max PrivateWS))"
        "Private bytes MB" = "$(& $avg PrivateBytes) (max $(& $max PrivateBytes))"
        "Working set MB"  = "$(& $avg WorkingSet) (max $(& $max WorkingSet))"
        "CPU % of 1 core" = [math]::Round(100 * $cpu / $elapsed, 3)
        "CPU s"           = [math]::Round($cpu, 2)
        "User input"      = if ([TtWin]::LastInput() -ne $input0) { "YES - discard" } else { "none" }
    } | Format-List
}
finally {
    Stop-Process -Id $p.Id -ErrorAction SilentlyContinue
    $p.WaitForExit(10000) | Out-Null
}
