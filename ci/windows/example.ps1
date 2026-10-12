# Runs one pushups example exe (Dioxus, Tauri or winit) with real WNS pushes, for the X1 proofs.
# The exe must have been built with PUSHUPS_WNS_REMOTE_ID set, so a process Windows starts for a
# push knows the remote id without any file to read.
# Run from the repository root after building the example, with WNS_TENANT_ID, WNS_APP_ID,
# WNS_OBJECT_ID and WNS_CLIENT_SECRET set. Takes the built exe's path.
param([Parameter(Mandatory)][string] $Exe)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# The Foundation package metapackage 2.5.1 depends on, the release ci/windows/bindings.sh pins.
$Foundation = '2.3.12'
# WNS may take minutes to answer a channel request.
$ChannelBound = [TimeSpan]::FromSeconds(240)
$PushBound = [TimeSpan]::FromSeconds(90)
# A push-started process also starts a window and a web view environment.
$ActivationBound = [TimeSpan]::FromSeconds(180)
$ExitBound = [TimeSpan]::FromSeconds(20)

foreach ($name in 'WNS_TENANT_ID', 'WNS_APP_ID', 'WNS_OBJECT_ID', 'WNS_CLIENT_SECRET') {
    if (-not [Environment]::GetEnvironmentVariable($name)) { throw "$name is not set" }
}

$exe = (Resolve-Path $Exe).Path
$app = [IO.Path]::GetFileNameWithoutExtension($exe)
$dir = Split-Path $exe
$log = Join-Path $env:TEMP 'pushups-example.log'
$work = Join-Path ($env:RUNNER_TEMP ?? [IO.Path]::GetTempPath()) "example-$app"
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

function Send-Push([string] $uri, [string] $token, [string] $payload) {
    $response = Invoke-WebRequest -Method Post -Uri $uri -TimeoutSec 60 -SkipHttpErrorCheck `
        -Headers @{ Authorization = "Bearer $token"; 'X-WNS-Type' = 'wns/raw' } `
        -ContentType 'application/octet-stream' -Body ([Text.Encoding]::UTF8.GetBytes($payload))
    $wns = foreach ($header in 'X-WNS-Status', 'X-WNS-NotificationStatus', 'X-WNS-Error-Description') {
        if ($response.Headers.ContainsKey($header)) { "$header=$($response.Headers[$header] -join ',')" }
    }
    "HTTP $($response.StatusCode) $($wns -join ' ')"
}

# Ends the app's process and reports whether it left within the bound.
function Stop-App([string] $label, [System.Diagnostics.Process] $p) {
    $p.Kill() | Out-Null
    $exited = $p.WaitForExit([int] $ExitBound.TotalMilliseconds)
    if (-not $exited) { Note "- the $label app did not exit within $ExitBound" }
    $exited
}

Note "## X1 on $((Get-CimInstance Win32_OperatingSystem).Caption) $([Environment]::OSVersion.Version), $app"

# The bootstrapper the 2.5 channel install loads, which a desktop machine ships nowhere, fetched as w0.ps1 does.
$bootstrap = Join-Path $dir 'Microsoft.WindowsAppRuntime.Bootstrap.dll'
if (Test-Path $bootstrap) {
    Note '- the bootstrapper already beside the exe'
} else {
    New-Item -ItemType Directory -Force $work | Out-Null
    $package = Join-Path $work 'foundation.zip'
    Invoke-WebRequest -TimeoutSec 600 -OutFile $package `
        "https://api.nuget.org/v3-flatcontainer/microsoft.windowsappsdk.foundation/$Foundation/microsoft.windowsappsdk.foundation.$Foundation.nupkg"
    Expand-Archive -Force $package (Join-Path $work 'foundation')
    Copy-Item (Join-Path $work 'foundation/runtimes/win-x64/native/Microsoft.WindowsAppRuntime.Bootstrap.dll') $bootstrap
    Note '- the bootstrapper copied beside the exe'
}

# The temp log a process started with nothing reading its output writes to.
Remove-Item $log -ErrorAction SilentlyContinue

$process = Start-Process -FilePath $exe -WorkingDirectory $dir -NoNewWindow -PassThru

$uri = $null
$tokenLine = Wait-Until $ChannelBound { First-Line $log 'ui token at_ms=\d+ Wns \{ channel_uri: "([^"]+)"' }
if (-not $tokenLine) {
    Note "- no token within $ChannelBound"
    $passed = $false
} elseif ($tokenLine -match 'channel_uri: "([^"]+)"') {
    $uri = $matches[1]
    Write-Host "::add-mask::$uri"
    Note "- channel created on $(([Uri] $uri).Host), $($tokenLine -replace '^.*ui token at_ms=\d+ ', '')"
} else {
    Note "- the token line has no channel uri: $tokenLine"
    $passed = $false
}

$token = $null
if ($uri) {
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
        $token = $null
        $passed = $false
    }
}

if ($uri -and $token) {
    $sent = [DateTimeOffset]::Now.ToUnixTimeMilliseconds()
    $payload = '{"seq":"1","sent_at_ms":"' + $sent + '"}'
    try {
        $result = Send-Push $uri $token $payload
    } catch {
        $result = "failed: $($_.Exception.Message)"
    }
    Note "- push 1 to the running app: $result"
    $got = Wait-Until $PushBound {
        $ui = First-Line $log 'ui message at_ms=\d+ started_app=false seq=1 '
        $handler = First-Line $log 'handler started_app=false .*seq=1 '
        if ($ui -and $handler) { [pscustomobject]@{ Ui = $ui; Handler = $handler } }
    }
    if ($got) {
        Note "- running: $($got.Ui)"
        Note "- running: $($got.Handler)"
    } else {
        Note "- the running app did not get seq=1 within $PushBound"
        $passed = $false
    }
    if (-not (Stop-App 'running' $process)) { $passed = $false }

    # The same channel still reaches the app once the process is gone.
    $sent = [DateTimeOffset]::Now.ToUnixTimeMilliseconds()
    $payload = '{"seq":"2","sent_at_ms":"' + $sent + '"}'
    try {
        $result = Send-Push $uri $token $payload
    } catch {
        $result = "failed: $($_.Exception.Message)"
    }
    Note "- push 2 with the app stopped: $result"
    $got2 = Wait-Until $ActivationBound { First-Line $log 'handler started_app=true .*seq=2 ' }
    if ($got2) {
        Note "- push-started: $got2"
        $started = Wait-Until $ExitBound { Get-Process -Name $app -ErrorAction SilentlyContinue | Select-Object -First 1 }
        if ($started) {
            if (-not (Stop-App 'push-started' $started)) { $passed = $false }
        } else {
            Note '- the push-started process was already gone when it was looked for'
        }
    } else {
        Note "- the stopped app was not started with seq=2 within $ActivationBound"
        $passed = $false
    }
}

Write-Host "--- $app pushups-example.log"
Get-Content $log -ErrorAction SilentlyContinue | Write-Host

if ($env:GITHUB_STEP_SUMMARY) { $verdict | Add-Content $env:GITHUB_STEP_SUMMARY }
exit ([int] (-not $passed))
