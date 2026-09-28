#Requires -Version 5.1
# The fohmixer-hub-stop task's action (S5 design note section 3 step 1), run
# inside conhost --headless: asks every hub under <DataDir>\app to stop with
# Ctrl-Break, its graceful stop, through the hub's own console, which only a
# process in the hub's session can reach (the task runs in the hub user's
# session), sent to the hub's process group only: this process is on that
# console while it sends and is never a target itself. It never ends a
# process (spec I7): Install-Fohmixer.ps1 waits for
# the hub to exit. The result goes to <DataDir>\logs\hub-stop.result.json
# (conhost passes no exit code on), with this run's start time: code 0 = the
# request reached every hub (or none runs), 1 = a request failed, 2 = this
# script failed; the exit code is the same. Each run appends to
# <DataDir>\logs\hub-stop.log. This process detaches from its own console to
# send the event, so it writes nothing to a console.
param([Parameter(Mandatory)][string]$DataDir)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$started = Get-Date
$logs = Join-Path $DataDir 'logs'
$stopLog = Join-Path $logs 'hub-stop.log'

function Write-StopLog([string]$Line) {
    Add-Content -LiteralPath $stopLog -Encoding UTF8 -Value ('{0} {1}' -f (Get-Date).ToString('o'), $Line)
}

try {
    New-Item -ItemType Directory -Force -Path $logs | Out-Null
    Import-Module (Join-Path $PSScriptRoot 'FohmixerPc.psm1') -Force
    $hubs = @(Get-FohHubProcess -DataDir $DataDir)
    $lines = @()
    $failed = 0
    foreach ($h in $hubs) {
        $code = Send-FohCtrlBreak -ProcessId $h.pid
        if ($code -eq 0) {
            $lines += "Ctrl-Break sent to pid $($h.pid) ($($h.path))"
        } else {
            $failed++
            $lines += "Ctrl-Break to pid $($h.pid) failed: Win32 error $code"
        }
    }
    if ($hubs.Count -eq 0) { $lines += 'no hub running' }
    $result = 0
    if ($failed -gt 0) { $result = 1 }
    $message = $lines -join '; '
    Write-FohStopResult -DataDir $DataDir -Code $result -Message $message -Started $started
    Write-StopLog $message
    exit $result
} catch {
    # The result first (the install waits for it), each write on its own.
    $message = 'failed: ' + $_.Exception.Message
    try {
        if (Get-Command -Name Write-FohStopResult -ErrorAction SilentlyContinue) {
            Write-FohStopResult -DataDir $DataDir -Code 2 -Message $message -Started $started
        }
    } catch {
        $message += ' (and writing the result failed: ' + $_.Exception.Message + ')'
    }
    try { Write-StopLog $message } catch { $null = $_ }
    exit 2
}
