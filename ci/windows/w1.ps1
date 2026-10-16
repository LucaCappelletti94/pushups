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
# The bound for opening the notification centre, finding the toast and its tap line.
$ToastBound = [TimeSpan]::FromSeconds(60)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win32Input {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, int dwExtraInfo);
    [DllImport("user32.dll")] public static extern bool keybd_event(uint bVk, uint wScan, uint dwFlags, int dwExtraInfo);
    public static void Click(double x, double y) {
        SetCursorPos((int)x, (int)y);
        mouse_event(0x0002, 0, 0, 0, 0);
        mouse_event(0x0004, 0, 0, 0, 0);
    }
    public static void WinN() {
        keybd_event(0x5B, 0, 0, 0);
        keybd_event(0x4E, 0, 0, 0);
        keybd_event(0x4E, 0, 2, 0);
        keybd_event(0x5B, 0, 2, 0);
    }
}
'@

# Tries the taskbar's notification centre toggle, then the Win+N shortcut the taskbar toggle
# is not found on. The toast search after the call confirms the answer.
function Open-NotificationCenter {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $taskbar = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty, 'Shell_TrayWnd')))
    if ($taskbar) {
        foreach ($id in @('SystemTrayNotification', 'NotificationCenter')) {
            $toggle = $taskbar.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition(
                [System.Windows.Automation.AutomationElement]::AutomationIdProperty, $id)))
            if ($toggle) { return (Invoke-Element $toggle) }
        }
    }
    [Win32Input]::WinN()
    Start-Sleep -Milliseconds 500
    return $true
}

# Invokes the element's pattern, or clicks its screen rectangle, and reports whether either ran.
function Invoke-Element([System.Windows.Automation.AutomationElement] $element) {
    $pattern = $null
    if ($element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
        $pattern.Invoke()
        return $true
    }
    $rect = $element.Current.BoundingRectangle
    if ($rect.Width -le 0 -or $rect.Height -le 0) { return $false }
    [Win32Input]::Click($rect.X + $rect.Width / 2, $rect.Y + $rect.Height / 2)
    return $true
}

function Find-ToastElement([string] $title) {
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $toast = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $title)))
    if ($toast) { return $toast }
    $elements = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($element in $elements) {
        $name = $element.Current.Name
        if ($name -and $name.Contains($title)) { return $element }
    }
    return $null
}

# Clicks the named toast and waits for the probe's tap line in $logDir.
function Click-Toast([string] $label, [string] $title, [string] $logDir, [string] $expected) {
    $toast = Find-ToastElement $title
    if (-not $toast) {
        if (-not (Open-NotificationCenter)) {
            Note "- $label: the notification centre could not be opened on this runner"
            return $false
        }
        $toast = Wait-Until $ToastBound { Find-ToastElement $title }
    }
    if (-not $toast) {
        Note "- $label: no toast named `$title within $ToastBound, not reachable on this runner"
        return $false
    }
    if (-not (Invoke-Element $toast)) {
        Note "- $label: the toast had no clickable pattern and no screen rectangle"
        return $false
    }
    Note "- $label: the toast was clicked"
    $got = Wait-Until $ToastBound { First-Line (Join-Path $logDir 'events.log') $expected }
    if (-not $got) {
        Note "- $label: no tap line within $ToastBound of the click"
        return $false
    }
    Note "- $label: $got"
    return $true
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

# Waits for the probe's show answer and reports it.
function Wait-Shown($probe) {
    $line = Wait-Until $ToastBound {
        First-Line (Join-Path $probe.Out 'events.log') '^(shown|show failed|install failed|no PROBE_REMOTE_ID)'
    }
    if (-not $line) {
        Note "- $($probe.Label): no show answer within $ToastBound"
        return $false
    }
    if ($line -notlike 'shown *') {
        Note "- $($probe.Label): $line"
        return $false
    }
    Note "- $($probe.Label): $line"
    return $true
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

# Pass 1, the small data: a running tap, then the tap with the app stopped.
$small = "w1 small $(Get-Random)"
$probe = Start-Probe 'small' "pushups w1 small" $small
if (-not (Wait-Shown $probe)) { $passed = $false } else {
    if (-not (Click-Toast 'small' 'pushups w1 small' $probe.Out ('^message started_app=false data_len=' + $small.Length + ' ' + [regex]::Escape($small) + '$'))) {
        $passed = $false
    }
}
Stop-Probe $probe

# The notification stays in the Action Center, so the click now starts the app for the tap.
Remove-Item $beside -Recurse -Force -ErrorAction SilentlyContinue
if (-not (Click-Toast 'small-closed' 'pushups w1 small' $beside ('^message started_app=true data_len=' + $small.Length + ' ' + [regex]::Escape($small) + '$'))) {
    $passed = $false
}
$started = Get-Process -Name 'pushups-w1-probe' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($started) {
    $startedProbe = [pscustomobject]@{ Label = 'tap-started'; Process = $started; Out = $beside }
    Stop-Probe $startedProbe
} else {
    Write-Host '--- tap-started events.log'
    Get-Content (Join-Path $beside 'events.log') -ErrorAction SilentlyContinue | Write-Host
}

# Passes 2 and 3 measure the launch attribute's bound at 1 KiB and 4 KiB of data.
foreach ($bytes in @(1024, 4096)) {
    $label = "$($bytes / 1024)KiB"
    $title = "pushups w1 $label"
    $data = -join ((1..$bytes) | ForEach-Object { [char] (97 + (Get-Random -Maximum 26)) })
    $probe = Start-Probe $label $title $data
    if (-not (Wait-Shown $probe)) { $passed = $false } else {
        if (-not (Click-Toast $label $title $probe.Out ('^message started_app=false data_len=' + $bytes + ' '))) {
            $passed = $false
        }
    }
    Stop-Probe $probe
}

Write-Step-Summary
if (-not $passed) { exit 1 }
