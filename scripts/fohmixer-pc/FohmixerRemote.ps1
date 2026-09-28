#Requires -Version 5.1
# fohmixer remote access on the Ableton PC (#17), dot-sourced by
# FohmixerPc.psm1 and shipped in the bundle next to it. One public name for
# the LAN and the Cloudflare tunnel:
# - the hub's toml gets [tls] (the HTTPS listener of the name), [acme] (its
#   Let's Encrypt certificate by DNS-01; the Cloudflare API token is set by
#   `fohmixer-hub cloudflare set-token` as the band user, never here),
#   [access] (the Access application's team and AUD tag) and [tunnel]
#   (cloudflared's readiness);
# - this PC resolves the name itself: a marked block in the hosts file maps
#   it to 127.0.0.1, so the mixer opens here with the router or the internet
#   down; a desktop shortcut for the band user opens https://<name>/;
# - the tunnel is its own Windows service, fohmixer-tunnel (an older
#   Cloudflared service of an earlier setup is never touched): cloudflared
#   with --protocol http2 (TCP, where a network blocks QUIC), its metrics on
#   127.0.0.1 (the hub reads /ready), and the connector token in a file with a
#   protected DACL (SYSTEM, Administrators), never on a command line: the
#   install reads it from stdin or a hidden prompt. The tunnel's ingress
#   (the name -> http://127.0.0.1:<HttpPort>) is Cloudflare's configuration
#   (scripts/cloudflare/fohmixer_cloudflare.py).
# No site value lives here (spec 5.2): the name, the team, the AUD and the
# token come as parameters. Windows PowerShell 5.1, ASCII only.

$script:HostsBegin = '# BEGIN fohmixer (Install-Fohmixer.ps1, #17: the public name on this PC)'
$script:HostsEnd = '# END fohmixer'
$script:TunnelService = 'fohmixer-tunnel'
$script:TunnelTokenFile = 'tunnel-token'
$script:ShortcutFile = 'fohmixer.url'

function Test-FohPublicName {
    # The public name as the hub takes it (config.rs is_dns_name): at least two
    # labels of letters, digits and inner hyphens (1-63 characters), the last
    # with a letter, at most 253 characters. Throws naming it otherwise.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Name)
    $labels = $Name.Split('.')
    $ok = $labels.Count -ge 2 -and $Name.Length -le 253 -and $labels[$labels.Count - 1] -cmatch '[A-Za-z]'
    foreach ($l in $labels) {
        if ($l -cnotmatch '^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?\z') { $ok = $false }
    }
    if (-not $ok) { throw "public name refused: [$Name] (a DNS name like foh.example.org)" }
}

function Get-FohPublicUrl {
    # https://<name>/ (with :<port> when it is not 443).
    param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][int]$HttpsPort)
    if ($HttpsPort -eq 443) { return "https://$Name/" }
    return "https://${Name}:$HttpsPort/"
}

function Get-FohRemoteToml {
    # The remote-access tables of fohmixer-hub.toml (CRLF lines, '' without a
    # public name). [acme] always goes with [tls]; [access] with a team;
    # [tunnel] with the tunnel service.
    param([AllowEmptyString()][string]$Name = '', [int]$HttpsPort = 443, [AllowEmptyString()][string]$AcmeEmail = '',
          [AllowEmptyString()][string]$AcmeDirectory = '', [AllowEmptyString()][string]$AccessTeam = '',
          [string[]]$AccessAud = @(), [switch]$Tunnel, [int]$TunnelMetricsPort = 20241)
    if (-not $Name) { return '' }
    $lines = @('', '[tls]', ('name = "{0}"' -f $Name), ('port = {0}' -f $HttpsPort), '', '[acme]')
    if ($AcmeEmail) { $lines += ('email = "{0}"' -f $AcmeEmail) }
    if ($AcmeDirectory) { $lines += ('directory = "{0}"' -f $AcmeDirectory) }
    if ($AccessTeam) {
        $auds = ($AccessAud | ForEach-Object { '"' + $_ + '"' }) -join ', '
        $lines += @('', '[access]', ('team_domain = "{0}"' -f $AccessTeam), ('aud = [{0}]' -f $auds))
    }
    if ($Tunnel) {
        $lines += @('', '[tunnel]', ('ready_url = "http://127.0.0.1:{0}/ready"' -f $TunnelMetricsPort))
    }
    return (($lines -join "`r`n") + "`r`n")
}

function Get-FohHostsText {
    # The hosts file's text with the fohmixer block mapping $Name to 127.0.0.1:
    # an existing block is replaced in place, else it is added at the end;
    # every other line stays as it was (its line ends included).
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text, [Parameter(Mandatory)][string]$Name)
    $block = $script:HostsBegin + "`r`n" + "127.0.0.1 $Name" + "`r`n" + $script:HostsEnd + "`r`n"
    $pattern = '(?ms)^' + [regex]::Escape($script:HostsBegin) + '\r?\n.*?^' + [regex]::Escape($script:HostsEnd) + '[^\r\n]*(\r?\n|\z)'
    $re = New-Object System.Text.RegularExpressions.Regex $pattern
    if ($re.IsMatch($Text)) {
        return $re.Replace($Text, $block.Replace('$', '$$'), 1)
    }
    if ($Text.Length -gt 0 -and -not $Text.EndsWith("`n")) { $Text += "`r`n" }
    return ($Text + $block)
}

function Set-FohHostsEntry {
    # The fohmixer block in the hosts file at $Path (ASCII, as Windows keeps
    # it); written only when it changes. Returns whether it changed.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Name)
    $text = ''
    if (Test-Path -LiteralPath $Path -PathType Leaf) { $text = [IO.File]::ReadAllText($Path, [Text.Encoding]::ASCII) }
    $new = Get-FohHostsText -Text $text -Name $Name
    if ($new -ceq $text) { return $false }
    [IO.File]::WriteAllText($Path, $new, [Text.Encoding]::ASCII)
    return $true
}

function Set-FohDesktopShortcut {
    # An internet shortcut to $Url in the folder $Desktop (the band user's
    # desktop), written only when it changes. Returns whether it changed.
    param([Parameter(Mandatory)][string]$Desktop, [Parameter(Mandatory)][string]$Url)
    if (-not (Test-Path -LiteralPath $Desktop -PathType Container)) { throw "the band user's desktop not found: $Desktop (pass -BandDesktop)" }
    $text = "[InternetShortcut]`r`nURL=$Url`r`n"
    return (Write-FohText -Path (Join-Path $Desktop $script:ShortcutFile) -Text $text)
}

function Get-FohTunnelCommand {
    # The fohmixer-tunnel service's command line: cloudflared runs the
    # remotely managed tunnel of the token in $TokenFile, over HTTP/2 (TCP),
    # with its metrics (/ready) on 127.0.0.1:$MetricsPort, never updating itself.
    param([Parameter(Mandatory)][string]$Exe, [Parameter(Mandatory)][string]$TokenFile, [Parameter(Mandatory)][int]$MetricsPort)
    return ((Format-FohArg $Exe) + ' tunnel --no-autoupdate --protocol http2 --metrics 127.0.0.1:' + $MetricsPort +
        ' run --token-file ' + (Format-FohArg $TokenFile))
}

function Test-FohTunnelToken {
    # A cloudflared connector token as the Cloudflare dashboard or API gives
    # it: one line of base64 characters, at least 100.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Token)
    return ($Token -cmatch '^[A-Za-z0-9+/=_-]{100,}\z')
}

function Read-FohTunnelToken {
    # The connector token from stdin when it is redirected (the install run
    # over MCP: pipe the token file in), else a hidden prompt. Never echoed.
    if ([Console]::IsInputRedirected) {
        $token = [Console]::In.ReadLine()
    } else {
        $secure = Read-Host -AsSecureString 'cloudflared tunnel token'
        $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        try { $token = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr) } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr) }
    }
    if ($null -eq $token) { $token = '' }
    $token = $token.Trim()
    if (-not (Test-FohTunnelToken -Token $token)) {
        throw ('tunnel token refused: one line of at least 100 base64 characters, as Cloudflare gives it ' +
            '(read ' + $token.Length + ' characters)')
    }
    return $token
}

function Set-FohTunnelDir {
    # The tunnel folder with a protected DACL (SYSTEM and Administrators full
    # control, inherited below): the connector token lets anyone run the
    # tunnel, so neither the band user nor the hub reads it.
    param([Parameter(Mandatory)][string]$Path)
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
    $full = [Security.AccessControl.FileSystemRights]::FullControl
    $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @('S-1-5-18', 'S-1-5-32-544')) {
        $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule(
            (New-Object Security.Principal.SecurityIdentifier $sid), $full, $inherit,
            [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow)))
    }
    [IO.Directory]::SetAccessControl($Path, $acl)
    $problems = @(Test-FohTunnelDirAcl -Path $Path)
    if ($problems.Count -gt 0) { throw ('tunnel folder DACL read-back: ' + ($problems -join '; ')) }
}

function Test-FohTunnelDirAcl {
    # The tunnel folder's DACL read back against Set-FohTunnelDir; returns the differences.
    param([Parameter(Mandatory)][string]$Path)
    $acl = [IO.Directory]::GetAccessControl($Path)
    $bad = @()
    if (-not $acl.AreAccessRulesProtected) { $bad += 'it inherits from its parent' }
    $seen = @()
    foreach ($r in @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))) {
        $sid = $r.IdentityReference.Value
        if (@('S-1-5-18', 'S-1-5-32-544') -notcontains $sid) { $bad += "a rule for $sid" }
        if ($r.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) { $bad += "a deny rule for $sid" }
        $seen += $sid
    }
    foreach ($sid in @('S-1-5-18', 'S-1-5-32-544')) { if ($seen -notcontains $sid) { $bad += "no rule for $sid" } }
    return $bad
}

function Set-FohTunnelToken {
    # The connector token in <TunnelDir>\tunnel-token (the folder's DACL set
    # first), written only when it changes. Returns whether it changed.
    param([Parameter(Mandatory)][string]$TunnelDir, [Parameter(Mandatory)][string]$Token)
    if (-not (Test-FohTunnelToken -Token $Token)) { throw 'tunnel token refused' }
    Set-FohTunnelDir -Path $TunnelDir
    $path = Join-Path $TunnelDir $script:TunnelTokenFile
    if ((Test-Path -LiteralPath $path -PathType Leaf) -and ([IO.File]::ReadAllText($path) -ceq $Token)) { return $false }
    [IO.File]::WriteAllText($path, $Token, [Text.Encoding]::ASCII)
    return $true
}

function Get-FohServicePath {
    # A service's command line (ImagePath), or $null when it does not exist.
    param([Parameter(Mandatory)][string]$Name)
    $s = Get-CimInstance -ClassName Win32_Service -Filter ("Name = '{0}'" -f $Name)
    if ($null -eq $s) { return $null }
    return [string]$s.PathName
}

function Set-FohTunnelService {
    # The fohmixer-tunnel service: created (automatic start, LocalSystem) or
    # its command line corrected, restarted after a failure (20 s, 20 s, 60 s;
    # the count resets after a day), then (re)started so it runs this command
    # line. Read back. Returns whether it changed anything.
    param([Parameter(Mandatory)][string]$BinPath, [string]$Name = $script:TunnelService)
    $changed = $false
    $current = Get-FohServicePath -Name $Name
    if ($null -eq $current) {
        New-Service -Name $Name -BinaryPathName $BinPath -DisplayName 'fohmixer tunnel (cloudflared)' -StartupType Automatic `
            -Description 'fohmixer: the Cloudflare tunnel of the public name (Install-Fohmixer.ps1, #17)' | Out-Null
        $changed = $true
    } elseif ($current -cne $BinPath) {
        $r = (Get-CimInstance -ClassName Win32_Service -Filter ("Name = '{0}'" -f $Name) |
            Invoke-CimMethod -MethodName Change -Arguments @{ PathName = $BinPath }).ReturnValue
        if ($r -ne 0) { throw "changing the $Name service's command line failed ($r)" }
        $changed = $true
    }
    & sc.exe failure $Name reset= 86400 actions= restart/20000/restart/20000/restart/60000 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "sc.exe failure $Name failed ($LASTEXITCODE)" }
    Set-Service -Name $Name -StartupType Automatic
    if ($changed) { Restart-Service -Name $Name } else { Start-Service -Name $Name }
    $after = Get-FohServicePath -Name $Name
    if ($after -cne $BinPath) { throw "the $Name service reads back as another command line" }
    return $changed
}
