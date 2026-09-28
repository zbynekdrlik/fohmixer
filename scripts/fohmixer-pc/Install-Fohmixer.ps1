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
The data folder gets a protected DACL: SYSTEM, Administrators and the band user.
When a step after the stop fails, the hub task is started again.
-NoTask (the self-test) leaves out steps 1, 6, 7 and the DACL.
Errors: one "fohmixer install FAILED: ..." line on stderr, exit code 1.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File <unzipped bundle>\Install-Fohmixer.ps1 -BundleZip <bundle zip> -BandUser <band account> -MasterUser <master account> -Layout <import folder>\layout.json
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
    # The folder of each user's Live preferences (Live <version>\Preferences\Library.cfg), when not
    # <SystemDrive>\Users\<user>\AppData\Roaming\Ableton: only read, to check Live uses the User Library.
    [string]$BandAbletonPrefs = '',
    [string]$MasterAbletonPrefs = '',
    # The self-test: no stop, no DACL, no tasks, no start, no readiness poll.
    [switch]$NoTask
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
try {
    Import-Module (Join-Path $PSScriptRoot 'FohmixerPc.psm1') -Force
    $result = Invoke-FohInstall -BundleZip $BundleZip -BandUser $BandUser -MasterUser $MasterUser -DataDir $DataDir `
        -HttpPort $HttpPort -BandPort $BandPort -MasterPort $MasterPort -Layout $Layout `
        -BandUserLibrary $BandUserLibrary -MasterUserLibrary $MasterUserLibrary `
        -BandAbletonPrefs $BandAbletonPrefs -MasterAbletonPrefs $MasterAbletonPrefs -NoTask:$NoTask
    ConvertTo-Json -InputObject $result -Depth 5
} catch {
    # One unwrapped line (a 5.1 error record wraps at the console width), then where it failed.
    [Console]::Error.WriteLine('fohmixer install FAILED: ' + $_.Exception.Message)
    [Console]::Error.WriteLine($_.ScriptStackTrace)
    exit 1
}
