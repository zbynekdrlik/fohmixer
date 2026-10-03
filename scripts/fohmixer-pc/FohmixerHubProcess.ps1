#Requires -Version 5.1
# The running hub (S5 design note section 3 steps 1 and 7): find it, start it
# (Start-FohmixerHub.ps1), stop it with Ctrl-Break (Stop-FohmixerHub.ps1, spec
# I7: never by force), wait for it; dot-sourced by FohmixerPc.psm1 and shipped
# in the bundle next to it. Windows PowerShell 5.1, ASCII only.

# Ctrl-Break through another process's console (the hub's graceful stop, as in
# iemmixer's iem-win console.rs): detach from this process's console, attach to
# the hub's, send the event to the hub's process group only, detach. A console
# is reachable only from a process in the same session. Never group 0 (every
# process on that console): the sender is on it too while it sends, and no
# handler can keep it alive, because AttachConsole and FreeConsole reset its
# handler table to the default handler, which ends the process
# (SetConsoleCtrlHandler, Remarks). Group 0 ended the stop script itself
# before it wrote its result (the #9 self-test failure).
$script:ConsoleCode = @'
using System;
using System.Runtime.InteropServices;

public static class FohmixerConsole
{
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FreeConsole();

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AttachConsole(uint processId);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GenerateConsoleCtrlEvent(uint ctrlEvent, uint processGroupId);

    const uint CtrlBreakEvent = 1;
    const int ErrorInvalidParameter = 87;

    // Sends Ctrl-Break to the process group processId (a process started with
    // CREATE_NEW_PROCESS_GROUP, whose id is its group's id) on that process's
    // console, and detaches again: 0, or the Win32 error of the failed step.
    // 0 is refused: as a group it means every process on the console, this
    // one included.
    public static int Break(uint processId)
    {
        if (processId == 0) return ErrorInvalidParameter;
        FreeConsole();
        if (!AttachConsole(processId)) return Marshal.GetLastWin32Error();
        int result = 0;
        if (!GenerateConsoleCtrlEvent(CtrlBreakEvent, processId)) result = Marshal.GetLastWin32Error();
        FreeConsole();
        return result;
    }
}
'@

# The hub's start (Start-FohmixerHub.ps1): CreateProcess with CREATE_NO_WINDOW
# and CREATE_NEW_PROCESS_GROUP, as iemmixer's iem-win spawn does for its
# children. The hub gets a console of its own that has no window (nothing on
# the desktop, no hand-off to Windows Terminal as the default terminal) and a
# process group of its own (its pid is the group id: the stop's Ctrl-Break goes
# to that group and to nothing else, FohmixerConsole); its stdout and stderr go
# straight to two files, so it does not depend on the launcher.
$script:SpawnCode = @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public sealed class FohmixerChild : IDisposable
{
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr handle);

    IntPtr process;
    public readonly int Id;

    internal FohmixerChild(IntPtr process, int id)
    {
        this.process = process;
        Id = id;
    }

    // Waits for the process to exit and returns its exit code.
    public int WaitForExit()
    {
        if (WaitForSingleObject(process, 0xFFFFFFFF) != 0) throw new Win32Exception(Marshal.GetLastWin32Error(), "WaitForSingleObject");
        uint code;
        if (!GetExitCodeProcess(process, out code)) throw new Win32Exception(Marshal.GetLastWin32Error(), "GetExitCodeProcess");
        return unchecked((int)code);
    }

    public void Dispose()
    {
        if (process != IntPtr.Zero) { CloseHandle(process); process = IntPtr.Zero; }
    }
}

public static class FohmixerSpawn
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo
    {
        public int cb;
        public string lpReserved;
        public string lpDesktop;
        public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInformation
    {
        public IntPtr hProcess, hThread;
        public int dwProcessId, dwThreadId;
    }

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessW(string applicationName, StringBuilder commandLine, IntPtr processAttributes,
        IntPtr threadAttributes, bool inheritHandles, uint creationFlags, IntPtr environment, string currentDirectory,
        ref StartupInfo startupInfo, out ProcessInformation processInformation);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetHandleInformation(SafeFileHandle handle, uint mask, uint flags);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr handle);

    const uint CreateNoWindow = 0x08000000;
    const uint CreateNewProcessGroup = 0x00000200;
    const int UseStdHandles = 0x00000100;
    const uint HandleFlagInherit = 1;

    // Starts exe (no arguments) in dir with this process's environment, no
    // console window, a process group of its own, no stdin, stdout and stderr
    // written to the two files (created or emptied; readable while it runs).
    public static FohmixerChild Start(string exe, string dir, string outPath, string errPath)
    {
        using (FileStream outFile = new FileStream(outPath, FileMode.Create, FileAccess.Write, FileShare.ReadWrite))
        using (FileStream errFile = new FileStream(errPath, FileMode.Create, FileAccess.Write, FileShare.ReadWrite))
        {
            Inheritable(outFile.SafeFileHandle);
            Inheritable(errFile.SafeFileHandle);
            StartupInfo si = new StartupInfo();
            si.cb = Marshal.SizeOf(typeof(StartupInfo));
            si.dwFlags = UseStdHandles;
            si.hStdOutput = outFile.SafeFileHandle.DangerousGetHandle();
            si.hStdError = errFile.SafeFileHandle.DangerousGetHandle();
            ProcessInformation pi;
            StringBuilder commandLine = new StringBuilder("\"" + exe + "\"");
            if (!CreateProcessW(exe, commandLine, IntPtr.Zero, IntPtr.Zero, true, CreateNoWindow | CreateNewProcessGroup, IntPtr.Zero, dir, ref si, out pi))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateProcess " + exe);
            CloseHandle(pi.hThread);
            return new FohmixerChild(pi.hProcess, pi.dwProcessId);
        }
    }

    static void Inheritable(SafeFileHandle handle)
    {
        if (!SetHandleInformation(handle, HandleFlagInherit, HandleFlagInherit))
            throw new Win32Exception(Marshal.GetLastWin32Error(), "SetHandleInformation");
    }
}
'@

# ---- the running hub: find, stop, wait (design note section 3 steps 1 and 7) ----

function Get-FohHubProcess {
    # The running hubs installed under <DataDir>\app (any version): pid and exe path.
    param([Parameter(Mandatory)][string]$DataDir)
    $prefix = (Join-Path $DataDir 'app') + '\'
    $out = @()
    foreach ($p in @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'fohmixer-hub.exe'")) {
        $path = [string]$p.ExecutablePath
        if ($path -and $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
            $out += [pscustomobject]@{ pid = [int]$p.ProcessId; path = $path }
        }
    }
    return $out
}

function Send-FohCtrlBreak {
    # Ctrl-Break to a hub's process group through its own console: the hub
    # stops gracefully on it (Start-FohHubProcess gives it that group). This
    # detaches the CALLING process from its console for good, so only a
    # process of its own calls it (Stop-FohmixerHub.ps1). Returns 0 or the Win32 error.
    param([Parameter(Mandatory)][int]$ProcessId)
    if ($ProcessId -le 0) { throw "pid $ProcessId refused" }
    $type = 'FohmixerConsole' -as [type]
    if ($null -eq $type) {
        Add-Type -TypeDefinition $script:ConsoleCode -Language CSharp -IgnoreWarnings
        $type = 'FohmixerConsole' -as [type]
    }
    return [int]$type::Break([uint32]$ProcessId)
}

function Start-FohHubProcess {
    # Starts the hub exe in the data folder with this process's environment, no
    # console window, a process group of its own (Send-FohCtrlBreak), its
    # output in the two files (FohmixerSpawn). Returns the child: Id,
    # WaitForExit() (the exit code), Dispose().
    param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$DataDir,
          [Parameter(Mandatory)][string]$Out, [Parameter(Mandatory)][string]$Err)
    $type = 'FohmixerSpawn' -as [type]
    if ($null -eq $type) {
        Add-Type -TypeDefinition $script:SpawnCode -Language CSharp -IgnoreWarnings
        $type = 'FohmixerSpawn' -as [type]
    }
    return $type::Start($Exe, $DataDir, $Out, $Err)
}

function Write-FohStopResult {
    # The stop script's result, <DataDir>\logs\hub-stop.result.json: code 0 = the
    # request reached every hub (or none ran), 1 = a request failed, 2 = the
    # script failed. Conhost does not pass an exit code on (Get-FohTaskCommand).
    param([Parameter(Mandatory)][string]$DataDir, [Parameter(Mandatory)][int]$Code, [Parameter(Mandatory)][string]$Message,
          [Parameter(Mandatory)][datetime]$Started)
    $json = ConvertTo-Json -Compress -InputObject ([ordered]@{ code = $Code; message = $Message; started = $Started.ToString('o') })
    $null = Write-FohText -Path (Join-Path $DataDir 'logs\hub-stop.result.json') -Text $json
}

function Read-FohStopResult {
    # Waits up to $TimeoutSeconds for a result of a stop script run that
    # started at or after $Since (an earlier run's late result is not taken)
    # and returns it.
    param([Parameter(Mandatory)][string]$DataDir, [int]$TimeoutSeconds = 30, [datetime]$Since = [datetime]::MinValue)
    $path = Join-Path $DataDir 'logs\hub-stop.result.json'
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $r = [IO.File]::ReadAllText($path) | ConvertFrom-Json
            if ([datetime]$r.started -ge $Since) { return $r }
        }
        if ((Get-Date) -ge $deadline) {
            throw "the stop task wrote no result within $TimeoutSeconds s ($path); the hub user must be logged on for it to run"
        }
        Start-Sleep -Milliseconds 250
    }
}

function Wait-FohHubExit {
    # Waits up to $TimeoutSeconds for every hub under <DataDir>\app to exit;
    # returns the ones still running. It never ends one (spec I7).
    param([Parameter(Mandatory)][string]$DataDir, [int]$TimeoutSeconds = 10)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        $left = @(Get-FohHubProcess -DataDir $DataDir)
        if ($left.Count -eq 0 -or (Get-Date) -ge $deadline) { return $left }
        Start-Sleep -Milliseconds 250
    }
}

function Stop-FohHub {
    # Design note section 3 step 1: asks every hub under <DataDir>\app to stop
    # and waits. The request is Ctrl-Break, the hub's graceful stop, sent by the
    # stop task from the hub user's own session (a console is reachable only
    # from there); its result file is waited for first (up to
    # $StopTaskSeconds, a failed request fails at once), then the hub's exit (up
    # to $TimeoutSeconds). Task Scheduler's End is not used: for a process
    # without a window it has no graceful request, only a hard end, which our
    # tasks forbid (spec I7). A hub still running after the wait is reported,
    # never ended. Returns what it did and whether a hub was stopped.
    param([Parameter(Mandatory)][string]$DataDir, [int]$TimeoutSeconds = 10, [int]$StopTaskSeconds = 30,
          [string]$TaskPath = $script:TaskPath)
    $running = @(Get-FohHubProcess -DataDir $DataDir)
    if ($running.Count -eq 0) { return [pscustomobject]@{ stopped = $false; text = 'no hub was running' } }
    $pids = ($running | ForEach-Object { $_.pid }) -join ', '
    if ($null -eq (Get-FohTask -Name $script:StopTask -TaskPath $TaskPath)) {
        throw ("a hub runs (pid $pids) but the task $TaskPath$($script:StopTask) is missing: stop the hub with " +
            'Ctrl-Break or Ctrl-C in its console, then run the install again')
    }
    $resultPath = Join-Path $DataDir 'logs\hub-stop.result.json'
    if (Test-Path -LiteralPath $resultPath) { Remove-Item -LiteralPath $resultPath }
    $since = (Get-Date).AddSeconds(-1)
    Start-ScheduledTask -TaskPath $TaskPath -TaskName $script:StopTask
    $result = Read-FohStopResult -DataDir $DataDir -TimeoutSeconds $StopTaskSeconds -Since $since
    if ([int]$result.code -ne 0) {
        throw "the stop task could not ask the hub (pid $pids) to stop: $($result.message) (code $($result.code)); the hub still runs"
    }
    $left = @(Wait-FohHubExit -DataDir $DataDir -TimeoutSeconds $TimeoutSeconds)
    if ($left.Count -gt 0) {
        $still = ($left | ForEach-Object { $_.pid }) -join ', '
        throw ("the hub (pid $still) did not stop within $TimeoutSeconds s of Ctrl-Break (see $DataDir\logs\hub.out.log). " +
            'It is not ended by force (spec I7): stop it in its session, then run the install again')
    }
    Wait-FohTaskIdle -Name $script:HubTask -TaskPath $TaskPath
    return [pscustomobject]@{ stopped = $true; text = "stopped the hub (pid $pids)" }
}

function Get-FohLogStamp {
    # The rotation stamp of a hub log (#43): its last write time in UTC,
    # yyyyMMdd-HHmmss-fff (the names sort by time).
    param([Parameter(Mandatory)][datetime]$Time)
    return $Time.ToUniversalTime().ToString('yyyyMMdd-HHmmss-fff', [Globalization.CultureInfo]::InvariantCulture)
}

function Move-FohHubLog {
    # Keeps the last $Keep starts of one hub log (#43, design note section
    # 3.3; $Name: hub.out or hub.err) in the folder $Logs, run by the launcher
    # before each start: the run that ended, <name>.log, becomes
    # <name>.<stamp of its last write>.log, a <name>.log.prev of an older
    # launcher joins them the same way, and the oldest past $Keep are
    # deleted. Returns the names kept, oldest first.
    param([Parameter(Mandatory)][string]$Logs, [Parameter(Mandatory)][string]$Name, [int]$Keep = 20)
    foreach ($f in @((Join-Path $Logs ($Name + '.log')), (Join-Path $Logs ($Name + '.log.prev')))) {
        if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { continue }
        $stamp = Get-FohLogStamp -Time (Get-Item -LiteralPath $f).LastWriteTimeUtc
        $target = Join-Path $Logs ('{0}.{1}.log' -f $Name, $stamp)
        $n = 0
        while (Test-Path -LiteralPath $target) {
            $n++
            $target = Join-Path $Logs ('{0}.{1}-{2}.log' -f $Name, $stamp, $n)
        }
        Move-Item -LiteralPath $f -Destination $target
    }
    $pattern = '^' + [regex]::Escape($Name) + '\.\d{8}-\d{6}-\d{3}(-\d+)?\.log$'
    $rotated = @(Get-ChildItem -LiteralPath $Logs -File | Where-Object { $_.Name -match $pattern } | Sort-Object Name)
    $excess = $rotated.Count - $Keep
    if ($excess -gt 0) {
        $rotated | Select-Object -First $excess | ForEach-Object { Remove-Item -LiteralPath $_.FullName }
    }
    return @($rotated | Select-Object -Skip ([Math]::Max($excess, 0)) | ForEach-Object { $_.Name })
}

function Get-FohLogTail {
    # The last lines of the hub's logs, for an error message.
    param([Parameter(Mandatory)][string]$DataDir, [int]$Lines = 8)
    $out = @()
    foreach ($n in @('hub.err.log', 'hub.out.log', 'hub-launch.log')) {
        $p = Join-Path $DataDir "logs\$n"
        if (Test-Path -LiteralPath $p -PathType Leaf) {
            $out += "--- $n"
            $out += @(Get-Content -LiteralPath $p -Tail $Lines)
        }
    }
    return ($out -join "`n")
}

function Wait-FohHubReady {
    # Design note section 3 step 7: polls http://127.0.0.1:<HttpPort>/api/version
    # until the hub answers, up to $TimeoutSeconds, and returns the answer. An
    # answer with another version (an old hub still serving) fails at once.
    param([Parameter(Mandatory)][int]$HttpPort, [Parameter(Mandatory)][string]$Version, [int]$TimeoutSeconds = 20)
    $uri = 'http://127.0.0.1:{0}/api/version' -f $HttpPort
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $last = 'no attempt'
    while ((Get-Date) -lt $deadline) {
        $answer = $null
        try {
            $answer = (Invoke-WebRequest -Uri $uri -UseBasicParsing -TimeoutSec 2).Content | ConvertFrom-Json
        } catch {
            $last = $_.Exception.Message
        }
        if ($null -ne $answer) {
            if ([string]$answer.version -cne $Version) { throw "$uri answers version $($answer.version), not $Version" }
            return $answer
        }
        Start-Sleep -Milliseconds 500
    }
    throw "the hub did not answer $uri within $TimeoutSeconds s (last error: $last)"
}
