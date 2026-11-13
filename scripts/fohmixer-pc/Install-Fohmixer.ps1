#Requires -Version 5.1
<#
.SYNOPSIS
Installs or updates fohmixer on the Ableton PC from a CI bundle (S5 design note,
docs/superpowers/specs/2026-09-28-s5-deploy-design.md, section 3).

.DESCRIPTION
Run it elevated, in Windows PowerShell 5.1, from the unzipped bundle, with the
bundle zip itself as -BundleZip. It is idempotent: a second run with the same
bundle and parameters writes nothing (it does stop and start the hub again).
Every parameter is checked before anything changes, and the bundle (against
its SHA256SUMS) before the hub is stopped. That includes each Live user's own
setting of the User Library: the newest Live version's Library.cfg must name
the User Library FohMixer goes into, else the install is refused with the fix
to make in Live (it never edits Live's preferences). Then, in order:
  1. stops the running tray (#39): a second start with --exit through the
     fohmixer-tray-stop task, then a wait of up to 10 s; then the running hub:
     Ctrl-Break through the fohmixer-hub-stop task, then a wait of up to 10 s;
     one that does not stop is reported, never ended (spec I7);
  2. installs the bundle as <DataDir>\app\<version> and points
     <DataDir>\app\current.txt at it;
  3. writes <DataDir>\fohmixer-hub.toml (HTTP port, the band and master
     instances, layout.json; an [eq] table the installed file has before
     [tls], the Pro-Q 4 screen turned on by hand, is kept after the
     instances; a first install writes none, the screen stays off);
  4. copies -Layout (the import's output) to <DataDir>\layout.json;
  5. installs FohMixer into each Live user's User Library\Remote Scripts with
     that user's INSTANCE and PORT in Config.py (replaced only when version.py
     differs), and the two Max for Live devices (#151, FohMixer LUFS.amxd
     and FohMixer Meter.amxd) into each User Library\Presets\Audio
     Effects\Max Audio Effect (written only when they differ); Live's
     preferences and a running Live are never touched;
  6. registers the tasks fohmixer-hub (at the band user's logon, Interactive,
     Limited, no time limit, IgnoreNew, never ended hard, in a console without
     a window) and fohmixer-hub-stop, the tasks fohmixer-tray (the tray at the
     band user's logon, the same settings) and fohmixer-tray-stop, and the
     firewall rule fohmixer-hub-http (TCP <HttpPort>, Domain and Private
     profiles);
  7. starts fohmixer-hub and polls http://127.0.0.1:<HttpPort>/api/version (up
     to 20 s) and prints it, then starts fohmixer-tray and waits for the tray
     to run (its icon: the hub's state, Open fohmixer, Copy URL).
Remote access (#17, FohmixerRemote.ps1), only with -PublicName: the toml gets
[tls] (HTTPS on -HttpsPort), [acme] (Let's Encrypt by DNS-01; set the
Cloudflare API token afterwards AS THE BAND USER: fohmixer-hub cloudflare
set-token, token on stdin), [access] with -AccessTeam/-AccessAud, and [tunnel]
when the tunnel is set up; the hosts file maps the name to 127.0.0.1 (a marked
block); the band user's desktop gets a shortcut to https://<name>/; the
firewall rule fohmixer-hub-https opens -HttpsPort. -SetTunnelToken reads the
cloudflared connector token from stdin (pipe it in) or a hidden prompt, never
from the command line, into <TunnelDir>\tunnel-token (SYSTEM and
Administrators only), and the service fohmixer-tunnel runs cloudflared
(--protocol http2, metrics on 127.0.0.1:<TunnelMetricsPort>) with it; a later
run without -SetTunnelToken keeps the stored token. A run without -PublicName
keeps the remote access an earlier run set up (its toml tables as installed;
the hosts block, shortcut, firewall rule and service untouched).
AbleSet's own page (#90), with -PublicName: -AbleSetName writes [ableset]
after the other remote tables (the hub serves that name beside the public
one and checks it is the same site) and maps it to 127.0.0.1 in the same
hosts block; -AbleSetAccessAud is the AbleSet Access application's AUD
tag(s), comma separated, in -AccessTeam's team (none: the LAN only, said on
one line); -AbleSetTitle is the tab's title, written only when it is not
the hub's default (AbleSet). A run with -PublicName and without
-AbleSetName removes [ableset]; one without -PublicName keeps it.
iemmixer's own page (#166), with -PublicName: -IemUrl writes [iem] last, the
page's https address (iemmixer serves it and allows the frame itself; the hub
checks the address); -IemTitle is the tab's title, written only when it is not
the hub's default (IEM MIXER). A run with -PublicName and without -IemUrl
removes [iem]; one without -PublicName keeps it.
The Stream Deck tab (#52), only with -CompanionHost: the toml gets [companion]
(Bitfocus Companion's Satellite API as host or host:port, default port 16622);
the hub registers there as one more Stream Deck ("fohmixer"). A run without
-CompanionHost keeps an installed [companion] table as it is.
Presenter's stage tab (#90), only with -PresenterUrl: the toml gets [presenter]
(Presenter's server on the LAN as http://<address>[:port]; the hub checks it).
A run without -PresenterUrl keeps an installed [presenter] table as it is.
The MBC Live through its link (#184), only with -MbcLink: the toml gets the
linked instance mbc after band and master (ws://<MBC LAN address>:39201). A
run without -MbcLink keeps an installed linked instance as it is.
-Site Mbc (#184) installs the MBC PC instead: the hub runs as -MbcUser; the
toml has one instance mbc on -MbcPort (39103) and [link] on -LinkListen
(checked by the new hub: never every address), the text from [tls] on kept;
FohMixer goes into Live's built-in -ScriptFolder (it must hold Live's
_Framework) with INSTANCE mbc, the port and LOG_DIR <DataDir>\logs, a copy
it replaces kept under <DataDir>\script-backups\<UTC stamp>; a first install
takes the bundle's mbc-frame.json as layout.json; the firewall rule
fohmixer-hub-link opens the link's port. The FOH parameters are refused
there, and the MBC ones on the FOH PC. Selecting FohMixer in a free
control-surface slot of the MBC Live is a step in Live.
The data folder gets a protected DACL: SYSTEM, Administrators and the hub's user.
When a step after the stop fails, the hub and tray tasks that ran are started again.
-NoTask (the self-test) leaves out steps 1, 6, 7 and the DACL.
Errors: one "fohmixer install FAILED: ..." line on stderr, exit code 1.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File <unzipped bundle>\Install-Fohmixer.ps1 -BundleZip <bundle zip> -BandUser <band account> -MasterUser <master account> -Layout <import folder>\layout.json

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File <unzipped bundle>\Install-Fohmixer.ps1 -Site Mbc -BundleZip <bundle zip> -MbcUser <MBC account> -LinkListen <MBC LAN address>:39201 -ScriptFolder "<Live's folder>\Resources\MIDI Remote Scripts"

.EXAMPLE
Get-Content <token file> | powershell -NoProfile -ExecutionPolicy Bypass -File <unzipped bundle>\Install-Fohmixer.ps1 -BundleZip <bundle zip> -BandUser <band account> -MasterUser <master account> -PublicName <name> -AccessTeam <team>.cloudflareaccess.com -AccessAud <aud> -SetTunnelToken
#>
[CmdletBinding()]
param(
    # The CI bundle zip (the artifact fohmixer-windows-<version>-<sha> of a master push).
    [Parameter(Mandatory)][string]$BundleZip,
    # Which PC (#184): Foh (the default: the band and master Lives) or Mbc
    # (the MBC Live, its hub and its instance link).
    [ValidateSet('Foh', 'Mbc')][string]$Site = 'Foh',
    # FOH: the Windows account that runs the band Live (and the hub): a local
    # account of this PC, without a domain part (looked up as
    # <computer>\<name>, also where the account is named like the computer).
    [string]$BandUser = '',
    # FOH: the Windows account that runs the master Live (a local account, as -BandUser).
    [string]$MasterUser = '',
    # FOH (#184): the MBC hub's instance link, ws://<MBC LAN address>:39201;
    # without it an installed linked instance is kept.
    [string]$MbcLink = '',
    # MBC (#184): the Windows account that runs the MBC Live (and the hub there).
    [string]$MbcUser = '',
    # MBC: the instance link's LAN address and port, <MBC LAN address>:39201
    # (never every address: the hub refuses 0.0.0.0).
    [string]$LinkListen = '',
    # MBC: the MBC Live's FohMixer script port.
    [int]$MbcPort = 39103,
    # MBC: Live's built-in MIDI Remote Scripts folder (in Live's Resources).
    [string]$ScriptFolder = '',
    [string]$DataDir = (Join-Path $env:ProgramData 'fohmixer'),
    [int]$HttpPort = 8480,
    [int]$BandPort = 39101,
    [int]$MasterPort = 39102,
    # The layout file to install (the TouchOSC import's output); none keeps the installed one
    # (on the MBC PC a first install takes the bundle's mbc-frame.json).
    [string]$Layout = '',
    # Live's User Library of each user, when not <SystemDrive>\Users\<user>\Documents\Ableton\User Library.
    [string]$BandUserLibrary = '',
    [string]$MasterUserLibrary = '',
    # Each user's Live preferences, only read, to check Live uses the User Library: the folder
    # holding "Live <version>" folders (the newest version's Library.cfg counts; default
    # <SystemDrive>\Users\<user>\AppData\Roaming\Ableton), one "Live <version>" folder, or its Library.cfg.
    [string]$BandAbletonPrefs = '',
    [string]$MasterAbletonPrefs = '',
    # The self-test: no stop, no DACL, no tasks, no start, no readiness poll.
    [switch]$NoTask,
    # Remote access (#17): the one public name (LAN and tunnel); nothing below is used without it
    # (FohmixerRemote.ps1 Resolve-FohRemote, which also holds the defaults).
    [string]$PublicName = '',
    # Default 443 (the defaults of these live in Resolve-FohRemote; only given ones are passed).
    [int]$HttpsPort,
    # The ACME account's contact, and another ACME directory (e.g. Let's Encrypt's staging).
    [string]$AcmeEmail = '',
    [string]$AcmeDirectory = '',
    # The Cloudflare Access application of the name: its team domain and AUD tag(s), comma separated.
    [string]$AccessTeam = '',
    [string]$AccessAud = '',
    # Read the cloudflared connector token from stdin (or a hidden prompt) and store it.
    [switch]$SetTunnelToken,
    [string]$CloudflaredExe = '',
    # Default 20241.
    [int]$TunnelMetricsPort,
    # Default <ProgramData>\fohmixer-tunnel; the hosts file and the band user's desktop default to Windows' own.
    [string]$TunnelDir = '',
    [string]$HostsFile = '',
    [string]$BandDesktop = '',
    # AbleSet's own page (#90), with -PublicName: its name beside the public name, the AbleSet
    # Access application's AUD tag(s), comma separated (none: the LAN only), and the tab's title.
    [string]$AbleSetName = '',
    [string]$AbleSetAccessAud = '',
    [string]$AbleSetTitle = '',
    # iemmixer's own page (#166), with -PublicName: its https address and the tab's title.
    [string]$IemUrl = '',
    [string]$IemTitle = '',
    # The Stream Deck tab (#52): Bitfocus Companion's Satellite API as host or host:port
    # (default port 16622). Without it an installed [companion] table is kept.
    [string]$CompanionHost = '',
    # Presenter's stage tab (#90): Presenter's server as http://<LAN address>[:port].
    # Without it an installed [presenter] table is kept.
    [string]$PresenterUrl = ''
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
try {
    Import-Module (Join-Path $PSScriptRoot 'FohmixerPc.psm1') -Force
    # Each PC's own parameters (#184): the other PC's are refused, the needed ones asked for.
    $given = @($PSBoundParameters.Keys)
    $mbcOnly = @('MbcUser', 'LinkListen', 'MbcPort', 'ScriptFolder')
    $fohOnly = @('BandUser', 'MasterUser', 'BandPort', 'MasterPort', 'BandUserLibrary', 'MasterUserLibrary', 'BandAbletonPrefs',
        'MasterAbletonPrefs', 'MbcLink', 'PublicName', 'HttpsPort', 'AcmeEmail', 'AcmeDirectory', 'AccessTeam', 'AccessAud',
        'SetTunnelToken', 'CloudflaredExe', 'TunnelMetricsPort', 'TunnelDir', 'HostsFile', 'BandDesktop', 'AbleSetName',
        'AbleSetAccessAud', 'AbleSetTitle', 'IemUrl', 'IemTitle', 'CompanionHost', 'PresenterUrl')
    if ($Site -eq 'Mbc') {
        Test-FohSiteArgs -Site $Site -Given $given -Others $fohOnly -Where 'FOH PC'
        foreach ($k in @('MbcUser', 'LinkListen', 'ScriptFolder')) {
            if (-not $PSBoundParameters[$k]) { throw "-Site Mbc needs -$k" }
        }
        $result = Invoke-FohInstall -Site Mbc -BundleZip $BundleZip -MbcUser $MbcUser -DataDir $DataDir -HttpPort $HttpPort -BandPort $BandPort -MasterPort $MasterPort `
            -MbcPort $MbcPort -LinkListen $LinkListen -ScriptFolder $ScriptFolder -Layout $Layout -NoTask:$NoTask
        ConvertTo-Json -InputObject $result -Depth 5
        exit 0
    }
    Test-FohSiteArgs -Site $Site -Given $given -Others $mbcOnly -Where 'MBC PC'
    foreach ($k in @('BandUser', 'MasterUser')) {
        if (-not $PSBoundParameters[$k]) { throw "-Site Foh needs -$k" }
    }
    # Remote access (#17): checked, the tunnel token read, before any change.
    $remoteArgs = @{}
    foreach ($k in @('PublicName', 'HttpsPort', 'AcmeEmail', 'AcmeDirectory', 'AccessTeam', 'AccessAud', 'SetTunnelToken',
            'CloudflaredExe', 'TunnelMetricsPort', 'TunnelDir', 'HostsFile', 'BandDesktop', 'AbleSetName', 'AbleSetAccessAud',
            'AbleSetTitle', 'IemUrl', 'IemTitle')) {
        if ($PSBoundParameters.ContainsKey($k)) { $remoteArgs[$k] = $PSBoundParameters[$k] }
    }
    $remote = Resolve-FohRemote -DataDir $DataDir -BandUser $BandUser -HttpPort $HttpPort -BandPort $BandPort `
        -MasterPort $MasterPort @remoteArgs
    # The Stream Deck tab (#52): checked before any change, kept when not given.
    $companion = Resolve-FohCompanion -DataDir $DataDir -CompanionHost $CompanionHost
    # Presenter's stage tab (#90): checked before any change, kept when not given.
    $presenter = Resolve-FohPresenter -DataDir $DataDir -PresenterUrl $PresenterUrl
    # The Pro-Q 4 screen (#71): an installed [eq] table is kept as it is.
    $eq = Resolve-FohEq -DataDir $DataDir
    # The MBC Live through its link (#184): checked before any change, kept when not given.
    $linked = Resolve-FohMbcLink -DataDir $DataDir -MbcLink $MbcLink
    $result = Invoke-FohInstall -BundleZip $BundleZip -BandUser $BandUser -MasterUser $MasterUser -DataDir $DataDir `
        -HttpPort $HttpPort -BandPort $BandPort -MasterPort $MasterPort -Layout $Layout `
        -BandUserLibrary $BandUserLibrary -MasterUserLibrary $MasterUserLibrary `
        -BandAbletonPrefs $BandAbletonPrefs -MasterAbletonPrefs $MasterAbletonPrefs -NoTask:$NoTask -Companion $companion `
        -Eq $eq -Presenter $presenter -Remote $remote -Linked $linked
    ConvertTo-Json -InputObject $result -Depth 5
} catch {
    # One unwrapped line (a 5.1 error record wraps at the console width), then where it failed.
    [Console]::Error.WriteLine('fohmixer install FAILED: ' + $_.Exception.Message)
    [Console]::Error.WriteLine($_.ScriptStackTrace)
    exit 1
}
