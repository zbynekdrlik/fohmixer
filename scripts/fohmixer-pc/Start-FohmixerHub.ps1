#Requires -Version 5.1
# The fohmixer-hub task's action (S5 design note section 3 step 6): runs the hub
# next to this script with its data folder, in a console of its own without a
# window, its output in <DataDir>\logs\hub.out.log and hub.err.log (the previous
# run's kept as *.prev), and waits for it, so the task runs exactly as long as
# the hub and IgnoreNew keeps one. The hub reads its data folder from
# FOHMIXER_DATA, which a task action cannot set: this script does.
# Stop-FohmixerHub.ps1 stops the hub with Ctrl-Break through that console (spec
# I7); this script then exits with the hub's exit code. Each start and exit is a
# line in <DataDir>\logs\hub-launch.log.
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
    $exe = Join-Path $PSScriptRoot 'fohmixer-hub.exe'
    $out = Join-Path $logs 'hub.out.log'
    $err = Join-Path $logs 'hub.err.log'
    foreach ($f in @($out, $err)) {
        if (Test-Path -LiteralPath $f) { Move-Item -LiteralPath $f -Destination ($f + '.prev') -Force }
    }
    $env:FOHMIXER_DATA = $DataDir
    # Plain log files: tracing-subscriber writes no colour codes with NO_COLOR set.
    $env:NO_COLOR = '1'
    # -WindowStyle Hidden with redirected output: a console of its own (Ctrl-Break
    # reaches the hub alone), no window.
    $hub = Start-Process -FilePath $exe -WorkingDirectory $DataDir -WindowStyle Hidden `
        -RedirectStandardOutput $out -RedirectStandardError $err -PassThru
    # Windows PowerShell 5.1 reports the exit code only when the handle was taken while it ran.
    $null = $hub.Handle
    Write-LaunchLog "started $exe (pid $($hub.Id))"
    $hub.WaitForExit()
    Write-LaunchLog "pid $($hub.Id) exited with $($hub.ExitCode)"
    exit $hub.ExitCode
} catch {
    Write-LaunchLog ('failed: ' + $_.Exception.Message)
    exit 1
}
