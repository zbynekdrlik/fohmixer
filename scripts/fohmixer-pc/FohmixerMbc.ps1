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
# - The MBC operator's app window (#280): the hub's [tray] app (the tray's
#   Open brings that window to the front, else starts it; never a browser
#   tab) and the MBC user's Startup shortcut "MBC mixer" with the same
#   command (Brave's app window on the local hub, in -AppProfile).
# No site value lives here: addresses, folders and accounts come as parameters.

# The MBC Live's instance (fohmixer_proto::roles::MBC_INSTANCE).
$script:MbcInstance = 'mbc'
# Where a replaced FohMixer copy of the MBC Live is kept, in the data folder.
$script:ScriptBackups = 'script-backups'
# The firewall rule of the instance link's port.
$script:LinkRule = 'fohmixer-hub-link'
# The MBC user's Startup shortcut of the app window (#280).
$script:AppShortcut = 'MBC mixer.lnk'
# Brave below Program Files and a user's own AppData\Local.
$script:BraveExe = 'BraveSoftware\Brave-Browser\Application\brave.exe'

function Get-FohTrayApp {
    # The MBC PC's app window command (#280): Brave's app window on the local
    # hub (no Access sign-in, no internet), in the profile -AppProfile names
    # (the one the owner set up: no profile picker), maximized.
    param([Parameter(Mandatory)][string]$Browser, [Parameter(Mandatory)][string]$AppProfile, [Parameter(Mandatory)][int]$HttpPort)
    return , @($Browser, ('--profile-directory=' + $AppProfile), ('--app=http://localhost:{0}/' -f $HttpPort), '--start-maximized')
}

function Get-FohTrayToml {
    # The [tray] table (#280, CRLF lines): app as TOML literal strings, so a
    # path's backslashes need no escaping. A word that is empty or holds a
    # single quote or a control character is refused (the hub checks the
    # rest: config check).
    param([Parameter(Mandatory)][AllowEmptyCollection()][AllowEmptyString()][string[]]$App)
    if ($App.Count -eq 0) { throw 'app command refused: no program' }
    foreach ($w in $App) {
        if (-not $w -or $w -match "['\x00-\x1f\x7f]") { throw "app command word refused: [$w]" }
    }
    $words = @($App | ForEach-Object { "'" + $_ + "'" }) -join ', '
    return ("`r`n[tray]`r`napp = [" + $words + "]`r`n")
}

function Format-FohAppArgs {
    # A shortcut's argument line of $Words: a word with a space (or an empty
    # one) in double quotes, the rest as it is; a double quote is refused.
    param([AllowEmptyCollection()][AllowEmptyString()][string[]]$Words = @())
    $out = @()
    foreach ($w in $Words) {
        if ($w.Contains('"')) { throw "app command word refused (it holds a double quote): $w" }
        if (-not $w -or $w.Contains(' ')) { $out += ('"' + $w + '"') } else { $out += $w }
    }
    return ($out -join ' ')
}

function Resolve-FohAppBrowser {
    # The browser of the app window (#280): -AppBrowser when given, else
    # Brave where Windows keeps it: its App Paths entry, Program Files (both
    # views), the MBC user's own install. Refused before any change when
    # that file does not exist.
    param([AllowEmptyString()][string]$AppBrowser = '', [Parameter(Mandatory)][string]$MbcUser)
    $candidates = @()
    if ($AppBrowser) {
        $candidates += $AppBrowser
    } else {
        foreach ($key in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\brave.exe',
                'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\brave.exe')) {
            if (Test-Path -LiteralPath $key) {
                $value = (Get-Item -LiteralPath $key).GetValue('')
                if ($value) { $candidates += ([string]$value).Trim('"') }
            }
        }
        foreach ($root in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
            if ($root) { $candidates += (Join-Path $root $script:BraveExe) }
        }
        $candidates += (Join-Path $env:SystemDrive ("Users\$MbcUser\AppData\Local\" + $script:BraveExe))
    }
    foreach ($c in $candidates) {
        if (Test-Path -LiteralPath $c -PathType Leaf) { return (Resolve-FohPath $c) }
    }
    if ($AppBrowser) { throw "-AppBrowser not found: $AppBrowser" }
    throw 'Brave not found (App Paths, Program Files, the MBC user''s own install): pass its brave.exe with -AppBrowser'
}

function Resolve-FohStartupFolder {
    # The MBC user's Startup folder (#280): -StartupFolder when given, else
    # <SystemDrive>\Users\<MBC user>\...\Startup; refused before any change
    # when it does not exist.
    param([AllowEmptyString()][string]$StartupFolder = '', [Parameter(Mandatory)][string]$MbcUser)
    if (-not $StartupFolder) {
        $StartupFolder = Join-Path $env:SystemDrive "Users\$MbcUser\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup"
    }
    if (-not (Test-Path -LiteralPath $StartupFolder -PathType Container)) {
        throw "the MBC user's Startup folder not found: $StartupFolder (pass -StartupFolder)"
    }
    return (Resolve-FohPath $StartupFolder)
}

function Set-FohAppShortcut {
    # The Startup shortcut "MBC mixer" (#280) in $Folder running $App (the
    # program, then its arguments): kept when its target and arguments are
    # these already, else written. Returns whether it was written.
    param([Parameter(Mandatory)][string]$Folder, [Parameter(Mandatory)][string[]]$App)
    $path = Join-Path $Folder $script:AppShortcut
    $arguments = Format-FohAppArgs -Words @($App | Select-Object -Skip 1)
    $shell = New-Object -ComObject WScript.Shell
    try {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $old = $shell.CreateShortcut($path)
            if (($old.TargetPath -ieq $App[0]) -and ($old.Arguments -ceq $arguments)) { return $false }
        }
        $link = $shell.CreateShortcut($path)
        $link.TargetPath = $App[0]
        $link.Arguments = $arguments
        $link.WorkingDirectory = Split-Path -Parent $App[0]
        $link.Description = 'fohmixer: the MBC mixer app window (#280)'
        $link.Save()
        return $true
    } finally {
        $null = [Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    }
}

function Get-FohMbcAppPlan {
    # The app window's plan of an MBC install (#280), checked before any
    # change: the command, its [tray] table and the Startup folder.
    param([AllowEmptyString()][string]$AppBrowser = '', [Parameter(Mandatory)][string]$AppProfile,
          [Parameter(Mandatory)][int]$HttpPort, [Parameter(Mandatory)][string]$MbcUser, [AllowEmptyString()][string]$StartupFolder = '')
    if (-not $AppProfile.Trim()) { throw '-AppProfile refused: empty' }
    $app = Get-FohTrayApp -Browser (Resolve-FohAppBrowser -AppBrowser $AppBrowser -MbcUser $MbcUser) -AppProfile $AppProfile -HttpPort $HttpPort
    $toml = Get-FohTrayToml -App $app
    $null = Format-FohAppArgs -Words @($app | Select-Object -Skip 1)
    return [pscustomobject]@{ app = $app; toml = $toml; startup = (Resolve-FohStartupFolder -StartupFolder $StartupFolder -MbcUser $MbcUser) }
}

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
    # link on -LinkListen, <LAN address>:<port>), then $Tray (the [tray]
    # table, #280, Get-FohTrayToml), then $Remote (the text from [tls] to the
    # end as installed, Get-FohInstalledRemoteToml: tables written there by
    # hand are kept).
    param([Parameter(Mandatory)][int]$HttpPort, [Parameter(Mandatory)][int]$MbcPort,
          [Parameter(Mandatory)][string]$LinkListen, [AllowEmptyString()][string]$Tray = '', [AllowEmptyString()][string]$Remote = '')
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
    return (($lines -join "`r`n") + "`r`n" + $Tray + $Remote)
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

function Test-FohMbcInstallArgs {
    # The MBC PC's install parameters checked before any change (#184): a
    # local account, the HTTP, script and link ports all different (the
    # hub's config check does not compare the link's with the others, and a
    # taken link port is only retried), Live's own script folder, the link
    # address's form (the hub checks the address itself). Returns the script
    # folder resolved and the link's port.
    param([string]$MbcUser, [int]$HttpPort, [int]$MbcPort, [string]$LinkListen, [string]$ScriptFolder)
    Test-FohUserName $MbcUser
    $null = New-FohMbcHubToml -HttpPort $HttpPort -MbcPort $MbcPort -LinkListen $LinkListen
    $linkPort = [int]($LinkListen.Substring($LinkListen.LastIndexOf(':') + 1))
    foreach ($p in @($HttpPort, $MbcPort, $linkPort)) { if ($p -lt 1 -or $p -gt 65535) { throw "port $p refused" } }
    if (@($HttpPort, $MbcPort, $linkPort | Select-Object -Unique).Count -ne 3) {
        throw "the ports must differ: http $HttpPort, mbc $MbcPort, link $linkPort"
    }
    $ScriptFolder = Resolve-FohPath $ScriptFolder
    Test-FohScriptFolder -Path $ScriptFolder
    return [pscustomobject]@{ scriptFolder = $ScriptFolder; linkPort = $linkPort }
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
