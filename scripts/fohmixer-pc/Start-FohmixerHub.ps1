#Requires -Version 5.1
# The fohmixer-hub task's action (S5 design note section 3 step 6), run inside
# conhost --headless: starts the hub next to this script in the data folder,
# with no console window (a console and a process group of its own, so the
# stop's Ctrl-Break reaches it alone),
# its output in <DataDir>\logs\hub.out.log and hub.err.log (the last 20 runs
# kept as hub.out.<UTC stamp>.log and hub.err.<UTC stamp>.log, #43:
# Move-FohHubLog), and waits for it, so the task runs exactly as long as the
# hub and IgnoreNew keeps one. The hub reads its data folder from
# FOHMIXER_DATA, which a task action cannot set: this script does, and drops an
# inherited PORT (it would override the toml's http_port; RUST_LOG is kept as
# the debugging switch). Stop-FohmixerHub.ps1 stops the hub with Ctrl-Break
# (spec I7). Each start and exit is a line in <DataDir>\logs\hub-launch.log.
param([Parameter(Mandatory)][string]$DataDir)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$logs = Join-Path $DataDir 'logs'
$launchLog = Join-Path $logs 'hub-launch.log'

function Write-LaunchLog([string]$Line) {
    Add-Content -LiteralPath $launchLog -Encoding UTF8 -Value ('{0} {1}' -f (Get-Date).ToString('o'), $Line)
}

try {
    New-Item -ItemType Directory -Force -Path $logs | Out-Null
    Import-Module (Join-Path $PSScriptRoot 'FohmixerPc.psm1') -Force
    $exe = Join-Path $PSScriptRoot 'fohmixer-hub.exe'
    $out = Join-Path $logs 'hub.out.log'
    $err = Join-Path $logs 'hub.err.log'
    foreach ($n in @('hub.out', 'hub.err')) {
        $null = Move-FohHubLog -Logs $logs -Name $n -Keep 20
    }
    $env:FOHMIXER_DATA = $DataDir
    if (Test-Path -LiteralPath 'Env:PORT') { Remove-Item -LiteralPath 'Env:PORT' }
    # Plain log files: tracing-subscriber writes no colour codes with NO_COLOR set.
    $env:NO_COLOR = '1'
    $hub = Start-FohHubProcess -Exe $exe -DataDir $DataDir -Out $out -Err $err
    try {
        Write-LaunchLog "started $exe (pid $($hub.Id))"
        $code = $hub.WaitForExit()
        Write-LaunchLog "pid $($hub.Id) exited with $code"
    } finally {
        $hub.Dispose()
    }
    exit $code
} catch {
    Write-LaunchLog ('failed: ' + $_.Exception.Message)
    exit 1
}
