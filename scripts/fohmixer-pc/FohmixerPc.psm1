#Requires -Version 5.1
# fohmixer S5 (design note docs/superpowers/specs/2026-09-28-s5-deploy-design.md,
# sections 2 and 3): the Ableton PC install, shipped in every bundle next to
# Install-Fohmixer.ps1, and the bundle build that the CI bundle job and the
# self-test (Test-Fohmixer.ps1) share. Windows PowerShell 5.1.
# - Nothing is ended by force (spec I7): the hub is asked to stop with
#   Ctrl-Break, its graceful stop, and waited for; a hub that does not stop is
#   reported, never ended. Its tasks may never be ended hard either. The tray
#   (#39) likewise: asked to exit by a second start with --exit, waited for.
# - Live's preferences and a running Live are never touched: only the FohMixer
#   folder in each user's User Library is written, and Live reads it at its
#   next start. The install reads each user's Library.cfg and refuses a User
#   Library Live does not use (#9); the fix is made in Live, never here.
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
$script:BundleFiles = @('VERSION', 'fohmixer-hub.exe', 'fohmixer-tray.exe', 'Install-Fohmixer.ps1', 'FohmixerPc.psm1',
    'FohmixerLivePrefs.ps1', 'FohmixerFirewall.ps1', 'FohmixerRemote.ps1', 'FohmixerTray.ps1', 'FohmixerHubProcess.ps1',
    'FohmixerTasks.ps1', 'Start-FohmixerHub.ps1', 'Stop-FohmixerHub.ps1',
    'FohMixer/__init__.py', 'FohMixer/Config.py', 'FohMixer/version.py')
# A SemVer version that is also a safe folder name (no trailing dot, no "..").
$script:SemVer = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?\z'

# The check of Live's User Library setting (Get-FohLivePrefsFile,
# Get-FohLiveUserLibrary, Test-FohLiveUserLibrary), in a file of its own.
. (Join-Path $PSScriptRoot 'FohmixerLivePrefs.ps1')
# The firewall rules (Get-FohFirewallRule, Test-FohFirewallRule,
# Set-FohFirewallRule), in a file of their own.
. (Join-Path $PSScriptRoot 'FohmixerFirewall.ps1')
# Remote access (#17): the public name's toml tables, hosts entry, desktop
# shortcut, and the tunnel's token and service, in a file of its own.
. (Join-Path $PSScriptRoot 'FohmixerRemote.ps1')
# The tray (#39): its two tasks, its graceful stop (a second start with
# --exit) and its start, in a file of its own.
. (Join-Path $PSScriptRoot 'FohmixerTray.ps1')

# The running hub (find, start, Ctrl-Break, wait) and our scheduled tasks,
# in files of their own.
. (Join-Path $PSScriptRoot 'FohmixerHubProcess.ps1')
. (Join-Path $PSScriptRoot 'FohmixerTasks.ps1')

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
    # zips the folder's content to $Zip. Entries are named here, '/' separated:
    # ZipFile.CreateFromDirectory keeps '\' in a host without a target framework
    # (powershell.exe), which breaks the zip format for other readers.
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
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::Open($Zip, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($f in @(Get-FohRelativeFiles -Root $Stage)) {
            $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $f.full, $f.rel, [IO.Compression.CompressionLevel]::Optimal)
        }
    } finally {
        $archive.Dispose()
    }
}

function New-FohBundle {
    # The Windows release bundle from a repo checkout, a built hub and a built
    # tray: fohmixer-hub.exe, fohmixer-tray.exe, FohMixer\ (without logs,
    # caches or compiled files), the PC scripts, VERSION (the workspace
    # version) and SHA256SUMS, zipped to
    # <OutDir>\fohmixer-windows-<version>-<Sha>.zip. Returns name, version, stage and zip.
    param([Parameter(Mandatory)][string]$RepoRoot, [Parameter(Mandatory)][string]$HubExe,
          [Parameter(Mandatory)][string]$TrayExe, [Parameter(Mandatory)][string]$OutDir, [Parameter(Mandatory)][string]$Sha)
    if ($Sha -cnotmatch '^[0-9a-f]{7,40}\z') { throw "commit refused: $Sha" }
    $RepoRoot = Resolve-FohPath $RepoRoot
    $HubExe = Resolve-FohPath $HubExe
    $TrayExe = Resolve-FohPath $TrayExe
    $OutDir = Resolve-FohPath $OutDir
    if (-not (Test-Path -LiteralPath $HubExe -PathType Leaf)) { throw "the hub $HubExe does not exist (build it first)" }
    if (-not (Test-Path -LiteralPath $TrayExe -PathType Leaf)) { throw "the tray $TrayExe does not exist (build it first)" }
    $version = Get-FohWorkspaceVersion -CargoToml (Join-Path $RepoRoot 'Cargo.toml')
    $scriptDir = Join-Path $RepoRoot 'live-script\FohMixer'
    $scriptVersion = Get-FohScriptVersion -Path (Join-Path $scriptDir 'version.py')
    if ($scriptVersion -cne $version) { throw "FohMixer\version.py says $scriptVersion, Cargo.toml $version" }
    $name = "fohmixer-windows-$version-$Sha"
    $stage = Join-Path $OutDir $name
    if (Test-Path -LiteralPath $stage) { throw "the stage folder $stage exists already" }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item -LiteralPath $HubExe -Destination (Join-Path $stage $script:HubExe)
    Copy-Item -LiteralPath $TrayExe -Destination (Join-Path $stage $script:TrayExe)
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
    # port, the two instances and the layout file (relative to the data folder),
    # then $Companion (Get-FohCompanionToml: the Stream Deck tab, #52), then
    # $Remote (Get-FohRemoteToml: the remote-access tables, #17, always last:
    # Get-FohInstalledRemoteToml reads from [tls] to the end).
    # There is no data_dir key: the hub's data folder is FOHMIXER_DATA.
    param([Parameter(Mandatory)][int]$HttpPort, [Parameter(Mandatory)][int]$BandPort, [Parameter(Mandatory)][int]$MasterPort,
          [AllowEmptyString()][string]$Companion = '', [AllowEmptyString()][string]$Remote = '')
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
    return (($lines -join "`r`n") + "`r`n" + $Companion + $Remote)
}

function Get-FohCompanionToml {
    # The [companion] table of fohmixer-hub.toml (#52, CRLF lines, '' without
    # -Endpoint): Bitfocus Companion's Satellite API as host or host:port
    # (default port 16622). The grid, the image size and the title keep the
    # hub's defaults; the hub checks the table (config check).
    param([AllowEmptyString()][string]$Endpoint = '')
    if (-not $Endpoint) { return '' }
    $m = [regex]::Match($Endpoint, '\A(?<host>[^\s:"\\]+)(:(?<port>[0-9]{1,5}))?\z')
    $port = 16622
    if ($m.Success -and $m.Groups['port'].Success) { $port = [int]$m.Groups['port'].Value }
    if (-not $m.Success -or $port -lt 1 -or $port -gt 65535) {
        throw "Companion endpoint refused: [$Endpoint] (host or host:port, port 1-65535)"
    }
    $lines = @('', '[companion]', ('host = "{0}"' -f $m.Groups['host'].Value), ('port = {0}' -f $port))
    return (($lines -join "`r`n") + "`r`n")
}

function Get-FohInstalledCompanionToml {
    # The [companion] table of an installed fohmixer-hub.toml's text (#52), from
    # the line end before it to the next table or the end, exactly as
    # Get-FohCompanionToml wrote it; '' without one.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text)
    $m = [regex]::Match($Text, '(?s)\r?\n\[companion\]\r?\n.*?(?=\r?\n\[|\z)')
    if ($m.Success) { return $m.Value }
    return ''
}

function Resolve-FohCompanion {
    # The [companion] table of an install (#52), checked before any change:
    # from -CompanionHost when given, else the installed toml's as it is (a
    # routine update keeps the Stream Deck tab), else ''.
    param([Parameter(Mandatory)][string]$DataDir, [AllowEmptyString()][string]$CompanionHost = '')
    if ($CompanionHost) { return (Get-FohCompanionToml -Endpoint $CompanionHost) }
    $toml = Join-Path (Resolve-FohPath $DataDir) 'fohmixer-hub.toml'
    if (-not (Test-Path -LiteralPath $toml -PathType Leaf)) { return '' }
    return (Get-FohInstalledCompanionToml -Text ([IO.File]::ReadAllText($toml)))
}

function Test-FohHubToml {
    # $Text checked as a config by the hub it is for ($Exe: `fohmixer-hub
    # config check`, config.rs, the one set of rules), before the install
    # stops the running hub: a config the new hub would refuse never reaches
    # the data folder (the hub would not start, the emergency path included).
    # The text goes to a file of its own in $Dir (never inside the bundle's
    # tree, which becomes app\<version>). Returns the hub's answer; throws
    # with its reason.
    param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$Text, [Parameter(Mandatory)][string]$Dir)
    $file = Join-Path $Dir ('.check-' + [guid]::NewGuid().ToString('N') + '.toml')
    [IO.File]::WriteAllText($file, $Text, $script:Utf8NoBom)
    try {
        # Continue only around the native call: its stderr is its answer.
        $said = & {
            $ErrorActionPreference = 'Continue'
            (& $Exe config check $file 2>&1 | ForEach-Object { "$_" }) -join ' '
        }
        $code = $LASTEXITCODE
    } finally {
        Remove-Item -LiteralPath $file
    }
    if ($code -ne 0) { throw "the new config is refused by the hub (exit $code): $said" }
    return $said
}

function Test-FohHubLayout {
    # $Layout checked by the hub it is for ($Exe: `fohmixer-hub layout check`,
    # layout.rs, the one set of rules) against the new config $Text, before the
    # install stops the running hub (#21): a layout the new hub cannot serve (a
    # schema it does not read, an unknown instance) never meets it, so a schema
    # change never leaves the surface without a layout. The config goes to a
    # file of its own in $Dir. Returns the hub's answer; throws with its reason.
    param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$Layout, [Parameter(Mandatory)][string]$Text, [Parameter(Mandatory)][string]$Dir)
    $file = Join-Path $Dir ('.check-' + [guid]::NewGuid().ToString('N') + '.toml')
    [IO.File]::WriteAllText($file, $Text, $script:Utf8NoBom)
    try {
        # Continue only around the native call: its stderr is its answer.
        $said = & {
            $ErrorActionPreference = 'Continue'
            (& $Exe layout check $Layout $file 2>&1 | ForEach-Object { "$_" }) -join ' '
        }
        $code = $LASTEXITCODE
    } finally {
        Remove-Item -LiteralPath $file
    }
    if ($code -ne 0) { throw "the layout is refused by the new hub (exit $code): $said" }
    return $said
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

# ---- the accounts ----

function Get-FohLocalAccountName {
    # An account of this PC as <computer>\<name>; a name with a domain or
    # computer part stays as it is. Windows looks a bare name up as a domain
    # name first: where an account is named like the computer (so on the PC,
    # #9), the bare name is this PC's account domain, which does not translate
    # to an account's SID, while <computer>\<name> is the account. Every
    # lookup of an account and every task principal and trigger use this name.
    param([Parameter(Mandatory)][string]$Name)
    if ($Name.Contains('\')) { return $Name }
    return ($env:COMPUTERNAME + '\' + $Name)
}

function Get-FohAccountSid {
    # The SID of an account of this PC (by Get-FohLocalAccountName); throws
    # "no account <name> on this PC" when it does not resolve.
    param([Parameter(Mandatory)][string]$Name)
    try {
        return (New-Object Security.Principal.NTAccount (Get-FohLocalAccountName $Name)).Translate([Security.Principal.SecurityIdentifier])
    } catch {
        throw "no account $Name on this PC: $($_.Exception.Message)"
    }
}

# ---- the data folder's DACL ----

function Set-FohDataDirAcl {
    # A protected DACL on the data folder, inherited by everything below:
    # SYSTEM and Administrators full control, the hub's user Modify (the hub
    # writes secrets\, the layout backups and its state there; no other user
    # reads its secrets). Read back; throws on a difference.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$User)
    $userSid = Get-FohAccountSid $User
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
    $userSid = (Get-FohAccountSid $User).Value
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

# ---- the install (design note section 3) ----

function Test-FohUserName {
    # A user name as the install takes it: an account of this PC without a
    # domain or computer part (the install adds the computer part itself,
    # Get-FohLocalAccountName).
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Name)
    if ($Name -cnotmatch '^[^"/\\\[\]:;|=,+*?<>]+\z' -or $Name.Trim() -cne $Name -or $Name.EndsWith('.')) {
        throw "user name refused: [$Name] (an account name of this PC, without a domain part)"
    }
}

function Invoke-FohInstall {
    # Design note section 3, in order. Every parameter is checked before
    # anything changes, and the bundle (against its SHA256SUMS, unpacked under
    # <DataDir>\app) before the DACL, the stop and the install. When a step
    # after the stop fails, the hub and tray tasks that ran are started again. -NoTask (the
    # self-test) leaves out what needs the real accounts: the DACL, the stop,
    # the tasks, the firewall rule, the start and the readiness poll. $Remote
    # is Resolve-FohRemote's plan (#17), resolved and checked by the caller;
    # $Companion is the [companion] table (#52, Resolve-FohCompanion).
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
        [string]$BandAbletonPrefs = '',
        [string]$MasterAbletonPrefs = '',
        [switch]$NoTask,
        [string]$TaskPath = $script:TaskPath,
        [int]$ReadyTimeoutSeconds = 20,
        [AllowEmptyString()][string]$Companion = '',
        $Remote = $null
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
    if (-not $BandAbletonPrefs) { $BandAbletonPrefs = Join-Path $env:SystemDrive "Users\$BandUser\AppData\Roaming\Ableton" }
    if (-not $MasterAbletonPrefs) { $MasterAbletonPrefs = Join-Path $env:SystemDrive "Users\$MasterUser\AppData\Roaming\Ableton" }
    Test-FohLiveUserLibrary -User $BandUser -Prefs (Resolve-FohPath $BandAbletonPrefs) -UserLibrary $BandUserLibrary -Switch 'Band'
    Test-FohLiveUserLibrary -User $MasterUser -Prefs (Resolve-FohPath $MasterAbletonPrefs) -UserLibrary $MasterUserLibrary -Switch 'Master'

    if ($Layout) {
        $Layout = Resolve-FohPath $Layout
        Test-FohLayoutFile -Path $Layout
    }
    $remoteToml = ''
    if ($Remote) { $remoteToml = $Remote.toml }
    if (-not $NoTask) {
        $me = New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())
        if (-not $me.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'run the install elevated: it registers the tasks for the band user and writes both users'' User Libraries'
        }
        foreach ($u in @($BandUser, $MasterUser)) { $null = Get-FohAccountSid $u }
    }

    # ---- the bundle: unpacked and checked before anything else changes ----
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
    $unpacked = Expand-FohBundle -BundleZip $BundleZip -DataDir $DataDir
    Write-Host "fohmixer install: bundle $($unpacked.version) checked against its SHA256SUMS"
    $hubWasRunning = $false
    $trayWasRunning = $false
    try {
        # The new config, checked by the new hub before anything else changes.
        $tomlText = New-FohHubToml -HttpPort $HttpPort -BandPort $BandPort -MasterPort $MasterPort -Companion $Companion -Remote $remoteToml
        $accepted = Test-FohHubToml -Exe (Join-Path $unpacked.dir $script:HubExe) -Text $tomlText -Dir (Split-Path -Parent $unpacked.dir)
        Write-Host "fohmixer install: the new hub accepts the config ($accepted)"
        # The layout it will serve (the new one, else the one in place), checked by
        # the new hub too (#21).
        $served = $Layout
        if (-not $served) { $served = Join-Path $DataDir 'layout.json' }
        if (Test-Path -LiteralPath $served -PathType Leaf) {
            $null = Test-FohHubLayout -Exe (Join-Path $unpacked.dir $script:HubExe) -Layout $served -Text $tomlText -Dir (Split-Path -Parent $unpacked.dir)
            Write-Host 'fohmixer install: the new hub accepts the layout'
        }
        if (-not $NoTask) {
            Set-FohDataDirAcl -Path $DataDir -User $BandUser
            # 1. the running tray (#39: it runs from app\<version>), then the running hub
            $trayWasRunning = @(Get-FohTrayProcess -DataDir $DataDir).Count -gt 0
            $trayStop = Stop-FohTray -DataDir $DataDir -TaskPath $TaskPath
            Write-Host "fohmixer install: $($trayStop.text)"
            $hubWasRunning = @(Get-FohHubProcess -DataDir $DataDir).Count -gt 0
            $stop = Stop-FohHub -DataDir $DataDir -TaskPath $TaskPath
            Write-Host "fohmixer install: $($stop.text)"
        }
        # 2. app\<version>, current.txt
        $app = Install-FohAppDir -Unpacked $unpacked -DataDir $DataDir
        Write-Host "fohmixer install: $($app.dir) (changed: $($app.changed))"
        # 3. the config (checked above)
        $toml = Write-FohText -Path (Join-Path $DataDir 'fohmixer-hub.toml') -Text $tomlText
        Write-Host "fohmixer install: fohmixer-hub.toml (changed: $toml)"
        # 4. the layout
        $layoutChanged = $false
        if ($Layout) {
            $layoutChanged = Install-FohLayout -Layout $Layout -DataDir $DataDir
            Write-Host "fohmixer install: layout.json (changed: $layoutChanged)"
        } elseif (-not (Test-Path -LiteralPath (Join-Path $DataDir 'layout.json') -PathType Leaf)) {
            Write-Warning "fohmixer install: no layout.json in $DataDir and no -Layout: the hub serves no layout until one is there"
        }
        # 5. the FohMixer copies (printed without their paths: they name the Windows accounts)
        $source = Join-Path $app.dir $script:ScriptName
        $copies = @()
        foreach ($u in @(
                @{ instance = 'band'; library = $BandUserLibrary; port = $BandPort },
                @{ instance = 'master'; library = $MasterUserLibrary; port = $MasterPort })) {
            $r = Install-FohUserScript -Source $source -UserLibrary $u.library -Instance $u.instance -Port $u.port
            Write-Host ("fohmixer install: the {0} user's FohMixer (replaced: {1}, Config.py written: {2})" -f $u.instance, $r.replaced, $r.config_written)
            $copies += [pscustomobject]@{ instance = $u.instance; replaced = $r.replaced; config_written = $r.config_written; removed = $r.removed }
        }
        # 5b. remote access (#17): the name on this PC, the shortcut, the tunnel token
        $remoteResult = $null
        if ($Remote) { $remoteResult = Install-FohRemoteFiles -Remote $Remote }
        # 6. the tasks and the firewall rule
        if (-not $NoTask) {
            Register-FohHubTasks -AppDir $app.dir -DataDir $DataDir -User $BandUser -TaskPath $TaskPath
            Write-Host "fohmixer install: tasks $TaskPath$($script:HubTask) and $TaskPath$($script:StopTask) registered and read back"
            Register-FohTrayTasks -AppDir $app.dir -DataDir $DataDir -User $BandUser -TaskPath $TaskPath
            Write-Host "fohmixer install: tasks $TaskPath$($script:TrayTask) and $TaskPath$($script:TrayStopTask) registered and read back"
            $firewall = Set-FohFirewallRule -Port $HttpPort
            Write-Host "fohmixer install: firewall rule fohmixer-hub-http, TCP $HttpPort, Domain and Private (changed: $firewall)"
            if ($Remote) { Install-FohRemoteServices -Remote $Remote -Result $remoteResult }
            foreach ($n in @(Get-NetConnectionProfile | Where-Object { "$($_.NetworkCategory)" -eq 'Public' })) {
                $publicNote = 'fohmixer install: the network on {0} is Public; the rule allows Domain and Private only, ' +
                    'so clients there are refused until it is set to Private'
                Write-Warning ($publicNote -f $n.InterfaceAlias)
            }
        }
    } catch {
        $failure = $_
        # A hub or a tray that ran before this install and is down now (the
        # stops above ended them): start its task again, so a failed install
        # does not leave either down.
        $restart = @()
        if ($hubWasRunning -and @(Get-FohHubProcess -DataDir $DataDir).Count -eq 0) {
            $restart += [pscustomobject]@{ task = $script:HubTask; what = 'hub' }
        }
        if ($trayWasRunning -and @(Get-FohTrayProcess -DataDir $DataDir).Count -eq 0) {
            $restart += [pscustomobject]@{ task = $script:TrayTask; what = 'tray' }
        }
        if ($restart.Count -eq 0) { throw }
        $again = @()
        foreach ($r in $restart) {
            try {
                Start-ScheduledTask -TaskPath $TaskPath -TaskName $r.task
                $again += "the $($r.what) task was started again"
            } catch {
                $again += "starting the $($r.what) task again failed too: $($_.Exception.Message)"
            }
        }
        throw ("{0} ({1})`n{2}" -f $failure.Exception.Message, ($again -join '; '), $failure.ScriptStackTrace)
    } finally {
        if (Test-Path -LiteralPath $unpacked.dir) { Remove-Item -LiteralPath $unpacked.dir -Recurse -Force }
    }
    # 7. the start and the readiness
    $answer = $null
    $trayPid = $null
    if (-not $NoTask) {
        Start-ScheduledTask -TaskPath $TaskPath -TaskName $script:HubTask
        try {
            $answer = Wait-FohHubReady -HttpPort $HttpPort -Version $unpacked.version -TimeoutSeconds $ReadyTimeoutSeconds
        } catch {
            throw ($_.Exception.Message + "`n" + (Get-FohLogTail -DataDir $DataDir))
        }
        Write-Host "fohmixer install: the hub answers version $($answer.version) ($($answer.git_hash))"
        # 7b. the tray (#39), after the hub: its first poll finds the hub up
        $trayPid = Start-FohTray -AppDir $app.dir -DataDir $DataDir -TaskPath $TaskPath
        Write-Host "fohmixer install: the tray runs (pid $trayPid)"
    }
    return [pscustomobject]@{
        version = $unpacked.version; app = $app.dir; app_changed = $app.changed; config_changed = $toml
        layout_changed = $layoutChanged; copies = $copies; hub = $answer; tray = $trayPid; remote = $remoteResult; companion = [bool]$Companion
    }
}
