#Requires -Version 5.1
# The MBC PC (#184, spec docs/superpowers/specs/2026-11-12-mbc-instance-design.md
# sections 6.1 and 6.5), dot-sourced by FohmixerPc.psm1 and shipped in the
# bundle next to it. Windows PowerShell 5.1, ASCII only.
# - The MBC hub's toml (New-FohMbcHubToml): its one instance `mbc` (the MBC
#   Live's script on 127.0.0.1) and [link] (its instance link on a LAN
#   address, never every address: the hub's own config check refuses
#   0.0.0.0, PowerShell only what breaks the TOML line); never the FOH PC's
#   [companion], [ableset], [iem] or [presenter].
# - The FohMixer copy in Live's built-in MIDI Remote Scripts folder
#   (Install-FohMbcScript; spec C5: the MBC Live's User Library is left as it
#   is), its log folder in the data folder (Live's user cannot write Live's
#   own folder) and a replaced copy kept, dated, in the data folder (every
#   folder in Live's would be listed as a control surface).
# - The FOH hub's linked instance (Get-FohLinkedToml, Resolve-FohMbcLink):
#   written with -MbcLink, kept by an update without it.
# No site value lives here: addresses, folders and accounts come as parameters.

# The MBC Live's instance (fohmixer_proto::roles::MBC_INSTANCE).
$script:MbcInstance = 'mbc'
# Where a replaced FohMixer copy of the MBC Live is kept, in the data folder.
$script:ScriptBackups = 'script-backups'
# The firewall rule of the instance link's port.
$script:LinkRule = 'fohmixer-hub-link'

function Get-FohLinkedToml {
    # The FOH hub's [[instances]] table of the MBC Live, reached through the
    # MBC hub's instance link (CRLF lines, '' without -Link): ws://<MBC LAN
    # address>:<port>. The hub checks the address (config check:
    # config::is_link_url); here only what could break the TOML line is
    # refused.
    param([AllowEmptyString()][string]$Link = '')
    if (-not $Link) { return '' }
    if ($Link -cnotmatch '\Aws://[^\s"\\]+\z') { throw "MBC link refused: [$Link] (ws://<MBC LAN address>:<port>)" }
    $lines = @('', '[[instances]]', ('name = "{0}"' -f $script:MbcInstance), ('link = "{0}"' -f $Link))
    return (($lines -join "`r`n") + "`r`n")
}

function Get-FohInstalledLinkedToml {
    # The linked instances' tables of an installed fohmixer-hub.toml's text
    # (#184): each [[instances]] table with a `link` key, from the line end
    # before it to the next table, exactly as Get-FohLinkedToml wrote it;
    # looked for only before [tls] (from there on the text is the remote
    # tables, kept or rewritten as a whole); '' without one.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text)
    $head = $Text
    $tls = [regex]::Match($Text, '\r?\n\[tls\]\r?\n')
    if ($tls.Success) { $head = $Text.Substring(0, $tls.Index) }
    $out = ''
    foreach ($m in [regex]::Matches($head, '(?s)\r?\n\[\[instances\]\]\r?\n.*?(?=\r?\n\[|\z)')) {
        if ($m.Value -cmatch '(?m)^link[ \t]*=') { $out += $m.Value }
    }
    return $out
}

function Resolve-FohMbcLink {
    # The FOH install's linked instance (#184), checked before any change:
    # from -MbcLink when given, else the installed toml's as it is (a routine
    # update keeps it), else ''.
    param([Parameter(Mandatory)][string]$DataDir, [AllowEmptyString()][string]$MbcLink = '')
    if ($MbcLink) { return (Get-FohLinkedToml -Link $MbcLink) }
    $toml = Join-Path (Resolve-FohPath $DataDir) 'fohmixer-hub.toml'
    if (-not (Test-Path -LiteralPath $toml -PathType Leaf)) { return '' }
    return (Get-FohInstalledLinkedToml -Text ([IO.File]::ReadAllText($toml)))
}

function New-FohMbcHubToml {
    # The MBC hub's fohmixer-hub.toml (#184): the HTTP port, the layout file,
    # the one instance `mbc` on its script's port and [link] (the instance
    # link on -LinkListen, <LAN address>:<port>), then $Remote (the text from
    # [tls] to the end as installed, Get-FohInstalledRemoteToml: tables
    # written there by hand are kept).
    param([Parameter(Mandatory)][int]$HttpPort, [Parameter(Mandatory)][int]$MbcPort,
          [Parameter(Mandatory)][string]$LinkListen, [AllowEmptyString()][string]$Remote = '')
    if ($LinkListen -cnotmatch '\A[^\s"\\]+:[0-9]{1,5}\z') {
        throw "link address refused: [$LinkListen] (<MBC LAN address>:<port>)"
    }
    $lines = @(
        '# fohmixer-hub configuration, written by Install-Fohmixer.ps1: run the install again to change it.',
        ('http_port = {0}' -f $HttpPort),
        'layout = "layout.json"',
        '',
        '[[instances]]',
        ('name = "{0}"' -f $script:MbcInstance),
        ('port = {0}' -f $MbcPort),
        '',
        '[link]',
        ('listen = "{0}"' -f $LinkListen),
        ('instance = "{0}"' -f $script:MbcInstance))
    return (($lines -join "`r`n") + "`r`n" + $Remote)
}

function Test-FohScriptFolder {
    # Live's built-in MIDI Remote Scripts folder (its Resources): it exists
    # and holds Live's own _Framework, so a wrong folder is refused before
    # anything changes.
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath (Join-Path $Path '_Framework') -PathType Container)) {
        throw "-ScriptFolder is not Live's MIDI Remote Scripts folder (no _Framework in it): $Path"
    }
}

function Install-FohMbcScript {
    # The MBC Live's FohMixer (#184, spec 6.5): <ScriptFolder>\FohMixer with
    # INSTANCE mbc, its port and its log folder in the data folder; a copy
    # it replaces (a Live update brings none, an older bundle's does) kept
    # whole under <DataDir>\script-backups\<UTC stamp>. Returns what changed.
    param([Parameter(Mandatory)][string]$Source, [Parameter(Mandatory)][string]$ScriptFolder,
          [Parameter(Mandatory)][int]$Port, [Parameter(Mandatory)][string]$DataDir)
    Test-FohScriptFolder -Path $ScriptFolder
    return (Install-FohScriptCopy -Source $Source -Dest (Join-Path $ScriptFolder $script:ScriptName) -Instance $script:MbcInstance `
            -Port $Port -LogDir (Join-Path $DataDir 'logs') -BackupDir (Join-Path $DataDir $script:ScriptBackups))
}

function Test-FohSiteArgs {
    # The parameters of the other site refused before any change (#184):
    # $Given are the bound parameters' names, $Others the other site's.
    param([Parameter(Mandatory)][string]$Site, [string[]]$Given = @(), [string[]]$Others = @(), [Parameter(Mandatory)][string]$Where)
    $wrong = @($Given | Where-Object { $Others -contains $_ })
    if ($wrong.Count -gt 0) {
        throw ('-Site {0} refuses -{1}: it is for the {2}' -f $Site, ($wrong -join ', -'), $Where)
    }
}
