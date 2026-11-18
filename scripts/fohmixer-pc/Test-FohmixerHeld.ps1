#Requires -Version 5.1
# Self-test of an install whose checking hub's image stays held (#250),
# dot-sourced by Test-Fohmixer.ps1 inside its try (its helpers and
# variables: Assert, Invoke-Ps, Get-InstallArgs, $install, $repo, $base,
# $TrayExe, $deviceDir, $sha). A test hub, a small console exe compiled
# here, is bundled as fohmixer-hub.exe: on any check it starts a copy of
# itself that inherits no handle and keeps running from the unpacked folder
# (its image held) until a release file appears or its time is up, writes
# that copy's pid to a file, then refuses the config. Covered:
# - held past the install's wait: the install reports the hub's refusal,
#   never the cleanup's error, and warns of the folder it left;
# - held for 2 s: the install reports the refusal and leaves no unpacked
#   bundle and no check file;
# - the files-in-use helpers (FohmixerHeld.ps1) in this process: a file this
#   process holds is held, its holders name this process, the wait ends held
#   after its bound, the leftover's removal warns with its holder and never
#   throws; once released, free and removed;
# - the issue's loop: the real hub's config check ten times on a fresh copy,
#   each returning with the exe free; the waits it logged, with the holders,
#   and Defender's state are printed (the evidence of what held it).
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
        StartupInfo si = new StartupInfo();
        si.cb = Marshal.SizeOf(typeof(StartupInfo));
        StringBuilder cmd = new StringBuilder("\"" + self + "\" hold " + ms + " \"" + release + "\"");
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

$release = Join-Path $heldDir 'release'
foreach ($c in @(
        # Past the install's wait (released by the test after the install returns).
        @{ what = 'a-held-hub-image'; ms = 120000; release = $release },
        # 2 s: within the install's wait.
        @{ what = 'a-briefly-held-hub-image'; ms = 2000; release = (Join-Path $heldDir 'never') })) {
    $dataH = Join-Path $heldDir ('data-' + $c.what)
    $pidFile = Join-Path $heldDir ('pid-' + $c.what)
    $env:FOHMIXER_TEST_HOLD_MS = [string]$c.ms
    $env:FOHMIXER_TEST_RELEASE = $c.release
    $env:FOHMIXER_TEST_PIDFILE = $pidFile
    try {
        $res = Invoke-Ps $install (Get-InstallArgs @{ BundleZip = $heldBundle.zip; DataDir = $dataH })
    } finally {
        Remove-Item -LiteralPath 'Env:FOHMIXER_TEST_HOLD_MS', 'Env:FOHMIXER_TEST_RELEASE', 'Env:FOHMIXER_TEST_PIDFILE'
        [IO.File]::WriteAllText($release, 'released')
        Wait-HeldCopy -PidFile $pidFile
    }
    Assert (Test-Path -LiteralPath $pidFile -PathType Leaf) "install-with-$($c.what)-ran-the-test-hub"
    Assert ($res.code -ne 0 -and $res.out.Contains('refused by the test hub (#250)')) "install-with-$($c.what)-reports-the-hubs-refusal"
    $left = @(Get-HeldLeftovers -DataDir $dataH)
    if ($c.ms -gt 60000) {
        Assert ($res.out.Contains('was not removed') -and $res.out.Contains('.unpack-')) "install-with-$($c.what)-warns-of-the-folder-it-left"
        # The folder it left, removed now that the copy has ended.
        foreach ($n in $left) { Remove-Item -LiteralPath (Join-Path (Join-Path $dataH 'app') $n) -Recurse -Force }
    } else {
        Assert ($left.Count -eq 0) "install-with-$($c.what)-leaves-no-unpacked-bundle-or-check-file ($($left -join ', '))"
    }
    if (Test-Path -LiteralPath $release) { Remove-Item -LiteralPath $release }
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
    $gone = Remove-FohLeftover -Path $inUse -What 'test folder' -WarningVariable warned -WarningAction SilentlyContinue
    $said = "$warned"
    Assert (-not $gone -and $said.Contains('the test folder') -and $said.Contains('was not removed') -and $said.Contains("held.bin (by: $PID ")) "leftover-not-removed-warns-with-its-holder ($said)"
} finally {
    $holding.Dispose()
}
$w = Wait-FohFileFree -Path $heldFile -TimeoutSeconds 1
Assert ($w.free -and -not $w.waited) 'file-wait-free-once-the-handle-is-closed'
Assert ((Remove-FohLeftover -Path $inUse -What 'test folder') -and -not (Test-Path -LiteralPath $inUse)) 'leftover-removed-once-free'
Assert (Remove-FohLeftover -Path $inUse -What 'test folder') 'leftover-already-gone-is-removed'

# ---- the issue's loop (#250): the real hub's config check, its exe free each time it returns ----
# Defender as the runner has it (the evidence of what holds the image).
try {
    $mp = Get-MpComputerStatus
    Write-Host ('held-loop: defender running mode {0}, real-time protection {1}' -f $mp.AMRunningMode, $mp.RealTimeProtectionEnabled)
} catch { Write-Host "held-loop: defender status not read ($($_.Exception.Message))" }
$loopDir = Join-Path $heldDir 'loop'
$loopHub = Join-Path $loopDir 'fohmixer-hub.exe'
New-Item -ItemType Directory -Force -Path $loopDir | Out-Null
Copy-Item -LiteralPath $HubExe -Destination $loopHub
$loopToml = New-FohHubToml -HttpPort 18481 -BandPort 39181 -MasterPort 39182
$waits = @()
for ($i = 1; $i -le 10; $i++) {
    $r = Invoke-FohHubCheck -Exe $loopHub -ArgList @('config', 'check') -Text $loopToml -Dir $heldDir
    Assert ($r.code -eq 0 -and $r.image.free -and (Test-FohFileFree -Path $loopHub)) "held-loop-$i-check-passes-and-returns-with-the-exe-free"
    if ($r.image.waited) { $waits += ('{0}: {1} ms (by: {2})' -f $i, $r.image.ms, (Format-FohHolders $r.image.holders)) }
}
$waitText = ''
if ($waits.Count -gt 0) { $waitText = ': ' + ($waits -join '; ') }
Write-Host ('held-loop: {0} of 10 checks waited for the exe{1}' -f $waits.Count, $waitText)
Assert (Remove-FohLeftover -Path $loopDir -What 'loop folder') 'held-loop-folder-removed'
