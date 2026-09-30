#Requires -Version 5.1
# Our scheduled tasks (S5 design note section 3 step 6; spec D9): how a task
# runs a bundle script, the read-back, the hub's two tasks (the tray's are in
# FohmixerTray.ps1); dot-sourced by FohmixerPc.psm1 and shipped in the bundle
# next to it. Windows PowerShell 5.1, ASCII only.

# ---- a task: find it, wait for it ----

function Get-FohTask {
    # One of our tasks, or $null. Listed, not asked for by name: a missing task
    # folder is then no error.
    param([Parameter(Mandatory)][string]$Name, [string]$TaskPath = $script:TaskPath)
    return (@(Get-ScheduledTask) | Where-Object { $_.TaskPath -eq $TaskPath -and $_.TaskName -eq $Name } | Select-Object -First 1)
}

function Wait-FohTaskIdle {
    # Waits until the task no longer runs (its action exited), so that a start
    # is not ignored (IgnoreNew); throws after $TimeoutSeconds.
    param([Parameter(Mandatory)][string]$Name, [string]$TaskPath = $script:TaskPath, [int]$TimeoutSeconds = 5)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        $t = Get-FohTask -Name $Name -TaskPath $TaskPath
        if ($null -eq $t -or "$($t.State)" -ne 'Running') { return }
        if ((Get-Date) -ge $deadline) { throw "the task $TaskPath$Name still runs $TimeoutSeconds s after its program stopped" }
        Start-Sleep -Milliseconds 250
    }
}

# ---- the scheduled tasks (design note section 3 step 6; spec D9) ----

function Get-FohTaskCommand {
    # How a task runs one of the bundle's scripts: Windows PowerShell 5.1 inside
    # conhost --headless, a console without a window. A console program a task
    # starts shows a window otherwise, one of Windows Terminal where Terminal is
    # the default terminal, which -WindowStyle Hidden cannot hide. Conhost does
    # not pass the script's exit code on: the stop script reports through a
    # result file, the launcher through hub-launch.log.
    param([Parameter(Mandatory)][string]$Script, [Parameter(Mandatory)][string]$DataDir)
    $system = Join-Path $env:SystemRoot 'System32'
    $ps = Join-Path $system 'WindowsPowerShell\v1.0\powershell.exe'
    return [pscustomobject]@{
        execute = (Join-Path $system 'conhost.exe')
        arguments = ('--headless ' + (Format-FohArg $ps) + ' -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File ' +
            (Format-FohArg $Script) + ' -DataDir ' + (Format-FohArg $DataDir))
    }
}

function Get-FohSid {
    # An account's SID, looked up as Get-FohAccountSid does (a task reads its
    # user back with or without the computer part: both give the same SID); a
    # SID is taken as it is. An account that does not resolve gives '' (the
    # caller reports it as a difference).
    param([AllowEmptyString()][string]$Account)
    if (-not $Account) { return '' }
    if ($Account -cmatch '^S-1-[0-9-]+\z') { return $Account }
    try {
        return (Get-FohAccountSid $Account).Value
    } catch {
        return ''
    }
}

function Get-FohTaskProblems {
    # What one of our tasks must read back as (Interactive for the user,
    # Limited, no time limit, IgnoreNew, no battery or idle stop, never ended
    # hard, normal priority; an -AtLogon task, the hub's or the tray's, starts
    # at the user's logon and may be restarted 3 x 1 min when it fails to
    # start); returns the differences.
    param([Parameter(Mandatory)]$Task, [Parameter(Mandatory)][string]$User, [Parameter(Mandatory)][string]$Execute,
          [Parameter(Mandatory)][string]$Arguments, [Parameter(Mandatory)][string]$WorkDir, [switch]$AtLogon)
    $bad = @()
    $p = $Task.Principal
    $s = $Task.Settings
    $userSid = Get-FohSid $User
    if ("$($p.LogonType)" -ne 'Interactive') { $bad += "logon type $($p.LogonType)" }
    if ("$($p.RunLevel)" -ne 'Limited') { $bad += "run level $($p.RunLevel)" }
    if (-not $userSid -or (Get-FohSid ([string]$p.UserId)) -ne $userSid) { $bad += "user $($p.UserId)" }
    if ($s.ExecutionTimeLimit -ne 'PT0S') { $bad += "time limit $($s.ExecutionTimeLimit)" }
    if ("$($s.MultipleInstances)" -ne 'IgnoreNew') { $bad += "instances $($s.MultipleInstances)" }
    if ($s.DisallowStartIfOnBatteries -or $s.StopIfGoingOnBatteries) { $bad += 'stops on batteries' }
    if ($s.IdleSettings.StopOnIdleEnd) { $bad += 'stops at idle end' }
    if ($s.AllowHardTerminate) { $bad += 'may be ended hard' }
    if ($s.Priority -ne 4) { $bad += "priority $($s.Priority)" }
    $restart = '{0}x{1}' -f $s.RestartCount, $s.RestartInterval
    $wantRestart = '0x'
    if ($AtLogon) { $wantRestart = '3xPT1M' }
    if ($restart -ne $wantRestart) { $bad += "restart $restart" }
    $actions = @($Task.Actions)
    if ($actions.Count -ne 1 -or $actions[0].Execute -ne $Execute -or $actions[0].Arguments -ne $Arguments -or
        $actions[0].WorkingDirectory -ne $WorkDir) {
        $bad += ('action ' + (($actions | ForEach-Object { "$($_.Execute) $($_.Arguments) in $($_.WorkingDirectory)" }) -join ' | '))
    }
    $triggers = @($Task.Triggers | Where-Object { $null -ne $_ })
    if ($AtLogon) {
        if ($triggers.Count -ne 1 -or $triggers[0].CimClass.CimClassName -ne 'MSFT_TaskLogonTrigger' -or
            (Get-FohSid ([string]$triggers[0].UserId)) -ne $userSid) {
            $bad += "triggers: not one logon trigger for $User"
        }
    } elseif ($triggers.Count -ne 0) {
        $bad += "$($triggers.Count) triggers"
    }
    return $bad
}

function Register-FohTask {
    # Registers (or updates) one of our tasks for $User (Interactive, Limited,
    # no time limit, IgnoreNew, never ended hard; with -AtLogon a logon trigger
    # for $User and a restart of a failed start, 3 x 1 min) and reads it back:
    # returns the differences, each prefixed with the task's name. It is not
    # started here.
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$Execute,
          [Parameter(Mandatory)][string]$Arguments, [Parameter(Mandatory)][string]$WorkDir,
          [Parameter(Mandatory)][string]$User, [Parameter(Mandatory)][string]$Description,
          [string]$TaskPath = $script:TaskPath, [switch]$AtLogon)
    $account = Get-FohLocalAccountName $User
    $principal = New-ScheduledTaskPrincipal -UserId $account -LogonType Interactive -RunLevel Limited
    $common = @{
        ExecutionTimeLimit = [TimeSpan]::Zero; MultipleInstances = 'IgnoreNew'; AllowStartIfOnBatteries = $true
        DontStopIfGoingOnBatteries = $true; DontStopOnIdleEnd = $true; DisallowHardTerminate = $true; Priority = 4
    }
    if ($AtLogon) {
        $settings = New-ScheduledTaskSettingsSet @common -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
    } else {
        $settings = New-ScheduledTaskSettingsSet @common
    }
    $action = New-ScheduledTaskAction -Execute $Execute -Argument $Arguments -WorkingDirectory $WorkDir
    $register = @{
        TaskPath = $TaskPath; TaskName = $Name; Action = $action; Principal = $principal
        Settings = $settings; Description = $Description; Force = $true
    }
    if ($AtLogon) { $register['Trigger'] = New-ScheduledTaskTrigger -AtLogOn -User $account }
    Register-ScheduledTask @register | Out-Null
    $task = Get-ScheduledTask -TaskPath $TaskPath -TaskName $Name
    $problems = @()
    foreach ($b in @(Get-FohTaskProblems -Task $task -User $User -Execute $Execute -Arguments $Arguments -WorkDir $WorkDir -AtLogon:$AtLogon)) {
        $problems += ('{0}: {1}' -f $Name, $b)
    }
    return $problems
}

function Register-FohHubTasks {
    # Registers (or updates) our two hub tasks for the hub's user and reads
    # them back: fohmixer-hub runs this bundle's Start-FohmixerHub.ps1 at the
    # user's logon; fohmixer-hub-stop (no trigger) runs its Stop-FohmixerHub.ps1
    # on demand, in the user's session. Neither is started here. Throws on a
    # read-back difference. The hub task's restart setting (3 x 1 min) covers a
    # failed start only: Task Scheduler does not restart a program that exits,
    # so a hub that crashes stays down until the next logon or a task start.
    param([Parameter(Mandatory)][string]$AppDir, [Parameter(Mandatory)][string]$DataDir,
          [Parameter(Mandatory)][string]$User, [string]$TaskPath = $script:TaskPath)
    $specs = @(
        @{ name = $script:HubTask; script = (Join-Path $AppDir 'Start-FohmixerHub.ps1'); logon = $true
           description = 'fohmixer: the hub (S5). Stop it only through fohmixer-hub-stop (Ctrl-Break).' },
        @{ name = $script:StopTask; script = (Join-Path $AppDir 'Stop-FohmixerHub.ps1'); logon = $false
           description = 'fohmixer: asks the running hub to stop (Ctrl-Break in its session).' })
    $problems = @()
    foreach ($spec in $specs) {
        $cmd = Get-FohTaskCommand -Script $spec.script -DataDir $DataDir
        $problems += @(Register-FohTask -Name $spec.name -Execute $cmd.execute -Arguments $cmd.arguments -WorkDir $DataDir `
                -User $User -Description $spec.description -TaskPath $TaskPath -AtLogon:$spec.logon)
    }
    if ($problems.Count -gt 0) { throw ('task read-back: ' + ($problems -join '; ')) }
}
