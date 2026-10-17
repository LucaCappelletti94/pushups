# Runs the W1 checks with the probe from ci/windows/w1 on a Windows machine with the Windows App SDK 2.5 runtime.
# Run from the repository root after `cargo build --release --manifest-path ci/windows/w1/Cargo.toml`,
# with WNS_TENANT_ID, WNS_APP_ID, WNS_OBJECT_ID and WNS_CLIENT_SECRET set.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'wns.ps1')

$work = Join-Path ($env:RUNNER_TEMP ?? [IO.Path]::GetTempPath()) 'w1'
$app = Join-Path $work 'app'
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $app | Out-Null
$exe = Join-Path $app 'pushups-w1-probe.exe'
$verdict = [Collections.Generic.List[string]]::new()
$passed = $true
# The bound for a toast's tap line after its click.
$ToastBound = [TimeSpan]::FromSeconds(60)
# The bound for a banner to settle after the show answer. Windows shows queued banners one at a
# time, so the clicks come before the bound pass queues a dozen more.
$BannerBound = [TimeSpan]::FromSeconds(8)
$screens = Join-Path $work 'screens'
New-Item -ItemType Directory -Force $screens | Out-Null

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win32Input {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, int dwExtraInfo);
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        mouse_event(0x0002, 0, 0, 0, 0);
        mouse_event(0x0004, 0, 0, 0, 0);
    }

    // The rectangle in the right half of the screen, below its top third, where `after` differs
    // from `before` in a block at least 150 by 50 pixels, as a toast banner does, or null. Columns
    // and rows with only a few changed pixels, a cursor or a clock, do not count.
    public static int[] Banner(int[] before, int[] after, int width, int height) {
        int left = width / 2, top = height / 3;
        var columns = new int[width];
        var rows = new int[height];
        for (int y = top; y < height; y++) {
            for (int x = left; x < width; x++) {
                int a = before[y * width + x], b = after[y * width + x];
                int change = Math.Abs((a & 0xff) - (b & 0xff)) + Math.Abs(((a >> 8) & 0xff) - ((b >> 8) & 0xff))
                    + Math.Abs(((a >> 16) & 0xff) - ((b >> 16) & 0xff));
                if (change > 60) { columns[x]++; rows[y]++; }
            }
        }
        int[] across = LongestRun(columns, left, width, 40), down = LongestRun(rows, top, height, 100);
        if (across[1] - across[0] < 150 || down[1] - down[0] < 50) return null;
        return new[] { across[0], down[0], across[1] - across[0], down[1] - down[0] };
    }

    static int[] LongestRun(int[] counts, int from, int to, int least) {
        int start = from, bestStart = from, bestEnd = from;
        for (int i = from; i <= to; i++) {
            if (i < to && counts[i] >= least) continue;
            if (i - start > bestEnd - bestStart) { bestStart = start; bestEnd = i; }
            start = i + 1;
        }
        return new[] { bestStart, bestEnd };
    }
}
'@
# Physical pixels for the captures and the clicks on a scaled display.
$null = [Win32Input]::SetProcessDPIAware()

function Capture-Screen {
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = [System.Drawing.Bitmap]::new($bounds.Width, $bounds.Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $graphics.Dispose()
    $bitmap
}

function Get-Pixels([System.Drawing.Bitmap] $bitmap) {
    $rect = [System.Drawing.Rectangle]::new(0, 0, $bitmap.Width, $bitmap.Height)
    $data = $bitmap.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $pixels = [int[]]::new($bitmap.Width * $bitmap.Height)
    [Runtime.InteropServices.Marshal]::Copy($data.Scan0, $pixels, 0, $pixels.Length)
    $bitmap.UnlockBits($data)
    , $pixels
}

# The screen before a toast, after `$settle` lets any banner still up time out.
function Capture-Before([bool] $settle) {
    if ($settle) { Start-Sleep -Seconds 8 }
    $bitmap = Capture-Screen
    $pixels = Get-Pixels $bitmap
    $bitmap.Dispose()
    , $pixels
}

# Clicks the banner a toast put on the screen since `$before` and waits for the probe's tap line
# in $logDir. UI Automation does not reach toast banners, so the banner is found by what changed
# on the screen, once two captures 250 ms apart place it alike, after it has slid in. Answers
# `unreached` when no banner settles, which the GitHub runner's session may not show, `failed`
# when a click brought no tap line, and `passed` otherwise.
function Click-Banner([string] $label, [int[]] $before, [string] $logDir, [string] $expected) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $banner = $null
    $last = $null
    while ($true) {
        $after = Capture-Screen
        $found = [Win32Input]::Banner($before, (Get-Pixels $after), $after.Width, $after.Height)
        if ($found -and $last -and -not (Compare-Object $found $last)) { $banner = $found }
        if ($banner -or $clock.Elapsed -ge $BannerBound) { break }
        $last = $found
        $after.Dispose()
        Start-Sleep -Milliseconds 250
    }
    if (-not $banner) {
        $after.Save((Join-Path $screens "$label.png"), [System.Drawing.Imaging.ImageFormat]::Png)
        $after.Dispose()
        Note "- ${label}: no banner settled on the screen within $BannerBound, not reachable in this session"
        return 'unreached'
    }
    $after.Dispose()
    $x, $y, $width, $height = $banner
    [Win32Input]::Click($x + [int] ($width / 2), $y + [int] ($height * 0.6))
    Note "- ${label}: the banner at $x,$y ${width}x$height was clicked"
    $got = Wait-Until $ToastBound { First-Line (Join-Path $logDir 'events.log') $expected }
    if (-not $got) {
        $shot = Capture-Screen
        $shot.Save((Join-Path $screens "$label.png"), [System.Drawing.Imaging.ImageFormat]::Png)
        $shot.Dispose()
        Note "- ${label}: no tap line within $ToastBound of the click"
        return 'failed'
    }
    Note "- ${label}: $got"
    return 'passed'
}

function Start-Probe([string] $label, [string] $title, [string] $data) {
    $out = Join-Path $work "out-$label"
    New-Item -ItemType Directory -Force $out | Out-Null
    $env:PROBE_OUT = $out
    $env:PROBE_REMOTE_ID = $env:WNS_OBJECT_ID
    $env:PROBE_TITLE = $title
    $env:PROBE_DATA = $data
    $process = Start-Process -FilePath $exe -WorkingDirectory $app -NoNewWindow -PassThru
    Remove-Item Env:PROBE_OUT, Env:PROBE_REMOTE_ID, Env:PROBE_TITLE, Env:PROBE_DATA
    [pscustomobject]@{ Label = $label; Process = $process; Out = $out }
}

function Stop-Probe($probe) {
    New-Item -ItemType File -Force (Join-Path $probe.Out 'stop') | Out-Null
    if (-not $probe.Process.WaitForExit([int] $ExitBound.TotalMilliseconds)) {
        $probe.Process.Kill()
        Note "- the $($probe.Label) probe did not stop within $ExitBound and was killed"
    }
    Write-Host "--- $($probe.Label) events.log"
    Get-Content (Join-Path $probe.Out 'events.log') -ErrorAction SilentlyContinue | Write-Host
}

# Waits for the probe's show answer, reports it, and returns the line, or nothing within the bound.
function Wait-Shown($probe) {
    $line = Wait-Until $ToastBound {
        First-Line (Join-Path $probe.Out 'events.log') '^(shown|show failed|install failed|no PROBE_REMOTE_ID)'
    }
    if (-not $line) {
        Note "- $($probe.Label): no show answer within $ToastBound"
        return $null
    }
    Note "- $($probe.Label): $line"
    return $line
}

function Random-Data([int] $bytes) {
    -join ((1..$bytes) | ForEach-Object { [char] (97 + (Get-Random -Maximum 26)) })
}

Note "## W1 on $((Get-CimInstance Win32_OperatingSystem).Caption) $([Environment]::OSVersion.Version)"
# The Appx module is Windows PowerShell's.
$runtimes = powershell.exe -NoProfile -Command "Get-AppxPackage -Name 'Microsoft.WindowsAppRuntime*' | ForEach-Object { `$_.Name + ' ' + `$_.Version }"
Note "- runtime packages: $($runtimes -join ', ')"

$package = Join-Path $work 'foundation.zip'
Invoke-WebRequest -TimeoutSec 600 -OutFile $package `
    "https://api.nuget.org/v3-flatcontainer/microsoft.windowsappsdk.foundation/$Foundation/microsoft.windowsappsdk.foundation.$Foundation.nupkg"
Expand-Archive $package (Join-Path $work 'foundation')
Copy-Item (Join-Path $work 'foundation/runtimes/win-x64/native/Microsoft.WindowsAppRuntime.Bootstrap.dll') $app
Copy-Item 'ci/windows/w1/target/release/pushups-w1-probe.exe' $exe
Copy-Item 'assets/brand/png/icon-macos-512.png' (Join-Path $app 'icon.png')

# The files a process Windows starts for a tap reads, with no environment from this script.
Set-Content (Join-Path $app 'remote_id.txt') $env:WNS_OBJECT_ID
$beside = Join-Path $app 'w1-out'

# Pass 1, the small data: a running tap, then a tap on a toast whose app has stopped.
$small = "w1 small $(Get-Random)"
$clicks = @()
$before = Capture-Before $false
$probe = Start-Probe 'small' "pushups w1 small" $small
if ((Wait-Shown $probe) -notlike 'shown *') { $passed = $false } else {
    $clicks += Click-Banner 'small' $before $probe.Out ('^message started_app=false data_len=' + $small.Length + ' ' + [regex]::Escape($small) + '$')
}
Stop-Probe $probe

# The probe stops while its banner is still up, so the click makes Windows start the app for the tap.
$closed = "w1 closed $(Get-Random)"
$before = Capture-Before ($clicks -notcontains 'passed')
$probe = Start-Probe 'small-closed' "pushups w1 closed" $closed
$shown = Wait-Shown $probe
Stop-Probe $probe
Remove-Item $beside -Recurse -Force -ErrorAction SilentlyContinue
if ($shown -notlike 'shown *') { $passed = $false } else {
    $clicks += Click-Banner 'small-closed' $before $beside ('^message started_app=true data_len=' + $closed.Length + ' ' + [regex]::Escape($closed) + '$')
}
$started = Get-Process -Name 'pushups-w1-probe' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($started) {
    $startedProbe = [pscustomobject]@{ Label = 'tap-started'; Process = $started; Out = $beside }
    Stop-Probe $startedProbe
} else {
    Write-Host '--- tap-started events.log'
    Get-Content (Join-Path $beside 'events.log') -ErrorAction SilentlyContinue | Write-Host
}

# Pass 2, 1 KiB of data, shown and clicked, once the banner of the toast the tap-started probe showed has gone.
$before = Capture-Before $true
$probe = Start-Probe '1KiB' 'pushups w1 1KiB' (Random-Data 1024)
if ((Wait-Shown $probe) -notlike 'shown *') { $passed = $false } else {
    $clicks += Click-Banner '1KiB' $before $probe.Out '^message started_app=false data_len=1024 '
}
Stop-Probe $probe
if ($clicks -contains 'failed') { $passed = $false }
if ($clicks -contains 'unreached') {
    Note '- the taps wait for a desktop session, since no banner appeared on this one''s screen'
}

# The largest data a toast takes, in 256-byte steps, ending at the first the App SDK refuses as too large.
$largest = 1024
foreach ($bytes in 5..16 | ForEach-Object { $_ * 256 }) {
    $label = "bound-$bytes"
    $probe = Start-Probe $label "pushups w1 $label" (Random-Data $bytes)
    $line = Wait-Shown $probe
    Stop-Probe $probe
    if ($line -like 'shown *') {
        $largest = $bytes
    } elseif ($line -like '*Maximum payload size exceeded*') {
        break
    } else {
        $passed = $false
        break
    }
}
Note "- the largest data shown: $largest bytes, in 256-byte steps"

Write-Step-Summary
if (-not $passed) { exit 1 }
