#Requires -Version 5.1
# fohmixer tray (#39): dot-sourced by FohmixerPc.psm1 (its helpers
# Format-FohArg, Get-FohTask, Wait-FohTaskIdle and Register-FohTask are used
# here). Windows PowerShell 5.1, ASCII only.
# - The tray (fohmixer-tray.exe, in the bundle next to the hub) runs in the
#   band user's session: the task fohmixer-tray starts it at that user's logon
#   (Interactive, Limited, never ended hard), as the hub's task does.
# - It is never ended by force (spec I7). A second start of the exe with
#   --exit hands that argument to the running tray (Tauri's single-instance
#   plugin, reachable only from the same session), which then exits; the task
#   fohmixer-tray-stop runs that second start in the band user's session, as
#   fohmixer-hub-stop does for the hub. The install waits for the exit and
#   fails, never ends the tray, when it stays.
# - No site value lives here (spec 5.2): users, folders and ports come as
#   parameters.

$script:TrayExe = 'fohmixer-tray.exe'
$script:TrayTask = 'fohmixer-tray'
$script:TrayStopTask = 'fohmixer-tray-stop'

function Get-FohTrayProcess {
    # The running trays installed under <DataDir>\app (any version): pid and exe path.
    param([Parameter(Mandatory)][string]$DataDir)
    $prefix = (Join-Path $DataDir 'app') + '\'
    $out = @()
    foreach ($p in @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'fohmixer-tray.exe'")) {
        $path = [string]$p.ExecutablePath
        if ($path -and $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
            $out += [pscustomobject]@{ pid = [int]$p.ProcessId; path = $path }
        }
    }
    return $out
}

function Get-FohTrayCommand {
    # How the tray tasks run the tray of $AppDir: with the hub's data folder
    # (its config names the ports and the public name), or with -Stop as the
    # second start that asks the running tray to exit.
    param([Parameter(Mandatory)][string]$AppDir, [Parameter(Mandatory)][string]$DataDir, [switch]$Stop)
    $arguments = '--data ' + (Format-FohArg $DataDir)
    if ($Stop) { $arguments = '--exit' }
    return [pscustomobject]@{ execute = (Join-Path $AppDir $script:TrayExe); arguments = $arguments }
}

function Register-FohTrayTasks {
    # Registers (or updates) the two tray tasks for the band user and reads
    # them back: fohmixer-tray runs this bundle's tray at the user's logon
    # (restarted 3 x 1 min when its start fails); fohmixer-tray-stop (no
    # trigger) runs its --exit on demand, in the user's session. Neither is
    # started here. Throws on a read-back difference.
    param([Parameter(Mandatory)][string]$AppDir, [Parameter(Mandatory)][string]$DataDir,
          [Parameter(Mandatory)][string]$User, [string]$TaskPath = $script:TaskPath)
    $specs = @(
        @{ name = $script:TrayTask; cmd = (Get-FohTrayCommand -AppDir $AppDir -DataDir $DataDir); logon = $true
           description = 'fohmixer: the tray (#39): the hub state, Open fohmixer, Copy URL. End it with its Exit or fohmixer-tray-stop.' },
        @{ name = $script:TrayStopTask; cmd = (Get-FohTrayCommand -AppDir $AppDir -DataDir $DataDir -Stop); logon = $false
           description = 'fohmixer: asks the running tray to exit (a second start with --exit in its session).' })
    $problems = @()
    foreach ($spec in $specs) {
        $problems += @(Register-FohTask -Name $spec.name -Execute $spec.cmd.execute -Arguments $spec.cmd.arguments -WorkDir $DataDir `
                -User $User -Description $spec.description -TaskPath $TaskPath -AtLogon:$spec.logon)
    }
    if ($problems.Count -gt 0) { throw ('task read-back: ' + ($problems -join '; ')) }
}

function Wait-FohTrayExit {
    # Waits up to $TimeoutSeconds for every tray under <DataDir>\app to exit;
    # returns the ones still running. It never ends one (spec I7).
    param([Parameter(Mandatory)][string]$DataDir, [int]$TimeoutSeconds = 10)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        $left = @(Get-FohTrayProcess -DataDir $DataDir)
        if ($left.Count -eq 0 -or (Get-Date) -ge $deadline) { return $left }
        Start-Sleep -Milliseconds 250
    }
}

function Stop-FohTray {
    # Asks every tray under <DataDir>\app to exit and waits (up to
    # $TimeoutSeconds): the stop task starts the tray again with --exit in the
    # band user's session, and the running tray exits on it. A tray still
    # running after the wait is reported, never ended (spec I7). Returns what
    # it did and whether a tray was stopped.
    param([Parameter(Mandatory)][string]$DataDir, [int]$TimeoutSeconds = 10, [string]$TaskPath = $script:TaskPath)
    $running = @(Get-FohTrayProcess -DataDir $DataDir)
    if ($running.Count -eq 0) { return [pscustomobject]@{ stopped = $false; text = 'no tray was running' } }
    $pids = ($running | ForEach-Object { $_.pid }) -join ', '
    if ($null -eq (Get-FohTask -Name $script:TrayStopTask -TaskPath $TaskPath)) {
        throw ("a tray runs (pid $pids) but the task $TaskPath$($script:TrayStopTask) is missing: end the tray with Exit " +
            'in its menu, then run the install again')
    }
    Start-ScheduledTask -TaskPath $TaskPath -TaskName $script:TrayStopTask
    $left = @(Wait-FohTrayExit -DataDir $DataDir -TimeoutSeconds $TimeoutSeconds)
    if ($left.Count -gt 0) {
        $still = ($left | ForEach-Object { $_.pid }) -join ', '
        throw ("the tray (pid $still) did not exit within $TimeoutSeconds s of --exit (see the band user's " +
            '%LOCALAPPDATA%\fohmixer\logs\fohmixer-tray.log.<date>). It is not ended by force (spec I7): end it with ' +
            'Exit in its menu, then run the install again')
    }
    Wait-FohTaskIdle -Name $script:TrayTask -TaskPath $TaskPath
    return [pscustomobject]@{ stopped = $true; text = "stopped the tray (pid $pids)" }
}

function Start-FohTray {
    # Starts the tray task (the tray in the band user's session) and waits up
    # to $TimeoutSeconds for the tray of $AppDir to run, then $SettleSeconds
    # more to see it stay; returns its pid, throws otherwise.
    param([Parameter(Mandatory)][string]$AppDir, [Parameter(Mandatory)][string]$DataDir, [int]$TimeoutSeconds = 15,
          [int]$SettleSeconds = 2, [string]$TaskPath = $script:TaskPath)
    Start-ScheduledTask -TaskPath $TaskPath -TaskName $script:TrayTask
    $exe = Join-Path $AppDir $script:TrayExe
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        $mine = @(Get-FohTrayProcess -DataDir $DataDir | Where-Object { $_.path -eq $exe })
        if ($mine.Count -gt 0) { break }
        if ((Get-Date) -ge $deadline) {
            throw ("the task $TaskPath$($script:TrayTask) started no tray ($exe) within $TimeoutSeconds s; the band user " +
                'must be logged on for it to run')
        }
        Start-Sleep -Milliseconds 250
    }
    Start-Sleep -Seconds $SettleSeconds
    $still = @(Get-FohTrayProcess -DataDir $DataDir | Where-Object { $_.pid -eq $mine[0].pid })
    if ($still.Count -eq 0) {
        throw ("the tray (pid $($mine[0].pid)) exited within $SettleSeconds s of its start (see the band user's " +
            '%LOCALAPPDATA%\fohmixer\logs\fohmixer-tray.log.<date>)')
    }
    return $mine[0].pid
}
