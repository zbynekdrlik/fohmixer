#Requires -Version 5.1
# The optional tables of fohmixer-hub.toml an install writes or keeps
# (Get-FohInstalledEqToml, Resolve-FohEq: the Pro-Q 4 screen, #71;
# Get-FohCompanionToml, Get-FohInstalledCompanionToml, Resolve-FohCompanion:
# the Stream Deck tab, #52; Get-FohPresenterToml, Get-FohInstalledPresenterToml,
# Resolve-FohPresenter: Presenter's stage tab, #90), dot-sourced by
# FohmixerPc.psm1 and shipped in the bundle next to it. Windows PowerShell
# 5.1, ASCII only.

function Get-FohInstalledEqToml {
    # The [eq] table of an installed fohmixer-hub.toml's text (#71): the Pro-Q
    # 4 screen, turned on by hand (.claude/rules/deploy-pc.md), so an update
    # keeps it as it is (its backend value included; the new hub checks it).
    # Its lines from [eq] to the next table or the end, looked for only
    # before [tls]: from there on the text is the remote tables, kept or
    # rewritten as a whole (Get-FohInstalledRemoteToml), so an [eq] there is
    # never taken twice. Written by hand, it may have no blank line before
    # it, LF line ends or trailing blanks: it comes back as New-FohHubToml
    # writes a table (a blank line before it, CRLF lines, blank lines
    # dropped); '' without one.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text)
    $head = $Text
    $tls = [regex]::Match($Text, '\r?\n\[tls\]\r?\n')
    if ($tls.Success) { $head = $Text.Substring(0, $tls.Index) }
    # The header may carry a comment, or end the file (#77 review).
    $m = [regex]::Match($head, '(?s)\r?\n\[eq\][ \t]*(?:#[^\r\n]*)?(?:\r?\n.*?(?=\r?\n\[|\z)|\z)')
    if (-not $m.Success) { return '' }
    $lines = @($m.Value -split '\r?\n' | ForEach-Object { $_.TrimEnd() } | Where-Object { $_ })
    return ("`r`n" + ($lines -join "`r`n") + "`r`n")
}

function Resolve-FohEq {
    # The [eq] table of an install (#71): the installed toml's as it is
    # (Get-FohInstalledEqToml: an update keeps the Pro-Q 4 screen on), else
    # '' (a first install: the screen stays off until it is turned on by hand).
    param([Parameter(Mandatory)][string]$DataDir)
    $toml = Join-Path (Resolve-FohPath $DataDir) 'fohmixer-hub.toml'
    if (-not (Test-Path -LiteralPath $toml -PathType Leaf)) { return '' }
    return (Get-FohInstalledEqToml -Text ([IO.File]::ReadAllText($toml)))
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

function Get-FohPresenterToml {
    # The [presenter] table of fohmixer-hub.toml (#90, the stage tab; CRLF
    # lines, '' without -Url): Presenter's server on the LAN as
    # http://<address>[:port]. The hub checks the address (config check:
    # plain http, a LAN address, nothing after the port); here only what
    # could break the TOML line is refused.
    param([AllowEmptyString()][string]$Url = '')
    if (-not $Url) { return '' }
    if ($Url -notmatch '\Ahttp://[^\s"\\]+\z') {
        throw "Presenter address refused: [$Url] (http://<LAN address>[:port])"
    }
    $lines = @('', '[presenter]', ('url = "{0}"' -f $Url))
    return (($lines -join "`r`n") + "`r`n")
}

function Get-FohInstalledPresenterToml {
    # The [presenter] table of an installed fohmixer-hub.toml's text (#90),
    # from the line end before it to the next table or the end, exactly as
    # Get-FohPresenterToml wrote it; '' without one.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text)
    $m = [regex]::Match($Text, '(?s)\r?\n\[presenter\]\r?\n.*?(?=\r?\n\[|\z)')
    if ($m.Success) { return $m.Value }
    return ''
}

function Resolve-FohPresenter {
    # The [presenter] table of an install (#90), checked before any change:
    # from -PresenterUrl when given, else the installed toml's as it is (a
    # routine update keeps the stage tab), else ''.
    param([Parameter(Mandatory)][string]$DataDir, [AllowEmptyString()][string]$PresenterUrl = '')
    if ($PresenterUrl) { return (Get-FohPresenterToml -Url $PresenterUrl) }
    $toml = Join-Path (Resolve-FohPath $DataDir) 'fohmixer-hub.toml'
    if (-not (Test-Path -LiteralPath $toml -PathType Leaf)) { return '' }
    return (Get-FohInstalledPresenterToml -Text ([IO.File]::ReadAllText($toml)))
}
