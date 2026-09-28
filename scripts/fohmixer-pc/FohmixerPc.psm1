#Requires -Version 5.1
# fohmixer S5 (design note docs/superpowers/specs/2026-09-28-s5-deploy-design.md,
# sections 2 and 3): the Ableton PC install, shipped in every bundle next to
# Install-Fohmixer.ps1, and the bundle build that the CI bundle job and the
# self-test (Test-Fohmixer.ps1) share. Windows PowerShell 5.1.
# - Nothing is ended by force (spec I7): the hub is asked to stop with
#   Ctrl-Break, its graceful stop, and waited for; a hub that does not stop is
#   reported, never ended. Its tasks may never be ended hard either.
# - Live's preferences and a running Live are never touched: only the FohMixer
#   folder in each user's User Library is written, and Live reads it at its
#   next start.
# - No site value lives here (spec 5.2): user names, folders and ports come as
#   parameters.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:Utf8NoBom = New-Object System.Text.UTF8Encoding $false
$script:HubExe = 'fohmixer-hub.exe'
$script:ScriptName = 'FohMixer'
$script:TaskPath = '\fohmixer\'
$script:HubTask = 'fohmixer-hub'
$script:StopTask = 'fohmixer-hub-stop'
# What a bundle holds besides SHA256SUMS (design note section 2).
$script:BundleFiles = @('VERSION', 'fohmixer-hub.exe', 'Install-Fohmixer.ps1', 'FohmixerPc.psm1',
    'Start-FohmixerHub.ps1', 'Stop-FohmixerHub.ps1',
    'FohMixer/__init__.py', 'FohMixer/Config.py', 'FohMixer/version.py')
# A SemVer version that is also a safe folder name (no trailing dot, no "..").
$script:SemVer = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?\z'

# Ctrl-Break through another process's console (the hub's graceful stop, as in
# iemmixer's iem-win console.rs): detach from this process's console, attach to
# the hub's, ignore the event here, send it to every process on that console,
# detach. A console is reachable only from a process in the same session.
$script:ConsoleCode = @'
using System;
using System.Runtime.InteropServices;

public static class FohmixerConsole
{
    public delegate bool CtrlHandler(uint ctrlType);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FreeConsole();

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AttachConsole(uint processId);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetConsoleCtrlHandler(CtrlHandler handler, bool add);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GenerateConsoleCtrlEvent(uint ctrlEvent, uint processGroupId);

    const uint CtrlBreakEvent = 1;

    // Referenced for the life of the process: the event arrives on a thread of its own.
    static readonly CtrlHandler Ignore = IgnoreEvent;
    static bool ignoring;

    static bool IgnoreEvent(uint ctrlType)
    {
        return true;
    }

    // Sends Ctrl-Break to every process on processId's console (this process
    // ignores it) and detaches again: 0, or the Win32 error of the failed step.
    public static int Break(uint processId)
    {
        if (!ignoring)
        {
            if (!SetConsoleCtrlHandler(Ignore, true)) return Marshal.GetLastWin32Error();
            ignoring = true;
        }
        FreeConsole();
        if (!AttachConsole(processId)) return Marshal.GetLastWin32Error();
        int result = 0;
        if (!GenerateConsoleCtrlEvent(CtrlBreakEvent, 0)) result = Marshal.GetLastWin32Error();
        FreeConsole();
        return result;
    }
}
'@

# ---- small helpers ----

function Resolve-FohPath {
    # A full path; a relative one is taken from PowerShell's current folder (the
    # .NET calls below would take the process's).
    param([Parameter(Mandatory)][string]$Path)
    return $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path).TrimEnd('\')
}

function Test-FohVersion {
    # A SemVer version usable as the app\<version> folder name.
    param([AllowEmptyString()][string]$Version)
    return ($Version -cmatch $script:SemVer)
}

function Format-FohArg {
    # One command-line argument in double quotes. A value never holds a quote; a
    # trailing backslash would escape the closing one, so it is dropped.
    param([Parameter(Mandatory)][string]$Value)
    if ($Value.Contains('"')) { throw "argument refused (it holds a double quote): $Value" }
    return '"' + $Value.TrimEnd('\') + '"'
}

function Get-FohLeafName {
    # An account name without its domain or computer part.
    param([AllowEmptyString()][string]$Name)
    return $Name.Substring($Name.LastIndexOf('\') + 1)
}

function Get-FohSha256 {
    param([Parameter(Mandatory)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-FohText {
    # UTF-8 text without a BOM, written to a temp file and moved over the target.
    # Returns $false (nothing written) when the file already holds exactly this text.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][AllowEmptyString()][string]$Text)
    if ((Test-Path -LiteralPath $Path -PathType Leaf) -and ([IO.File]::ReadAllText($Path) -ceq $Text)) { return $false }
    $tmp = $Path + '.tmp'
    [IO.File]::WriteAllText($tmp, $Text, $script:Utf8NoBom)
    Move-Item -LiteralPath $tmp -Destination $Path -Force
    return $true
}

function Get-FohRelativeFiles {
    # Every file under $Root: its path relative to $Root ('/' separated) and its full path.
    param([Parameter(Mandatory)][string]$Root)
    $base = (Get-Item -LiteralPath $Root).FullName.TrimEnd('\')
    $out = @()
    foreach ($f in @(Get-ChildItem -LiteralPath $base -Recurse -File -Force)) {
        $out += [pscustomobject]@{ rel = $f.FullName.Substring($base.Length + 1).Replace('\', '/'); full = $f.FullName }
    }
    return ($out | Sort-Object -Property rel)
}

function Test-FohSkipped {
    # Whether a FohMixer file is Live's or Python's, never the bundle's: logs\
    # (the script's log folder), __pycache__\ and compiled files.
    param([Parameter(Mandatory)][string]$Rel)
    return (($Rel -match '(^|/)(__pycache__|logs)/') -or ($Rel -like '*.pyc'))
}

function Get-FohScriptVersion {
    # VERSION = "..." of a FohMixer version.py.
    param([Parameter(Mandatory)][string]$Path)
    $m = [regex]::Match([IO.File]::ReadAllText($Path), '(?m)^VERSION[ \t]*=[ \t]*"([^"\r\n]+)"')
    if (-not $m.Success) { throw "no VERSION = `"...`" in $Path" }
    return $m.Groups[1].Value
}

function Get-FohWorkspaceVersion {
    # [workspace.package].version of Cargo.toml, read from its own table (never
    # the first version line of the file).
    param([Parameter(Mandatory)][string]$CargoToml)
    $inPackage = $false
    foreach ($line in [IO.File]::ReadAllLines((Resolve-FohPath $CargoToml))) {
        if ($line -match '^\s*\[') { $inPackage = ($line.Trim() -ceq '[workspace.package]'); continue }
        if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            $v = $Matches[1]
            if (-not (Test-FohVersion $v)) { throw "workspace version refused: $v" }
            return $v
        }
    }
    throw "no [workspace.package] version in $CargoToml"
}

# ---- the bundle (design note section 2) ----

function New-FohBundleZip {
    # Writes <Stage>\SHA256SUMS ("<sha256>  <path>" for every other file) and
    # zips the folder's content (relative entries) to $Zip.
    param([Parameter(Mandatory)][string]$Stage, [Parameter(Mandatory)][string]$Zip)
    $Stage = Resolve-FohPath $Stage
    $Zip = Resolve-FohPath $Zip
    $version = ([IO.File]::ReadAllText((Join-Path $Stage 'VERSION'))).Trim()
    if (-not (Test-FohVersion $version)) { throw "bundle VERSION refused: $version" }
    if (Test-Path -LiteralPath $Zip) { throw "the zip $Zip exists already" }
    $sums = Join-Path $Stage 'SHA256SUMS'
    if (Test-Path -LiteralPath $sums) { Remove-Item -LiteralPath $sums }
    $lines = @()
    foreach ($f in @(Get-FohRelativeFiles -Root $Stage)) { $lines += ('{0}  {1}' -f (Get-FohSha256 $f.full), $f.rel) }
    [IO.File]::WriteAllText($sums, (($lines -join "`n") + "`n"), $script:Utf8NoBom)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($Stage, $Zip)
}

function New-FohBundle {
    # The Windows release bundle from a repo checkout and a built hub:
    # fohmixer-hub.exe, FohMixer\ (without logs, caches or compiled files), the
    # PC scripts, VERSION (the workspace version) and SHA256SUMS, zipped to
    # <OutDir>\fohmixer-windows-<version>-<Sha>.zip. Returns name, version, stage and zip.
    param([Parameter(Mandatory)][string]$RepoRoot, [Parameter(Mandatory)][string]$HubExe,
          [Parameter(Mandatory)][string]$OutDir, [Parameter(Mandatory)][string]$Sha)
    if ($Sha -cnotmatch '^[0-9a-f]{7,40}\z') { throw "commit refused: $Sha" }
    $RepoRoot = Resolve-FohPath $RepoRoot
    $HubExe = Resolve-FohPath $HubExe
    $OutDir = Resolve-FohPath $OutDir
    if (-not (Test-Path -LiteralPath $HubExe -PathType Leaf)) { throw "the hub $HubExe does not exist (build it first)" }
    $version = Get-FohWorkspaceVersion -CargoToml (Join-Path $RepoRoot 'Cargo.toml')
    $scriptDir = Join-Path $RepoRoot 'live-script\FohMixer'
    $scriptVersion = Get-FohScriptVersion -Path (Join-Path $scriptDir 'version.py')
    if ($scriptVersion -cne $version) { throw "FohMixer\version.py says $scriptVersion, Cargo.toml $version" }
    $name = "fohmixer-windows-$version-$Sha"
    $stage = Join-Path $OutDir $name
    if (Test-Path -LiteralPath $stage) { throw "the stage folder $stage exists already" }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item -LiteralPath $HubExe -Destination (Join-Path $stage $script:HubExe)
    foreach ($f in @(Get-FohRelativeFiles -Root $scriptDir)) {
        if (Test-FohSkipped $f.rel) { continue }
        $target = Join-Path (Join-Path $stage $script:ScriptName) $f.rel.Replace('/', '\')
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        Copy-Item -LiteralPath $f.full -Destination $target
    }
    # -Include is ignored next to -LiteralPath in Windows PowerShell 5.1: filter by extension.
    foreach ($f in @(Get-ChildItem -LiteralPath (Join-Path $RepoRoot 'scripts\fohmixer-pc') -File |
            Where-Object { @('.ps1', '.psm1') -contains $_.Extension })) {
        Copy-Item -LiteralPath $f.FullName -Destination $stage
    }
    [IO.File]::WriteAllText((Join-Path $stage 'VERSION'), ($version + "`n"), $script:Utf8NoBom)
    $zip = Join-Path $OutDir ($name + '.zip')
    New-FohBundleZip -Stage $stage -Zip $zip
    return [pscustomobject]@{ name = $name; version = $version; stage = $stage; zip = $zip }
}

function Test-FohBundle {
    # An unpacked bundle checked against its SHA256SUMS (every file listed with
    # its right sum, nothing unlisted, nothing missing) and the design's contents;
    # returns its version, throws naming the first problem.
    param([Parameter(Mandatory)][string]$Root)
    $sumsPath = Join-Path $Root 'SHA256SUMS'
    if (-not (Test-Path -LiteralPath $sumsPath -PathType Leaf)) { throw 'bundle refused: it has no SHA256SUMS' }
    $want = @{}
    foreach ($line in [IO.File]::ReadAllLines($sumsPath)) {
        if ($line -eq '') { continue }
        if ($line -cnotmatch '^([0-9a-f]{64})  (\S.*)\z') { throw "bundle refused: SHA256SUMS line [$line]" }
        $want[$Matches[2]] = $Matches[1]
    }
    $seen = @{}
    foreach ($f in @(Get-FohRelativeFiles -Root $Root)) {
        if ($f.rel -ceq 'SHA256SUMS') { continue }
        if (-not $want.ContainsKey($f.rel)) { throw "bundle refused: $($f.rel) is not in SHA256SUMS" }
        $sum = Get-FohSha256 $f.full
        if ($sum -cne $want[$f.rel]) { throw "bundle refused: $($f.rel): sha256 $sum, SHA256SUMS says $($want[$f.rel])" }
        $seen[$f.rel] = $true
    }
    foreach ($k in $want.Keys) {
        if (-not $seen.ContainsKey($k)) { throw "bundle refused: $k is in SHA256SUMS but not in the bundle" }
    }
    foreach ($n in $script:BundleFiles) {
        if (-not $seen.ContainsKey($n)) { throw "bundle refused: it has no $n" }
    }
    $version = ([IO.File]::ReadAllText((Join-Path $Root 'VERSION'))).Trim()
    if (-not (Test-FohVersion $version)) { throw "bundle refused: VERSION [$version]" }
    $scriptVersion = Get-FohScriptVersion -Path (Join-Path $Root 'FohMixer\version.py')
    if ($scriptVersion -cne $version) { throw "bundle refused: FohMixer\version.py says $scriptVersion, VERSION $version" }
    return $version
}

function Test-FohSameTree {
    # Whether two folders hold the same files with the same content.
    param([Parameter(Mandatory)][string]$A, [Parameter(Mandatory)][string]$B)
    $x = @(Get-FohRelativeFiles -Root $A)
    $y = @(Get-FohRelativeFiles -Root $B)
    if ($x.Count -ne $y.Count) { return $false }
    for ($i = 0; $i -lt $x.Count; $i++) {
        if ($x[$i].rel -cne $y[$i].rel) { return $false }
        if ((Get-FohSha256 $x[$i].full) -cne (Get-FohSha256 $y[$i].full)) { return $false }
    }
    return $true
}

function Expand-FohBundle {
    # Unpacks the zip into <DataDir>\app\.unpack-<id> and checks it
    # (Test-FohBundle). Nothing else changes; the caller moves it into place
    # with Install-FohAppDir or removes it. Returns the folder and the version.
    param([Parameter(Mandatory)][string]$BundleZip, [Parameter(Mandatory)][string]$DataDir)
    $app = Join-Path $DataDir 'app'
    New-Item -ItemType Directory -Force -Path $app | Out-Null
    $tmp = Join-Path $app ('.unpack-' + [guid]::NewGuid().ToString('N'))
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    try {
        [IO.Compression.ZipFile]::ExtractToDirectory($BundleZip, $tmp)
        $version = Test-FohBundle -Root $tmp
    } catch {
        if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Recurse -Force }
        throw
    }
    return [pscustomobject]@{ dir = $tmp; version = $version }
}

function Install-FohAppDir {
    # Design note section 3 step 2: moves an unpacked bundle to
    # <DataDir>\app\<version> and points <DataDir>\app\current.txt at it. An
    # installed folder with the same files stays as it is (the unpacked copy is
    # removed); a different one is replaced, so no hub may run from it. Earlier
    # versions stay for a manual rollback. Returns the folder and whether
    # anything changed.
    param([Parameter(Mandatory)]$Unpacked, [Parameter(Mandatory)][string]$DataDir)
    $app = Join-Path $DataDir 'app'
    $dest = Join-Path $app $Unpacked.version
    $changed = $true
    if (Test-Path -LiteralPath $dest) {
        if (Test-FohSameTree -A $Unpacked.dir -B $dest) {
            $changed = $false
            Remove-Item -LiteralPath $Unpacked.dir -Recurse -Force
        } else {
            Remove-Item -LiteralPath $dest -Recurse -Force
        }
    }
    if ($changed) { Move-Item -LiteralPath $Unpacked.dir -Destination $dest }
    $currentChanged = Write-FohText -Path (Join-Path $app 'current.txt') -Text ($dest + "`r`n")
    return [pscustomobject]@{ dir = $dest; changed = ($changed -or $currentChanged) }
}

# ---- configuration, layout, the FohMixer copies (design note section 3 steps 3-5) ----

function New-FohHubToml {
    # <DataDir>\fohmixer-hub.toml as the hub's config.rs reads it: the HTTP
    # port, the two instances and the layout file (relative to the data folder).
    # There is no data_dir key: the hub's data folder is FOHMIXER_DATA.
    param([Parameter(Mandatory)][int]$HttpPort, [Parameter(Mandatory)][int]$BandPort, [Parameter(Mandatory)][int]$MasterPort)
    $lines = @(
        '# fohmixer-hub configuration, written by Install-Fohmixer.ps1: run the install again to change it.',
        ('http_port = {0}' -f $HttpPort),
        'layout = "layout.json"',
        '',
        '[[instances]]',
        'name = "band"',
        ('port = {0}' -f $BandPort),
        '',
        '[[instances]]',
        'name = "master"',
        ('port = {0}' -f $MasterPort))
    return (($lines -join "`r`n") + "`r`n")
}

function Set-FohConfigText {
    # Config.py's text with INSTANCE and PORT set. Exactly one assignment of each
    # must exist; everything else is kept, line endings included.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text, [Parameter(Mandatory)][string]$Instance,
          [Parameter(Mandatory)][int]$Port)
    if ($Instance -cnotmatch '^[a-z][a-z0-9_-]*\z') { throw "instance name refused: $Instance" }
    $rules = @(
        @{ key = 'INSTANCE'; value = ('INSTANCE = "{0}"' -f $Instance) },
        @{ key = 'PORT'; value = ('PORT = {0}' -f $Port) })
    foreach ($r in $rules) {
        $pattern = '(?m)^' + $r.key + '[ \t]*=[^\r\n]*'
        $n = [regex]::Matches($Text, $pattern).Count
        if ($n -ne 1) { throw "Config.py has $n assignments of $($r.key), not one" }
        $Text = [regex]::Replace($Text, $pattern, $r.value)
    }
    return $Text
}

function Test-FohLayoutFile {
    # The import's layout file exists and is JSON (the hub checks the rest).
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "layout file not found: $Path" }
    try {
        $null = ([IO.File]::ReadAllText($Path) | ConvertFrom-Json)
    } catch {
        throw "layout file $Path is not JSON: $($_.Exception.Message)"
    }
}

function Install-FohLayout {
    # Design note section 3 step 4: <DataDir>\layout.json becomes the import's
    # layout file, written only when it differs. The hub keeps the backups of
    # every layout it accepts.
    param([Parameter(Mandatory)][string]$Layout, [Parameter(Mandatory)][string]$DataDir)
    $target = Join-Path $DataDir 'layout.json'
    if ((Test-Path -LiteralPath $target -PathType Leaf) -and ((Get-FohSha256 $target) -ceq (Get-FohSha256 $Layout))) { return $false }
    Copy-Item -LiteralPath $Layout -Destination ($target + '.tmp') -Force
    Move-Item -LiteralPath ($target + '.tmp') -Destination $target -Force
    return $true
}

function Install-FohUserScript {
    # Design note section 3 step 5, one Live user: <UserLibrary>\Remote
    # Scripts\FohMixer. The User Library must exist (Live made it; a missing one
    # means Live keeps it elsewhere, and nothing is guessed). The copy is
    # replaced only when its version.py differs from the bundle's: every bundle
    # file is written, version.py last (an interrupted copy is redone next
    # time), and files the bundle no longer has are removed; Live's logs\ and
    # Python's __pycache__\ stay. Config.py always carries this user's INSTANCE
    # and PORT and is written only when that changes it. Live reads the folder at
    # its start only, so a running Live is not affected. Returns what changed.
    param([Parameter(Mandatory)][string]$Source, [Parameter(Mandatory)][string]$UserLibrary,
          [Parameter(Mandatory)][string]$Instance, [Parameter(Mandatory)][int]$Port)
    if (-not (Test-Path -LiteralPath $UserLibrary -PathType Container)) { throw "Live's User Library not found: $UserLibrary" }
    $dest = Join-Path (Join-Path $UserLibrary 'Remote Scripts') $script:ScriptName
    $sourceVersion = [IO.File]::ReadAllText((Join-Path $Source 'version.py'))
    $destVersion = Join-Path $dest 'version.py'
    $replace = $true
    if (Test-Path -LiteralPath $destVersion -PathType Leaf) { $replace = ([IO.File]::ReadAllText($destVersion) -cne $sourceVersion) }
    $removed = @()
    if ($replace) {
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        $files = @(Get-FohRelativeFiles -Root $Source | Where-Object { -not (Test-FohSkipped $_.rel) })
        $keep = @{}
        foreach ($f in $files) {
            $keep[$f.rel] = $true
            if ($f.rel -ceq 'version.py' -or $f.rel -ceq 'Config.py') { continue }
            $target = Join-Path $dest $f.rel.Replace('/', '\')
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
            Copy-Item -LiteralPath $f.full -Destination $target -Force
        }
        foreach ($f in @(Get-FohRelativeFiles -Root $dest)) {
            if ((Test-FohSkipped $f.rel) -or $keep.ContainsKey($f.rel)) { continue }
            Remove-Item -LiteralPath $f.full
            $removed += $f.rel
        }
    }
    $config = Join-Path $dest 'Config.py'
    $base = Join-Path $Source 'Config.py'
    if (-not $replace -and (Test-Path -LiteralPath $config -PathType Leaf)) { $base = $config }
    $configText = Set-FohConfigText -Text ([IO.File]::ReadAllText($base)) -Instance $Instance -Port $Port
    $configWritten = Write-FohText -Path $config -Text $configText
    if ($replace) { Copy-Item -LiteralPath (Join-Path $Source 'version.py') -Destination $destVersion -Force }
    return [pscustomobject]@{ path = $dest; replaced = $replace; config_written = $configWritten; removed = $removed }
}

# ---- the data folder's DACL ----

function Set-FohDataDirAcl {
    # A protected DACL on the data folder, inherited by everything below:
    # SYSTEM and Administrators full control, the hub's user Modify (the hub
    # writes secrets\, the layout backups and its state there; no other user
    # reads its secrets). Read back; throws on a difference.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$User)
    $userSid = (New-Object Security.Principal.NTAccount $User).Translate([Security.Principal.SecurityIdentifier])
    $full = [Security.AccessControl.FileSystemRights]::FullControl
    $modify = [Security.AccessControl.FileSystemRights]::Modify
    $want = @(
        @{ sid = (New-Object Security.Principal.SecurityIdentifier 'S-1-5-18'); rights = $full },
        @{ sid = (New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-544'); rights = $full },
        @{ sid = $userSid; rights = $modify })
    $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($w in $want) {
        $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($w.sid, $w.rights, $inherit,
            [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow)))
    }
    [IO.Directory]::SetAccessControl($Path, $acl)
    $problems = @(Test-FohDataDirAcl -Path $Path -User $User)
    if ($problems.Count -gt 0) { throw ("data folder DACL read-back: " + ($problems -join '; ')) }
}

function Test-FohDataDirAcl {
    # The data folder's DACL read back against Set-FohDataDirAcl; returns the differences.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$User)
    $userSid = (New-Object Security.Principal.NTAccount $User).Translate([Security.Principal.SecurityIdentifier]).Value
    $sync = [int][Security.AccessControl.FileSystemRights]::Synchronize
    $want = @{}
    $want['S-1-5-18'] = [int][Security.AccessControl.FileSystemRights]::FullControl
    $want['S-1-5-32-544'] = [int][Security.AccessControl.FileSystemRights]::FullControl
    $want[$userSid] = [int][Security.AccessControl.FileSystemRights]::Modify
    $acl = [IO.Directory]::GetAccessControl($Path)
    $bad = @()
    if (-not $acl.AreAccessRulesProtected) { $bad += 'it inherits from its parent' }
    $seen = @{}
    foreach ($r in @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))) {
        $sid = $r.IdentityReference.Value
        if ($r.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) { $bad += "a deny rule for $sid"; continue }
        if (-not $want.ContainsKey($sid)) { $bad += "a rule for $sid"; continue }
        if (([int]$r.FileSystemRights -bor $sync) -ne ($want[$sid] -bor $sync)) { $bad += "$sid has $($r.FileSystemRights)" }
        if ($r.InheritanceFlags -ne [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit') { $bad += "$sid is not inherited below" }
        if ($seen.ContainsKey($sid)) { $bad += "$sid has two rules" }
        $seen[$sid] = $true
    }
    foreach ($sid in $want.Keys) { if (-not $seen.ContainsKey($sid)) { $bad += "no rule for $sid" } }
    return $bad
}

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
    # Ctrl-Break to a hub through its own console: the hub stops gracefully on
    # it. This detaches the CALLING process from its console for good, so only a
    # process of its own calls it (Stop-FohmixerHub.ps1). Returns 0 or the Win32 error.
    param([Parameter(Mandatory)][int]$ProcessId)
    if ($ProcessId -le 0) { throw "pid $ProcessId refused" }
    $type = 'FohmixerConsole' -as [type]
    if ($null -eq $type) {
        Add-Type -TypeDefinition $script:ConsoleCode -Language CSharp
        $type = 'FohmixerConsole' -as [type]
    }
    return [int]$type::Break([uint32]$ProcessId)
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
        if ((Get-Date) -ge $deadline) { throw "the task $TaskPath$Name still runs $TimeoutSeconds s after its hub stopped" }
        Start-Sleep -Milliseconds 250
    }
}

function Stop-FohHub {
    # Design note section 3 step 1: asks every hub under <DataDir>\app to stop
    # and waits up to $TimeoutSeconds. The request is Ctrl-Break, the hub's
    # graceful stop, sent by the stop task from the hub user's own session (a
    # console is reachable only from there). Task Scheduler's End is not used:
    # for a process without a window it has no graceful request, only a hard end,
    # which our tasks forbid (spec I7). A hub still running after the wait is
    # reported, never ended.
    param([Parameter(Mandatory)][string]$DataDir, [int]$TimeoutSeconds = 10, [string]$TaskPath = $script:TaskPath)
    $running = @(Get-FohHubProcess -DataDir $DataDir)
    if ($running.Count -eq 0) { return 'no hub was running' }
    $pids = ($running | ForEach-Object { $_.pid }) -join ', '
    if ($null -eq (Get-FohTask -Name $script:StopTask -TaskPath $TaskPath)) {
        throw ("a hub runs (pid $pids) but the task $TaskPath$($script:StopTask) is missing: stop the hub with " +
            'Ctrl-Break or Ctrl-C in its console, then run the install again')
    }
    Start-ScheduledTask -TaskPath $TaskPath -TaskName $script:StopTask
    $left = @(Wait-FohHubExit -DataDir $DataDir -TimeoutSeconds $TimeoutSeconds)
    if ($left.Count -gt 0) {
        $info = Get-ScheduledTaskInfo -TaskPath $TaskPath -TaskName $script:StopTask
        $still = ($left | ForEach-Object { $_.pid }) -join ', '
        $msg = 'the hub (pid {0}) did not stop within {1} s of Ctrl-Break (the stop task''s last result 0x{2:x8}; see {3}\logs\hub-stop.log). ' -f
            $still, $TimeoutSeconds, [int64]$info.LastTaskResult, $DataDir
        throw ($msg + 'It is not ended by force (spec I7): stop it in its session, then run the install again')
    }
    Wait-FohTaskIdle -Name $script:HubTask -TaskPath $TaskPath
    return "stopped the hub (pid $pids)"
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

# ---- the scheduled tasks (design note section 3 step 6; spec D9) ----

function Get-FohTaskActionArgs {
    # The tasks' powershell.exe arguments for one of the bundle's scripts.
    param([Parameter(Mandatory)][string]$Script, [Parameter(Mandatory)][string]$DataDir)
    return ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File ' + (Format-FohArg $Script) +
        ' -DataDir ' + (Format-FohArg $DataDir))
}

function Get-FohTaskProblems {
    # What one of our tasks must read back as (Interactive for the user,
    # Limited, no time limit, IgnoreNew, no battery or idle stop, never ended
    # hard, normal priority; the hub task restarts on failure 3 x 1 min and
    # starts at the user's logon); returns the differences.
    param([Parameter(Mandatory)]$Task, [Parameter(Mandatory)][string]$User, [Parameter(Mandatory)][string]$Execute,
          [Parameter(Mandatory)][string]$Arguments, [Parameter(Mandatory)][string]$WorkDir, [switch]$Hub)
    $bad = @()
    $p = $Task.Principal
    $s = $Task.Settings
    if ("$($p.LogonType)" -ne 'Interactive') { $bad += "logon type $($p.LogonType)" }
    if ("$($p.RunLevel)" -ne 'Limited') { $bad += "run level $($p.RunLevel)" }
    if ((Get-FohLeafName ([string]$p.UserId)) -ne (Get-FohLeafName $User)) { $bad += "user $($p.UserId)" }
    if ($s.ExecutionTimeLimit -ne 'PT0S') { $bad += "time limit $($s.ExecutionTimeLimit)" }
    if ("$($s.MultipleInstances)" -ne 'IgnoreNew') { $bad += "instances $($s.MultipleInstances)" }
    if ($s.DisallowStartIfOnBatteries -or $s.StopIfGoingOnBatteries) { $bad += 'stops on batteries' }
    if ($s.IdleSettings.StopOnIdleEnd) { $bad += 'stops at idle end' }
    if ($s.AllowHardTerminate) { $bad += 'may be ended hard' }
    if ($s.Priority -ne 4) { $bad += "priority $($s.Priority)" }
    $restart = '{0}x{1}' -f $s.RestartCount, $s.RestartInterval
    $wantRestart = '0x'
    if ($Hub) { $wantRestart = '3xPT1M' }
    if ($restart -ne $wantRestart) { $bad += "restart $restart" }
    $actions = @($Task.Actions)
    if ($actions.Count -ne 1 -or $actions[0].Execute -ne $Execute -or $actions[0].Arguments -ne $Arguments -or
        $actions[0].WorkingDirectory -ne $WorkDir) {
        $bad += ('action ' + (($actions | ForEach-Object { "$($_.Execute) $($_.Arguments) in $($_.WorkingDirectory)" }) -join ' | '))
    }
    $triggers = @($Task.Triggers | Where-Object { $null -ne $_ })
    if ($Hub) {
        if ($triggers.Count -ne 1 -or $triggers[0].CimClass.CimClassName -ne 'MSFT_TaskLogonTrigger' -or
            (Get-FohLeafName ([string]$triggers[0].UserId)) -ne (Get-FohLeafName $User)) {
            $bad += "triggers: not one logon trigger for $User"
        }
    } elseif ($triggers.Count -ne 0) {
        $bad += "$($triggers.Count) triggers"
    }
    return $bad
}

function Register-FohHubTasks {
    # Registers (or updates) our two tasks for the hub's user and reads them
    # back: fohmixer-hub runs this bundle's Start-FohmixerHub.ps1 at the user's
    # logon; fohmixer-hub-stop (no trigger) runs its Stop-FohmixerHub.ps1 on
    # demand, in the user's session. Neither is started here. Throws on a
    # read-back difference.
    param([Parameter(Mandatory)][string]$AppDir, [Parameter(Mandatory)][string]$DataDir,
          [Parameter(Mandatory)][string]$User, [string]$TaskPath = $script:TaskPath)
    $ps = Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
    $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Limited
    $common = @{
        ExecutionTimeLimit = [TimeSpan]::Zero; MultipleInstances = 'IgnoreNew'; AllowStartIfOnBatteries = $true
        DontStopIfGoingOnBatteries = $true; DontStopOnIdleEnd = $true; DisallowHardTerminate = $true; Priority = 4
    }
    $hubSettings = New-ScheduledTaskSettingsSet @common -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
    $stopSettings = New-ScheduledTaskSettingsSet @common
    $specs = @(
        @{ name = $script:HubTask; script = (Join-Path $AppDir 'Start-FohmixerHub.ps1'); settings = $hubSettings; hub = $true
           description = 'fohmixer: the hub (S5). Stop it only through fohmixer-hub-stop (Ctrl-Break).' },
        @{ name = $script:StopTask; script = (Join-Path $AppDir 'Stop-FohmixerHub.ps1'); settings = $stopSettings; hub = $false
           description = 'fohmixer: asks the running hub to stop (Ctrl-Break in its session).' })
    $problems = @()
    foreach ($spec in $specs) {
        $taskArgs = Get-FohTaskActionArgs -Script $spec.script -DataDir $DataDir
        $action = New-ScheduledTaskAction -Execute $ps -Argument $taskArgs -WorkingDirectory $DataDir
        $register = @{
            TaskPath = $TaskPath; TaskName = $spec.name; Action = $action; Principal = $principal
            Settings = $spec.settings; Description = $spec.description; Force = $true
        }
        if ($spec.hub) { $register['Trigger'] = New-ScheduledTaskTrigger -AtLogOn -User $User }
        Register-ScheduledTask @register | Out-Null
        $task = Get-ScheduledTask -TaskPath $TaskPath -TaskName $spec.name
        foreach ($b in @(Get-FohTaskProblems -Task $task -User $User -Execute $ps -Arguments $taskArgs -WorkDir $DataDir -Hub:$spec.hub)) {
            $problems += ('{0}: {1}' -f $spec.name, $b)
        }
    }
    if ($problems.Count -gt 0) { throw ('task read-back: ' + ($problems -join '; ')) }
}

# ---- the install (design note section 3) ----

function Test-FohUserName {
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Name)
    if ($Name -cnotmatch '^[^"/\\\[\]:;|=,+*?<>]+\z' -or $Name.Trim() -cne $Name -or $Name.EndsWith('.')) {
        throw "user name refused: [$Name] (an account name of this PC, without a domain part)"
    }
}

function Invoke-FohInstall {
    # Design note section 3, in order; every input is checked before anything
    # changes. -NoTask (the self-test) leaves out what needs the real accounts:
    # the stop, the DACL, the tasks, the start and the readiness poll.
    param(
        [Parameter(Mandatory)][string]$BundleZip,
        [Parameter(Mandatory)][string]$BandUser,
        [Parameter(Mandatory)][string]$MasterUser,
        [Parameter(Mandatory)][string]$DataDir,
        [Parameter(Mandatory)][int]$HttpPort,
        [Parameter(Mandatory)][int]$BandPort,
        [Parameter(Mandatory)][int]$MasterPort,
        [string]$Layout = '',
        [string]$BandUserLibrary = '',
        [string]$MasterUserLibrary = '',
        [switch]$NoTask,
        [string]$TaskPath = $script:TaskPath,
        [int]$ReadyTimeoutSeconds = 20
    )
    # ---- checks ----
    $BundleZip = Resolve-FohPath $BundleZip
    $DataDir = Resolve-FohPath $DataDir
    if (-not (Test-Path -LiteralPath $BundleZip -PathType Leaf)) { throw "bundle not found: $BundleZip" }
    Test-FohUserName $BandUser
    Test-FohUserName $MasterUser
    if ($BandUser -eq $MasterUser) { throw 'the band and master users must be two accounts' }
    foreach ($p in @($HttpPort, $BandPort, $MasterPort)) { if ($p -lt 1 -or $p -gt 65535) { throw "port $p refused" } }
    if (@($HttpPort, $BandPort, $MasterPort | Select-Object -Unique).Count -ne 3) {
        throw "the ports must differ: http $HttpPort, band $BandPort, master $MasterPort"
    }
    if (-not $BandUserLibrary) { $BandUserLibrary = Join-Path $env:SystemDrive "Users\$BandUser\Documents\Ableton\User Library" }
    if (-not $MasterUserLibrary) { $MasterUserLibrary = Join-Path $env:SystemDrive "Users\$MasterUser\Documents\Ableton\User Library" }
    $BandUserLibrary = Resolve-FohPath $BandUserLibrary
    $MasterUserLibrary = Resolve-FohPath $MasterUserLibrary
    if ($BandUserLibrary -eq $MasterUserLibrary) { throw "both users would share one User Library: $BandUserLibrary" }
    foreach ($lib in @($BandUserLibrary, $MasterUserLibrary)) {
        if (-not (Test-Path -LiteralPath $lib -PathType Container)) {
            throw "Live's User Library not found: $lib (Live creates it at its first start; pass its folder with -BandUserLibrary or -MasterUserLibrary when Live keeps it elsewhere)"
        }
    }
    if ($Layout) {
        $Layout = Resolve-FohPath $Layout
        Test-FohLayoutFile -Path $Layout
    }
    if (-not $NoTask) {
        $me = New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())
        if (-not $me.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'run the install elevated: it registers the tasks for the band user and writes both users'' User Libraries'
        }
        foreach ($u in @($BandUser, $MasterUser)) {
            try {
                $null = (New-Object Security.Principal.NTAccount $u).Translate([Security.Principal.SecurityIdentifier])
            } catch {
                throw "no account $u on this PC: $($_.Exception.Message)"
            }
        }
    }

    # ---- the bundle, unpacked and checked before the hub is stopped ----
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    if (-not $NoTask) { Set-FohDataDirAcl -Path $DataDir -User $BandUser }
    $unpacked = Expand-FohBundle -BundleZip $BundleZip -DataDir $DataDir
    try {
        Write-Host "fohmixer install: bundle $($unpacked.version) checked against its SHA256SUMS"
        # 1. the running hub
        if (-not $NoTask) { Write-Host ('fohmixer install: ' + (Stop-FohHub -DataDir $DataDir -TaskPath $TaskPath)) }
        # 2. app\<version>, current.txt
        $app = Install-FohAppDir -Unpacked $unpacked -DataDir $DataDir
    } finally {
        if (Test-Path -LiteralPath $unpacked.dir) { Remove-Item -LiteralPath $unpacked.dir -Recurse -Force }
    }
    Write-Host "fohmixer install: $($app.dir) (changed: $($app.changed))"
    # 3. the config
    $toml = Write-FohText -Path (Join-Path $DataDir 'fohmixer-hub.toml') -Text (New-FohHubToml -HttpPort $HttpPort -BandPort $BandPort -MasterPort $MasterPort)
    Write-Host "fohmixer install: fohmixer-hub.toml (changed: $toml)"
    # 4. the layout
    $layoutChanged = $false
    if ($Layout) {
        $layoutChanged = Install-FohLayout -Layout $Layout -DataDir $DataDir
        Write-Host "fohmixer install: layout.json from $Layout (changed: $layoutChanged)"
    } elseif (-not (Test-Path -LiteralPath (Join-Path $DataDir 'layout.json') -PathType Leaf)) {
        Write-Warning "fohmixer install: no layout.json in $DataDir and no -Layout: the hub serves no layout until one is there"
    }
    # 5. the FohMixer copies
    $source = Join-Path $app.dir $script:ScriptName
    $copies = @()
    foreach ($u in @(
            @{ instance = 'band'; user = $BandUser; library = $BandUserLibrary; port = $BandPort },
            @{ instance = 'master'; user = $MasterUser; library = $MasterUserLibrary; port = $MasterPort })) {
        $r = Install-FohUserScript -Source $source -UserLibrary $u.library -Instance $u.instance -Port $u.port
        Write-Host ("fohmixer install: {0} -> {1} (replaced: {2}, Config.py written: {3})" -f $u.instance, $r.path, $r.replaced, $r.config_written)
        $copies += [pscustomobject]@{ instance = $u.instance; path = $r.path; replaced = $r.replaced; config_written = $r.config_written; removed = $r.removed }
    }
    # 6. and 7. the tasks, the start, the readiness
    $answer = $null
    if (-not $NoTask) {
        Register-FohHubTasks -AppDir $app.dir -DataDir $DataDir -User $BandUser -TaskPath $TaskPath
        Write-Host "fohmixer install: tasks $TaskPath$($script:HubTask) and $TaskPath$($script:StopTask) registered and read back"
        Start-ScheduledTask -TaskPath $TaskPath -TaskName $script:HubTask
        $answer = Wait-FohHubReady -HttpPort $HttpPort -Version $unpacked.version -TimeoutSeconds $ReadyTimeoutSeconds
        Write-Host "fohmixer install: the hub answers version $($answer.version) ($($answer.git_hash))"
    }
    return [pscustomobject]@{
        version = $unpacked.version; app = $app.dir; app_changed = $app.changed; config_changed = $toml
        layout_changed = $layoutChanged; copies = $copies; hub = $answer
    }
}
