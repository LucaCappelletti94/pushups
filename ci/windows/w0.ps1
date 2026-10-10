# Runs the W0 checks with the probe from ci/windows/w0 on a Windows machine with the Windows App SDK 2.5 runtime.
# Run from the repository root after `cargo build --release --manifest-path ci/windows/w0/Cargo.toml`,
# with WNS_TENANT_ID, WNS_APP_ID, WNS_OBJECT_ID and WNS_CLIENT_SECRET set.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# The Foundation package metapackage 2.5.1 depends on, the release ci/windows/bindings.sh pins.
$Foundation = '2.3.12'
# WNS may take minutes to answer a channel request.
$ChannelBound = [TimeSpan]::FromSeconds(240)
$PushBound = [TimeSpan]::FromSeconds(90)
$ExitBound = [TimeSpan]::FromSeconds(20)

foreach ($name in 'WNS_TENANT_ID', 'WNS_APP_ID', 'WNS_OBJECT_ID', 'WNS_CLIENT_SECRET') {
    if (-not [Environment]::GetEnvironmentVariable($name)) { throw "$name is not set" }
}

$work = Join-Path ($env:RUNNER_TEMP ?? [IO.Path]::GetTempPath()) 'w0'
$app = Join-Path $work 'app'
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $app | Out-Null
$exe = Join-Path $app 'pushups-w0-probe.exe'
$verdict = [Collections.Generic.List[string]]::new()
$passed = $true

function Note([string] $line) {
    Write-Host $line
    $verdict.Add($line)
}

# Runs `$done` until it returns something or `$bound` passes on a monotonic clock.
function Wait-Until([TimeSpan] $bound, [scriptblock] $done) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.Elapsed -lt $bound) {
        $result = & $done
        if ($result) { return $result }
        Start-Sleep -Milliseconds 250
    }
    return $null
}

function First-Line([string] $file, [string] $pattern) {
    Get-Content $file -ErrorAction SilentlyContinue | Where-Object { $_ -match $pattern } | Select-Object -First 1
}

function Start-Probe([string] $label, [string] $remoteId) {
    $out = Join-Path $work "out-$label"
    New-Item -ItemType Directory -Force $out | Out-Null
    $env:PROBE_OUT = $out
    $env:PROBE_REMOTE_ID = $remoteId
    $process = Start-Process -FilePath $exe -WorkingDirectory $app -NoNewWindow -PassThru
    Remove-Item Env:PROBE_OUT, Env:PROBE_REMOTE_ID
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
    Write-Host "--- $($probe.Label) handled.log"
    Get-Content (Join-Path $probe.Out 'handled.log') -ErrorAction SilentlyContinue | Write-Host
}

# Waits for the probe's channel and returns its URI, noting the outcome either way.
function Wait-Channel($probe) {
    $line = Wait-Until $ChannelBound {
        First-Line (Join-Path $probe.Out 'events.log') '^(token|failed|install failed|register failed|no PROBE_REMOTE_ID)'
    }
    if (-not $line) {
        Note "- $($probe.Label): no channel answer within $ChannelBound"
        return $null
    }
    if ($line -notlike 'token *') {
        Note "- $($probe.Label): $line"
        return $null
    }
    $uri = (Get-Content -Raw (Join-Path $probe.Out 'channel.txt')).Trim()
    Write-Host "::add-mask::$uri"
    Note "- $($probe.Label): channel created on $(([Uri] $uri).Host), $($line -replace '^token \S+ ', '')"
    $uri
}

function Send-Push([string] $uri, [string] $token, [string] $payload) {
    $response = Invoke-WebRequest -Method Post -Uri $uri -TimeoutSec 60 -SkipHttpErrorCheck `
        -Headers @{ Authorization = "Bearer $token"; 'X-WNS-Type' = 'wns/raw' } `
        -ContentType 'application/octet-stream' -Body ([Text.Encoding]::UTF8.GetBytes($payload))
    $wns = foreach ($header in 'X-WNS-Status', 'X-WNS-NotificationStatus', 'X-WNS-Error-Description') {
        if ($response.Headers.ContainsKey($header)) { "$header=$($response.Headers[$header] -join ',')" }
    }
    "HTTP $($response.StatusCode) $($wns -join ' ')"
}

Note "## W0 on $((Get-CimInstance Win32_OperatingSystem).Caption) $([Environment]::OSVersion.Version)"
# The Appx module is Windows PowerShell's.
$runtimes = powershell.exe -NoProfile -Command "Get-AppxPackage -Name 'Microsoft.WindowsAppRuntime*' | ForEach-Object { `$_.Name + ' ' + `$_.Version }"
Note "- runtime packages: $($runtimes -join ', ')"

$package = Join-Path $work 'foundation.zip'
Invoke-WebRequest -TimeoutSec 600 -OutFile $package `
    "https://api.nuget.org/v3-flatcontainer/microsoft.windowsappsdk.foundation/$Foundation/microsoft.windowsappsdk.foundation.$Foundation.nupkg"
Expand-Archive $package (Join-Path $work 'foundation')
Copy-Item (Join-Path $work 'foundation/runtimes/win-x64/native/Microsoft.WindowsAppRuntime.Bootstrap.dll') $app
Copy-Item 'ci/windows/w0/target/release/pushups-w0-probe.exe' $exe

# Which GUIDs `CreateChannelAsync` accepts. Delivery then tells whether each channel reaches the app.
$ids = [ordered]@{ 'object-id' = $env:WNS_OBJECT_ID; 'app-id' = $env:WNS_APP_ID }
$channels = [Collections.Generic.List[string]]::new()
foreach ($label in $ids.Keys) {
    $probe = Start-Probe $label $ids[$label]
    $uri = Wait-Channel $probe
    Stop-Probe $probe
    if ($uri) { $channels.Add($label) }
}

$token = $null
if ($channels.Count -eq 0) {
    Note '- no channel, so no push was sent'
    $passed = $false
} else {
    try {
        $token = (Invoke-RestMethod -Method Post -TimeoutSec 60 `
                -Uri "https://login.microsoftonline.com/$env:WNS_TENANT_ID/oauth2/v2.0/token" `
                -Body @{
                grant_type = 'client_credentials'
                client_id = $env:WNS_APP_ID
                client_secret = $env:WNS_CLIENT_SECRET
                scope = 'https://wns.windows.com/.default'
            }).access_token
        Write-Host "::add-mask::$token"
        Note '- access token issued for https://wns.windows.com/.default'
    } catch {
        $reason = if ($_.ErrorDetails) { $_.ErrorDetails.Message } else { $_.Exception.Message }
        Note "- access token refused: $reason"
        $passed = $false
    }
}

$delivered = $null
if ($token) {
    foreach ($label in $channels) {
        $probe = Start-Probe "running-$label" $ids[$label]
        $uri = Wait-Channel $probe
        if ($uri) {
            Note "- push to the running app on the $label channel: $(Send-Push $uri $token "w0 running $label")"
            $got = Wait-Until $PushBound { First-Line (Join-Path $probe.Out 'events.log') "^message started_app=false w0 running $label$" }
            if ($got) {
                Note "- the running app got the $label push as ``Event::Message``"
                if (-not $delivered) { $delivered = $label }
            } else { Note "- the running app got nothing on the $label channel within $PushBound" }
        }
        Stop-Probe $probe
    }
    if (-not $delivered) { $passed = $false }
}

if ($delivered) {
    # The files a process Windows starts for a push reads, with no environment from this script.
    Set-Content (Join-Path $app 'remote_id.txt') $ids[$delivered]
    # A fresh registration with the GUID that delivered, so the closed push targets the exe this one registered.
    $probe = Start-Probe "closed-$delivered" $ids[$delivered]
    $uri = Wait-Channel $probe
    Stop-Probe $probe
    if ($uri) {
        $beside = Join-Path $app 'w0-out'
        Note "- push with the app closed: $(Send-Push $uri $token 'w0 closed')"
        $got = Wait-Until $PushBound { First-Line (Join-Path $beside 'handled.log') '^message started_app=true w0 closed$' }
        if ($got) { Note '- Windows started the app for the push and its background handler ran' } else {
            Note "- the closed app was not started within $PushBound"
            $passed = $false
        }
        New-Item -ItemType Directory -Force $beside | Out-Null
        $process = Get-Process -Name 'pushups-w0-probe' -ErrorAction SilentlyContinue | Select-Object -First 1
        $started = [pscustomobject]@{ Label = 'push-started'; Out = $beside; Process = $process }
        if ($started.Process) { Stop-Probe $started } else {
            Write-Host '--- push-started handled.log'
            Get-Content (Join-Path $beside 'handled.log') -ErrorAction SilentlyContinue | Write-Host
        }
    } else { $passed = $false }
}

if ($env:GITHUB_STEP_SUMMARY) { $verdict | Add-Content $env:GITHUB_STEP_SUMMARY }
if (-not $passed) { exit 1 }
