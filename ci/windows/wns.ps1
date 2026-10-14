# The WNS helper layer dot-sourced at the top of w0.ps1 and example.ps1.

# The Foundation package metapackage 2.5.1 depends on, the release ci/windows/bindings.sh pins.
$Foundation = '2.3.12'
# WNS may take minutes to answer a channel request.
$ChannelBound = [TimeSpan]::FromSeconds(240)
$PushBound = [TimeSpan]::FromSeconds(90)
$ExitBound = [TimeSpan]::FromSeconds(20)

foreach ($name in 'WNS_TENANT_ID', 'WNS_APP_ID', 'WNS_OBJECT_ID', 'WNS_CLIENT_SECRET') {
    if (-not [Environment]::GetEnvironmentVariable($name)) { throw "$name is not set" }
}

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

function Get-WnsToken {
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
        $token
    } catch {
        $reason = if ($_.ErrorDetails) { $_.ErrorDetails.Message } else { $_.Exception.Message }
        Note "- access token refused: $reason"
        $null
    }
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

function Write-Step-Summary {
    if ($env:GITHUB_STEP_SUMMARY) { $verdict | Add-Content $env:GITHUB_STEP_SUMMARY }
}
