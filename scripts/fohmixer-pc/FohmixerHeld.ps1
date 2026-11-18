# Files in use (#250), dot-sourced by FohmixerPc.psm1: which processes hold
# a file (the Restart Manager), a bounded wait until a file is free, the
# removal of what an install leaves behind, which never throws (it runs in a
# finally or a catch, where a throw would replace the install's own error),
# and the run of a new hub's check (Test-FohHubToml, Test-FohHubLayout) that
# returns once its exe is free again. Windows PowerShell 5.1.

# How long a check of the new hub waits for its exe to be free again (#250):
# the unpacked folder is moved or removed right after, which a held file fails.
$script:HubImageWaitSeconds = 20

# The Restart Manager (rstrtmgr.dll): the processes that have a file open or
# mapped (a running image), the query installers use to name them.
$script:HeldCode = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class FohmixerHolders {
    [StructLayout(LayoutKind.Sequential)]
    struct UniqueProcess { public int pid; public System.Runtime.InteropServices.ComTypes.FILETIME started; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct ProcessInfo {
        public UniqueProcess process;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string app;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string service;
        public int type;
        public uint status;
        public uint session;
        [MarshalAs(UnmanagedType.Bool)] public bool restartable;
    }
    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmStartSession(out uint session, int flags, StringBuilder key);
    [DllImport("rstrtmgr.dll")]
    static extern int RmEndSession(uint session);
    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmRegisterResources(uint session, uint files, string[] names, uint apps, IntPtr uniqueApps,
        uint services, string[] serviceNames);
    [DllImport("rstrtmgr.dll")]
    static extern int RmGetList(uint session, out uint needed, ref uint count, [In, Out] ProcessInfo[] info, out uint reasons);

    const int MoreData = 234;

    // "<pid> <name>" per process holding the file; one "restart manager
    // error <n>" entry when the query fails.
    public static string[] Of(string path) {
        uint session;
        int rc = RmStartSession(out session, 0, new StringBuilder(64));
        if (rc != 0) { return new string[] { "restart manager error " + rc }; }
        try {
            rc = RmRegisterResources(session, 1, new string[] { path }, 0, IntPtr.Zero, 0, null);
            if (rc != 0) { return new string[] { "restart manager error " + rc }; }
            uint needed;
            uint reasons;
            uint count = 0;
            ProcessInfo[] info = null;
            // The list can grow between the size query and the read: asked again then.
            for (int i = 0; i < 3; i++) {
                rc = RmGetList(session, out needed, ref count, info, out reasons);
                if (rc != MoreData) { break; }
                count = needed;
                info = new ProcessInfo[count];
            }
            if (rc != 0) { return new string[] { "restart manager error " + rc }; }
            List<string> held = new List<string>();
            for (int i = 0; i < count; i++) {
                // The process name, never the Restart Manager's app name: for
                // a program with a window that is its title, which can name a
                // file, a folder or an account.
                string name = null;
                try { name = System.Diagnostics.Process.GetProcessById(info[i].process.pid).ProcessName; } catch (Exception) { name = null; }
                if (string.IsNullOrEmpty(name)) { name = info[i].service; }
                if (string.IsNullOrEmpty(name)) { name = "(name not read)"; }
                held.Add(info[i].process.pid + " " + name);
            }
            return held.ToArray();
        } finally {
            RmEndSession(session);
        }
    }
}
'@

function Get-FohFileHolders {
    # The processes holding $Path open or mapped, as "<pid> <name>" (names
    # only, never their paths: a path can name a Windows account).
    param([Parameter(Mandatory)][string]$Path)
    $type = 'FohmixerHolders' -as [type]
    if ($null -eq $type) {
        Add-Type -TypeDefinition $script:HeldCode -Language CSharp -IgnoreWarnings
        $type = 'FohmixerHolders' -as [type]
    }
    return @($type::Of($Path))
}

function Format-FohHolders {
    # Holders for a log line: "<pid> <name>, ..." or "none named".
    param([AllowEmptyCollection()][AllowNull()][string[]]$Holders = @())
    $named = @($Holders | Where-Object { $_ })
    if ($named.Count -eq 0) { return 'none named' }
    return ($named -join ', ')
}

function Test-FohFileFree {
    # Whether $Path opens for reading and writing with no sharing: false
    # while another handle is open on it (whatever its sharing) or it is
    # mapped as a running image, the two ways a later removal or move fails
    # (a sharing or lock violation). Any other failure (the file gone, read
    # only, refused) is no hold: it is said once and counts as free, so a
    # wait ends at once and the removal or move after it shows its own error.
    param([Parameter(Mandatory)][string]$Path)
    try {
        $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $stream.Dispose()
        return $true
    } catch {
        # A .NET method's exception reaches PowerShell wrapped (MethodInvocationException).
        $e = $_.Exception
        while ($null -ne $e -and -not ($e -is [IO.IOException])) { $e = $e.InnerException }
        # 0x80070020 ERROR_SHARING_VIOLATION, 0x80070021 ERROR_LOCK_VIOLATION
        if ($null -ne $e -and @(-2147024864, -2147024863) -contains $e.HResult) { return $false }
        Write-Host ('fohmixer install: {0} could not be checked for other handles ({1}); taken as free' -f $Path, $_.Exception.Message)
        return $true
    }
}

function Read-FohFileHolders {
    # Get-FohFileHolders for a log line: a lookup that fails (the type not
    # compiled, the Restart Manager refusing) is named in its place, never a
    # throw (the holders only explain a wait, they never decide one).
    param([Parameter(Mandatory)][string]$Path)
    try {
        return @(Get-FohFileHolders -Path $Path)
    } catch {
        return @("holders not read: $($_.Exception.Message)")
    }
}

function Wait-FohFileFree {
    # Waits up to $TimeoutSeconds until $Path is free (Test-FohFileFree),
    # read every 50 ms. Returns free, waited (whether a read found it held),
    # ms (the time until free, or until the wait ended) and holders (read at
    # the first held read, and again when the wait ends with it held).
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][int]$TimeoutSeconds)
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $waited = $false
    $holders = @()
    while ($true) {
        if (Test-FohFileFree -Path $Path) {
            return [pscustomobject]@{ free = $true; waited = $waited; ms = [int]$clock.ElapsedMilliseconds; holders = $holders }
        }
        if (-not $waited) {
            $waited = $true
            $holders = @(Read-FohFileHolders -Path $Path)
        }
        if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            $now = @(Read-FohFileHolders -Path $Path)
            if ($now.Count -gt 0) { $holders = $now }
            return [pscustomobject]@{ free = $false; waited = $true; ms = [int]$clock.ElapsedMilliseconds; holders = $holders }
        }
        Start-Sleep -Milliseconds 50
    }
}

function Get-FohHeldFiles {
    # The files of $Path (a file, or every file in a folder) that are held
    # now (Test-FohFileFree), as their full paths.
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return @() }
    $files = @(Get-Item -LiteralPath $Path -Force)
    if ($files[0].PSIsContainer) { $files = @(Get-ChildItem -LiteralPath $Path -Recurse -File -Force) }
    return @($files | Where-Object { -not (Test-FohFileFree -Path $_.FullName) } | ForEach-Object { $_.FullName })
}

function Remove-FohLeftover {
    # Removes $Path (a file, or a folder with everything in it) when it is
    # there. When the removal fails, it waits for the files that are held then
    # (Wait-FohFileFree, $TimeoutSeconds for them all) and tries once more
    # (also when none was seen held: a short hold can end before the look).
    # A removal that still fails never throws (it may run while an error is
    # in flight, which a throw would replace): one warning names $What, $Path,
    # the reason and the files still held with their holders. Returns whether
    # $Path is gone.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$What, [int]$TimeoutSeconds = 10)
    try {
        if (-not (Test-Path -LiteralPath $Path)) { return $true }
        Remove-Item -LiteralPath $Path -Recurse -Force
        return $true
    } catch {
        $reason = $_.Exception.Message
    }
    $heldText = 'held: none'
    try {
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $waitedFor = @()
        foreach ($f in @(Get-FohHeldFiles -Path $Path)) {
            $left = [Math]::Max(0, $TimeoutSeconds - [int]$clock.Elapsed.TotalSeconds)
            $w = Wait-FohFileFree -Path $f -TimeoutSeconds $left
            $waitedFor += ('{0} (by: {1})' -f (Split-Path -Leaf $f), (Format-FohHolders $w.holders))
        }
        $forText = 'no file seen held'
        if ($waitedFor.Count -gt 0) { $forText = $waitedFor -join '; ' }
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force
            Write-Host ('fohmixer install: the {0} removed at its second try, after a wait of {1} ms ({2})' -f $What, [int]$clock.ElapsedMilliseconds, $forText)
            return $true
        } catch {
            $reason = $_.Exception.Message
        }
        $held = @()
        foreach ($f in @(Get-FohHeldFiles -Path $Path)) {
            $held += ('{0} (by: {1})' -f (Split-Path -Leaf $f), (Format-FohHolders @(Read-FohFileHolders -Path $f)))
        }
        if ($held.Count -gt 0) { $heldText = 'held: ' + ($held -join '; ') }
    } catch {
        $heldText = "its holders not read: $($_.Exception.Message)"
    }
    try {
        Write-Warning ('fohmixer install: the {0} {1} was not removed: {2} ({3}); remove it by hand' -f $What, $Path, $reason, $heldText)
    } catch {
        # A warning that cannot be written (a caller's -WarningAction Stop) is no reason to throw here.
        $null = $_
    }
    return $false
}

function Invoke-FohHubCheck {
    # Runs a check of the new hub: `$Exe <ArgList> <file>`, with $Text (a
    # config) in a file of its own in $Dir (never inside the bundle's tree,
    # which becomes app\<version>). Returns its exit code (code), its answer
    # (said, stderr included) and the wait for its exe (image, Wait-FohFileFree):
    # it returns only once the exe is free again or after
    # $script:HubImageWaitSeconds (#250), since the unpacked folder is moved or
    # removed right after and a held file fails that. A wait is logged with the
    # processes that held the exe; a wait that ends held is a warning (the
    # callers refuse a check that passed with its exe still held: the move
    # of the folder would fail after the stop).
    param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string[]]$ArgList,
          [Parameter(Mandatory)][string]$Text, [Parameter(Mandatory)][string]$Dir)
    $file = Join-Path $Dir ('.check-' + [guid]::NewGuid().ToString('N') + '.toml')
    [IO.File]::WriteAllText($file, $Text, $script:Utf8NoBom)
    try {
        # Continue only around the native call: its stderr is its answer.
        $said = & {
            $ErrorActionPreference = 'Continue'
            (& $Exe @ArgList $file 2>&1 | ForEach-Object { "$_" }) -join ' '
        }
        $code = $LASTEXITCODE
    } finally {
        $null = Remove-FohLeftover -Path $file -What 'check file'
    }
    $name = Split-Path -Leaf $Exe
    $image = Wait-FohFileFree -Path $Exe -TimeoutSeconds $script:HubImageWaitSeconds
    if (-not $image.free) {
        Write-Warning ("fohmixer install: the new hub's {0} is still in use {1} ms after its check (by: {2})" -f $name, $image.ms, (Format-FohHolders $image.holders))
    } elseif ($image.waited) {
        Write-Host ("fohmixer install: the new hub's {0} was in use for {1} ms after its check (by: {2})" -f $name, $image.ms, (Format-FohHolders $image.holders))
    }
    return [pscustomobject]@{ code = $code; said = $said; image = $image }
}

function Assert-FohHubCheckFree {
    # A check that passed while its exe stays held (Invoke-FohHubCheck's
    # image) stops the install here, before any change: after the stop, the
    # move of the unpacked folder into app\<version> would fail on it.
    param([Parameter(Mandatory)]$Check, [Parameter(Mandatory)][string]$Exe)
    if ($Check.image.free) { return }
    throw ("the new hub's {0} stays in use {1} ms after its check (by: {2}); nothing changed, run the install again" -f
        (Split-Path -Leaf $Exe), $Check.image.ms, (Format-FohHolders $Check.image.holders))
}
