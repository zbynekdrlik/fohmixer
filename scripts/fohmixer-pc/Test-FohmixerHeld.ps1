#Requires -Version 5.1
# Self-test of an install whose checking hub's image stays held (#250),
# dot-sourced by Test-Fohmixer.ps1 inside its try (its helpers and
# variables: Assert, Invoke-Ps, Get-InstallArgs, $install, $repo, $base,
# $TrayExe, $deviceDir, $sha). A test hub, a small console exe compiled
# here, is bundled as fohmixer-hub.exe: on any check it starts a copy of
# itself that inherits no handle and keeps running from the unpacked folder
# (its image held) until a release file appears or its time is up, writes
# that copy's pid to a file, then refuses the config (or, with
# FOHMIXER_TEST_PASS=1, accepts it). Without its variables it starts nothing.
# Covered (the install's output read without whitespace: the console host
# wraps a warning line at the window's width):
# - refused, held past the install's waits: the install reports the hub's
#   refusal, never the cleanup's error, warns that the exe stays in use, and
#   warns of the folder it left;
# - refused, held for 2 s: the install reports the refusal, logs its wait
#   and leaves no unpacked bundle and no check file;
# - accepted, held past the waits: the install refuses ("stays in use",
#   nothing changed): no toml, no app\<version>;
# - the files-in-use helpers (FohmixerHeld.ps1) in this process: a file this
#   process holds is held, its holders name this process, the wait ends held
#   after its bound, the leftover's removal waits, then warns with its holder
#   and never throws; once released, free and removed;
# - the issue's loop: ten rounds as an install does them (a fresh copy of the
#   real hub in a folder of its own, its config check returning with the exe
#   free, the folder moved and removed); the waits, with their holders, and
#   Defender's state are printed (the evidence of what held it).
# The copies end by themselves (the release file, their time): nothing is
# ended by force (spec I7).
Set-StrictMode -Version Latest

$heldSource = @'
using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class FohmixerTestHub {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int cb; public string reserved; public string desktop; public string title;
        public int x, y, xSize, ySize, xChars, yChars, fill, flags;
        public short show, reserved2; public IntPtr reserved3, stdIn, stdOut, stdErr;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr process, thread; public int pid, tid; }
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcess(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags,
        IntPtr env, string dir, ref StartupInfo si, out ProcessInfo pi);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr h);

    public static int Main(string[] args) {
        if (args.Length == 3 && args[0] == "hold") {
            // Runs (its image held) until the release file appears or the time is up.
            DateTime until = DateTime.UtcNow.AddMilliseconds(int.Parse(args[1]));
            while (DateTime.UtcNow < until && !File.Exists(args[2])) { Thread.Sleep(50); }
            return 0;
        }
        string self = Process.GetCurrentProcess().MainModule.FileName;
        string ms = Environment.GetEnvironmentVariable("FOHMIXER_TEST_HOLD_MS");
        string release = Environment.GetEnvironmentVariable("FOHMIXER_TEST_RELEASE");
        string pidFile = Environment.GetEnvironmentVariable("FOHMIXER_TEST_PIDFILE");
        int holdMs;
        // Without its three variables it starts nothing (a copy that is not in
        // hold mode would start another).
        if (string.IsNullOrEmpty(release) || string.IsNullOrEmpty(pidFile) || !int.TryParse(ms, out holdMs) || holdMs <= 0) {
            Console.Error.WriteLine("the test hub needs FOHMIXER_TEST_HOLD_MS, FOHMIXER_TEST_RELEASE and FOHMIXER_TEST_PIDFILE");
            return 4;
        }
        StartupInfo si = new StartupInfo();
        si.cb = Marshal.SizeOf(typeof(StartupInfo));
        StringBuilder cmd = new StringBuilder("\"" + self + "\" hold " + holdMs + " \"" + release + "\"");
        ProcessInfo pi;
        // No inherited handle (the check's output pipe ends with this process),
        // no window (CREATE_NO_WINDOW), its working folder outside the bundle.
        if (!CreateProcess(self, cmd, IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, Path.GetTempPath(), ref si, out pi)) {
            Console.Error.WriteLine("the test hub could not start its copy: error " + Marshal.GetLastWin32Error());
            return 3;
        }
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
        File.WriteAllText(pidFile, pi.pid.ToString());
        // FOHMIXER_TEST_PASS=1: the check passes (its image still held by the copy).
        if (Environment.GetEnvironmentVariable("FOHMIXER_TEST_PASS") == "1") {
            Console.Error.WriteLine("fohmixer-hub: the test hub accepts it: OK");
            return 0;
        }
        Console.Error.WriteLine("refused by the test hub (#250)");
        return 2;
    }
}
'@

$heldDir = Join-Path $base 'held'
New-Item -ItemType Directory -Force -Path $heldDir | Out-Null
$heldExe = Join-Path $heldDir 'test-hub.exe'
Add-Type -TypeDefinition $heldSource -Language CSharp -OutputAssembly $heldExe -OutputType ConsoleApplication
$heldOut = Join-Path $heldDir 'out'
New-Item -ItemType Directory -Force -Path $heldOut | Out-Null
$heldBundle = New-FohBundle -RepoRoot $repo -HubExe $heldExe -TrayExe $TrayExe -DeviceDir $deviceDir -OutDir $heldOut -Sha $sha

function Wait-HeldCopy([string]$PidFile) {
    # The test hub's copy, ended by itself (the release file or its time): waited for, never ended.
    if (-not (Test-Path -LiteralPath $PidFile -PathType Leaf)) { return }
    $copy = Get-Process -Id ([int]([IO.File]::ReadAllText($PidFile).Trim())) -ErrorAction SilentlyContinue
    if ($null -ne $copy) {
        if (-not $copy.WaitForExit(60000)) { throw "the test hub's copy (pid $($copy.Id)) still runs after 60 s" }
    }
}

function Get-HeldLeftovers([string]$DataDir) {
    $app = Join-Path $DataDir 'app'
    if (-not (Test-Path -LiteralPath $app)) { return @() }
    return @(Get-ChildItem -LiteralPath $app -Force | Where-Object { $_.Name.StartsWith('.') } | ForEach-Object { $_.Name })
}

function Get-Flat([string]$Text) {
    # The install's output without whitespace: Windows PowerShell's console
    # host wraps a warning line at the window's width, also when redirected.
    return ($Text -replace '\s', '')
}

foreach ($c in @(
        # Refused, past the install's waits (released by the test after the install returns).
        @{ what = 'a-held-hub-image'; ms = 120000; pass = $false },
        # Refused, 2 s: within the install's wait.
        @{ what = 'a-briefly-held-hub-image'; ms = 2000; pass = $false },
        # Passed, past the install's waits: refused before any change.
        @{ what = 'a-passed-check-and-a-held-hub-image'; ms = 120000; pass = $true })) {
    $dataH = Join-Path $heldDir ('data-' + $c.what)
    $pidFile = Join-Path $heldDir ('pid-' + $c.what)
    $release = Join-Path $heldDir ('release-' + $c.what)
    $env:FOHMIXER_TEST_HOLD_MS = [string]$c.ms
    $env:FOHMIXER_TEST_RELEASE = $release
    $env:FOHMIXER_TEST_PIDFILE = $pidFile
    if ($c.pass) { $env:FOHMIXER_TEST_PASS = '1' }
    try {
        $res = Invoke-Ps $install (Get-InstallArgs @{ BundleZip = $heldBundle.zip; DataDir = $dataH })
    } finally {
        foreach ($v in @('FOHMIXER_TEST_HOLD_MS', 'FOHMIXER_TEST_RELEASE', 'FOHMIXER_TEST_PIDFILE', 'FOHMIXER_TEST_PASS')) {
            if (Test-Path -LiteralPath "Env:$v") { Remove-Item -LiteralPath "Env:$v" }
        }
        [IO.File]::WriteAllText($release, 'released')
        Wait-HeldCopy -PidFile $pidFile
    }
    Assert (Test-Path -LiteralPath $pidFile -PathType Leaf) "install-with-$($c.what)-ran-the-test-hub"
    $flat = Get-Flat $res.out
    $left = @(Get-HeldLeftovers -DataDir $dataH)
    if ($c.pass) {
        Assert ($res.code -ne 0 -and $flat.Contains((Get-Flat 'stays in use')) -and $flat.Contains((Get-Flat 'nothing changed, run the install again'))) "install-with-$($c.what)-refuses-it"
        # Refused at the config check itself: before its "accepts" line, the layout check, the DACL and the stop.
        Assert (-not $flat.Contains((Get-Flat 'the new hub accepts the config'))) "install-with-$($c.what)-refuses-it-at-the-config-check"
        Assert (-not (Test-Path -LiteralPath (Join-Path $dataH 'fohmixer-hub.toml')) -and -not (Test-Path -LiteralPath (Join-Path $dataH "app\$($heldBundle.version)"))) "install-with-$($c.what)-changes-nothing"
    } else {
        Assert ($res.code -ne 0 -and $res.out.Contains('refused by the test hub (#250)')) "install-with-$($c.what)-reports-the-hubs-refusal"
    }
    if ($c.ms -gt 60000) {
        Assert ($flat.Contains((Get-Flat 'fohmixer-hub.exe is still in use'))) "install-with-$($c.what)-warns-the-exe-stays-in-use"
        Assert ($flat.Contains((Get-Flat 'the unpacked bundle')) -and $flat.Contains((Get-Flat 'was not removed')) -and
            @($left | Where-Object { $_.StartsWith('.unpack-') }).Count -eq 1) "install-with-$($c.what)-warns-of-the-folder-it-left ($($left -join ', '))"
        # The folder it left, removed now that the copy has ended.
        foreach ($n in $left) { Assert (Remove-FohLeftover -Path (Join-Path (Join-Path $dataH 'app') $n) -What 'test leftover') "install-with-$($c.what)-its-leftover-removed-after" }
    } else {
        Assert ($flat.Contains((Get-Flat 'fohmixer-hub.exe was in use for'))) "install-with-$($c.what)-logs-its-wait"
        Assert ($left.Count -eq 0) "install-with-$($c.what)-leaves-no-unpacked-bundle-or-check-file ($($left -join ', '))"
    }
}

# ---- the files-in-use helpers (FohmixerHeld.ps1), in this process ----
$inUse = Join-Path $heldDir 'in-use'
$heldFile = Join-Path $inUse 'held.bin'
New-Item -ItemType Directory -Force -Path $inUse | Out-Null
[IO.File]::WriteAllText($heldFile, 'held')
Assert (Test-FohFileFree -Path $heldFile) 'file-free-when-no-one-holds-it'
# Read sharing only: another reader may open it, a removal or a move may not.
$holding = [IO.File]::Open($heldFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
try {
    Assert (-not (Test-FohFileFree -Path $heldFile)) 'file-held-while-a-handle-is-open'
    $holders = @(Get-FohFileHolders -Path $heldFile)
    Assert (@($holders | Where-Object { $_.StartsWith("$PID ") }).Count -eq 1) "file-holders-name-this-process ($($holders -join ', '))"
    $w = Wait-FohFileFree -Path $heldFile -TimeoutSeconds 1
    Assert (-not $w.free -and $w.waited -and $w.ms -ge 1000 -and @($w.holders).Count -ge 1) "file-wait-ends-held-after-its-bound ($($w.ms) ms)"
    $warned = $null
    $gone = Remove-FohLeftover -Path $inUse -What 'test folder' -TimeoutSeconds 1 -WarningVariable warned -WarningAction SilentlyContinue
    $said = "$warned"
    Assert (-not $gone -and $said.Contains('the test folder') -and $said.Contains('was not removed') -and
        $said -match ('held\.bin \(by: ([^)]*, )?' + $PID + ' ')) "leftover-not-removed-warns-with-its-holder ($said)"
} finally {
    $holding.Dispose()
}
$w = Wait-FohFileFree -Path $heldFile -TimeoutSeconds 1
Assert ($w.free -and -not $w.waited) 'file-wait-free-once-the-handle-is-closed'
Assert ((Remove-FohLeftover -Path $inUse -What 'test folder') -and -not (Test-Path -LiteralPath $inUse)) 'leftover-removed-once-free'
Assert (Remove-FohLeftover -Path $inUse -What 'test folder') 'leftover-already-gone-is-removed'
# Held by another process for 2 s: the removal fails, waits for the file, and its second try removes it.
New-Item -ItemType Directory -Force -Path $inUse | Out-Null
[IO.File]::WriteAllText($heldFile, 'held')
$holdingMark = Join-Path $heldDir 'holding'
$holdScript = Join-Path $heldDir 'hold.ps1'
[IO.File]::WriteAllText($holdScript, (@(
            'param([string]$File, [string]$Mark)',
            '$ErrorActionPreference = "Stop"',
            '$s = [IO.File]::Open($File, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)',
            '[IO.File]::WriteAllText($Mark, "holding")',
            'Start-Sleep -Seconds 2',
            '$s.Dispose()') -join "`r`n"))
$holder = Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $holdScript, $heldFile, $holdingMark) -WindowStyle Hidden -PassThru
try {
    $deadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path -LiteralPath $holdingMark) -and -not $holder.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 50 }
    Assert (Test-Path -LiteralPath $holdingMark) 'leftover-holder-process-holds-the-file'
    $said = @(Remove-FohLeftover -Path $inUse -What 'test folder' -TimeoutSeconds 10 6>&1)
    $gone = @($said | Where-Object { $_ -is [bool] })
    $text = (@($said | Where-Object { $_ -isnot [bool] } | ForEach-Object { "$_" }) -join ' ')
    Assert ($gone.Count -eq 1 -and $gone[0] -and -not (Test-Path -LiteralPath $inUse) -and $text.Contains('removed at its second try') -and
        $text.Contains('held.bin (by: ')) "leftover-held-briefly-removed-after-a-wait ($text)"
} finally {
    if (-not $holder.WaitForExit(30000)) { throw "the holder process (pid $($holder.Id)) still runs after 30 s" }
}

# ---- the issue's loop (#250): the real hub's config check, its exe free each time it returns ----
# Defender as the runner has it (the evidence of what holds the image).
try {
    $mp = Get-MpComputerStatus
    Write-Host ('held-loop: defender running mode {0}, real-time protection {1}' -f $mp.AMRunningMode, $mp.RealTimeProtectionEnabled)
} catch { Write-Host "held-loop: defender status not read ($($_.Exception.Message))" }
# Each round as an install does it: a fresh copy of the exe in a folder of
# its own, its check, then the folder moved (Install-FohAppDir) and removed.
$loopToml = New-FohHubToml -HttpPort 18481 -BandPort 39181 -MasterPort 39182
$waits = @()
for ($i = 1; $i -le 10; $i++) {
    $loopDir = Join-Path $heldDir ".loop-$i"
    $loopHub = Join-Path $loopDir 'fohmixer-hub.exe'
    New-Item -ItemType Directory -Force -Path $loopDir | Out-Null
    Copy-Item -LiteralPath $HubExe -Destination $loopHub
    $r = Invoke-FohHubCheck -Exe $loopHub -ArgList @('config', 'check') -Text $loopToml -Dir $heldDir
    Assert ($r.code -eq 0 -and $r.image.free) "held-loop-$i-check-passes-and-returns-with-the-exe-free"
    if ($r.image.waited) { $waits += ('{0}: {1} ms (by: {2})' -f $i, $r.image.ms, (Format-FohHolders $r.image.holders)) }
    $moved = Join-Path $heldDir "loop-$i"
    Move-Item -LiteralPath $loopDir -Destination $moved
    $said = @(Remove-FohLeftover -Path $moved -What 'loop folder' 6>&1)
    $gone = @($said | Where-Object { $_ -is [bool] })
    $text = (@($said | Where-Object { $_ -isnot [bool] } | ForEach-Object { "$_" }) -join ' ')
    if ($text) { $waits += ('{0}: removal: {1}' -f $i, $text) }
    Assert ($gone.Count -eq 1 -and $gone[0] -and -not (Test-Path -LiteralPath $moved)) "held-loop-$i-folder-moved-and-removed"
}
$waitText = ''
if ($waits.Count -gt 0) { $waitText = ': ' + ($waits -join '; ') }
Write-Host ('held-loop: {0} waits in 10 rounds (the exe after its check, the folder at its removal){1}' -f $waits.Count, $waitText)
