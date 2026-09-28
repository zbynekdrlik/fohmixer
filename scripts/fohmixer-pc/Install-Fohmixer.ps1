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
  1. stops the running hub: Ctrl-Break through the fohmixer-hub-stop task, then
     a wait of up to 10 s; a hub that does not stop is reported, never ended
     (spec I7);
  2. installs the bundle as <DataDir>\app\<version> and points
     <DataDir>\app\current.txt at it;
  3. writes <DataDir>\fohmixer-hub.toml (HTTP port, the band and master
     instances, layout.json);
  4. copies -Layout (the import's output) to <DataDir>\layout.json;
  5. installs FohMixer into each Live user's User Library\Remote Scripts with
     that user's INSTANCE and PORT in Config.py (replaced only when version.py
     differs); Live's preferences and a running Live are never touched;
  6. registers the tasks fohmixer-hub (at the band user's logon, Interactive,
     Limited, no time limit, IgnoreNew, never ended hard, in a console without
     a window) and fohmixer-hub-stop, and the firewall rule fohmixer-hub-http
     (TCP <HttpPort>, Domain and Private profiles);
  7. starts fohmixer-hub and polls http://127.0.0.1:<HttpPort>/api/version (up
     to 20 s) and prints it.
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
The data folder gets a protected DACL: SYSTEM, Administrators and the band user.
When a step after the stop fails, the hub task is started again.
-NoTask (the self-test) leaves out steps 1, 6, 7 and the DACL.
Errors: one "fohmixer install FAILED: ..." line on stderr, exit code 1.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File <unzipped bundle>\Install-Fohmixer.ps1 -BundleZip <bundle zip> -BandUser <band account> -MasterUser <master account> -Layout <import folder>\layout.json

.EXAMPLE
Get-Content <token file> | powershell -NoProfile -ExecutionPolicy Bypass -File <unzipped bundle>\Install-Fohmixer.ps1 -BundleZip <bundle zip> -BandUser <band account> -MasterUser <master account> -PublicName <name> -AccessTeam <team>.cloudflareaccess.com -AccessAud <aud> -SetTunnelToken
#>
[CmdletBinding()]
param(
    # The CI bundle zip (the artifact fohmixer-windows-<version>-<sha> of a master push).
    [Parameter(Mandatory)][string]$BundleZip,
    # The Windows account that runs the band Live (and the hub): a local account
    # of this PC, without a domain part (looked up as <computer>\<name>, also
    # where the account is named like the computer).
    [Parameter(Mandatory)][string]$BandUser,
    # The Windows account that runs the master Live (a local account, as -BandUser).
    [Parameter(Mandatory)][string]$MasterUser,
    [string]$DataDir = (Join-Path $env:ProgramData 'fohmixer'),
    [int]$HttpPort = 8480,
    [int]$BandPort = 39101,
    [int]$MasterPort = 39102,
    # The layout file to install (the TouchOSC import's output); none keeps the installed one.
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
    [int]$HttpsPort = 443,
    # The ACME account's contact, and another ACME directory (e.g. Let's Encrypt's staging).
    [string]$AcmeEmail = '',
    [string]$AcmeDirectory = '',
    # The Cloudflare Access application of the name: its team domain and AUD tag(s), comma separated.
    [string]$AccessTeam = '',
    [string]$AccessAud = '',
    # Read the cloudflared connector token from stdin (or a hidden prompt) and store it.
    [switch]$SetTunnelToken,
    [string]$CloudflaredExe = '',
    [int]$TunnelMetricsPort = 20241,
    # Default <ProgramData>\fohmixer-tunnel; the hosts file and the band user's desktop default to Windows' own.
    [string]$TunnelDir = '',
    [string]$HostsFile = '',
    [string]$BandDesktop = ''
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
try {
    Import-Module (Join-Path $PSScriptRoot 'FohmixerPc.psm1') -Force
    # Remote access (#17): checked, the tunnel token read, before any change.
    $remoteArgs = @{}
    foreach ($k in @('PublicName', 'HttpsPort', 'AcmeEmail', 'AcmeDirectory', 'AccessTeam', 'AccessAud', 'SetTunnelToken',
            'CloudflaredExe', 'TunnelMetricsPort', 'TunnelDir', 'HostsFile', 'BandDesktop')) {
        if ($PSBoundParameters.ContainsKey($k)) { $remoteArgs[$k] = $PSBoundParameters[$k] }
    }
    $remote = Resolve-FohRemote -DataDir $DataDir -BandUser $BandUser -HttpPort $HttpPort -BandPort $BandPort `
        -MasterPort $MasterPort @remoteArgs
    $result = Invoke-FohInstall -BundleZip $BundleZip -BandUser $BandUser -MasterUser $MasterUser -DataDir $DataDir `
        -HttpPort $HttpPort -BandPort $BandPort -MasterPort $MasterPort -Layout $Layout `
        -BandUserLibrary $BandUserLibrary -MasterUserLibrary $MasterUserLibrary `
        -BandAbletonPrefs $BandAbletonPrefs -MasterAbletonPrefs $MasterAbletonPrefs -NoTask:$NoTask -Remote $remote
    ConvertTo-Json -InputObject $result -Depth 5
} catch {
    # One unwrapped line (a 5.1 error record wraps at the console width), then where it failed.
    [Console]::Error.WriteLine('fohmixer install FAILED: ' + $_.Exception.Message)
    [Console]::Error.WriteLine($_.ScriptStackTrace)
    exit 1
}
