# Files in use (#250), dot-sourced by FohmixerPc.psm1: which processes hold
# a file (the Restart Manager), a bounded wait until a file is free, and the
# removal of what an install leaves behind, which never throws: it runs in a
# finally or a catch, where a throw would replace the install's own error.
# Windows PowerShell 5.1.

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
                string name = info[i].app;
                if (string.IsNullOrEmpty(name)) { name = info[i].service; }
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
    param([AllowEmptyCollection()][string[]]$Holders = @())
    if (@($Holders).Count -eq 0) { return 'none named' }
    return (@($Holders) -join ', ')
}

function Test-FohFileFree {
    # Whether $Path opens for reading and writing with no sharing: false
    # while another handle is open on it (whatever its sharing) or it is
    # mapped as a running image, the two ways a later removal or move fails
    # (a sharing or lock violation). Any other failure throws.
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
        throw
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
            $holders = @(Get-FohFileHolders -Path $Path)
        }
        if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
            $now = @(Get-FohFileHolders -Path $Path)
            if ($now.Count -gt 0) { $holders = $now }
            return [pscustomobject]@{ free = $false; waited = $true; ms = [int]$clock.ElapsedMilliseconds; holders = $holders }
        }
        Start-Sleep -Milliseconds 50
    }
}

function Remove-FohLeftover {
    # Removes $Path (a file, or a folder with everything in it) when it is
    # there. A removal that fails never throws (it may run while an error is
    # in flight, which a throw would replace): one warning names $What, $Path,
    # the reason and the processes holding its files. Returns whether $Path is gone.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$What)
    if (-not (Test-Path -LiteralPath $Path)) { return $true }
    try {
        Remove-Item -LiteralPath $Path -Recurse -Force
        return $true
    } catch {
        $reason = $_.Exception.Message
    }
    $held = @()
    try {
        $files = @(Get-Item -LiteralPath $Path -Force)
        if ($files[0].PSIsContainer) { $files = @(Get-ChildItem -LiteralPath $Path -Recurse -File -Force) }
        foreach ($f in $files) {
            if (-not (Test-FohFileFree -Path $f.FullName)) { $held += ('{0} (by: {1})' -f $f.Name, (Format-FohHolders (Get-FohFileHolders -Path $f.FullName))) }
        }
        $heldText = 'held: none'
        if ($held.Count -gt 0) { $heldText = 'held: ' + ($held -join '; ') }
    } catch {
        $heldText = "its holders not read: $($_.Exception.Message)"
    }
    Write-Warning ('fohmixer install: the {0} {1} was not removed: {2} ({3}); remove it by hand' -f $What, $Path, $reason, $heldText)
    return $false
}
