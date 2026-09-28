#Requires -Version 5.1
# fohmixer S5: the install's check of Live's own User Library setting (#9),
# dot-sourced by FohmixerPc.psm1 and shipped in the bundle next to it. Live
# lists a remote script only from the User Library its preferences name
# (<profile>\AppData\Roaming\Ableton\Live <version>\Preferences\Library.cfg);
# on the PC the master user's said <UserLibrary /> and Live never listed
# FohMixer. These functions only read Live's preferences; the fix is the
# owner's, in Live. Windows PowerShell 5.1, ASCII only.

function Get-FohLivePrefsFile {
    # The Library.cfg that Live uses, from $Prefs: that file itself; a
    # "Live <version>" folder's own Preferences\Library.cfg; or, among a user's
    # Live preferences (<Prefs>\Live <version>\Preferences\Library.cfg, one
    # folder per Live version), the newest version's, by version number (12.10
    # is newer than 12.2; a folder not named "Live <version>" is not Live's).
    # The owner passes the file or the folder when a newer, unused Live left
    # its folder behind. '' when there is none.
    param([Parameter(Mandatory)][string]$Prefs)
    if (Test-Path -LiteralPath $Prefs -PathType Leaf) { return $Prefs }
    $own = Join-Path $Prefs 'Preferences\Library.cfg'
    if (Test-Path -LiteralPath $own -PathType Leaf) { return $own }
    if (-not (Test-Path -LiteralPath $Prefs -PathType Container)) { return '' }
    $best = ''
    $bestVersion = $null
    foreach ($d in @(Get-ChildItem -LiteralPath $Prefs -Directory -Force)) {
        $m = [regex]::Match($d.Name, '^Live (\d+)(\.\d+){0,3}\z')
        if (-not $m.Success) { continue }
        $text = $d.Name.Substring(5)
        if (-not $text.Contains('.')) { $text += '.0' }
        $cfg = Join-Path $d.FullName 'Preferences\Library.cfg'
        if (-not (Test-Path -LiteralPath $cfg -PathType Leaf)) { continue }
        $version = [version]$text
        if ($null -eq $bestVersion -or $version -gt $bestVersion) {
            $best = $cfg
            $bestVersion = $version
        }
    }
    return $best
}

function Get-FohLiveUserLibrary {
    # The User Library a Live Library.cfg sets: ProjectPath\ProjectName of
    # ContentLibrary/UserLibrary/LibraryProject (Live writes ProjectPath with
    # forward slashes). '' for <UserLibrary /> (no User Library set: Live then
    # lists no Remote Scripts of one, #9).
    param([Parameter(Mandatory)][string]$Cfg)
    try {
        $xml = [xml][IO.File]::ReadAllText($Cfg)
    } catch {
        throw "$Cfg is not a Library.cfg Live wrote: $($_.Exception.Message)"
    }
    $project = '/Ableton/ContentLibrary/UserLibrary/LibraryProject'
    $path = $xml.SelectSingleNode("$project/ProjectPath/@Value")
    $name = $xml.SelectSingleNode("$project/ProjectName/@Value")
    if ($null -eq $path -or $null -eq $name -or -not $path.Value -or -not $name.Value) { return '' }
    $folder = $path.Value.Replace('/', '\')
    return [IO.Path]::GetFullPath([IO.Path]::Combine($folder, $name.Value)).TrimEnd('\')
}

function Test-FohLiveUserLibrary {
    # The install's check, before anything changes, that Live uses the User
    # Library FohMixer goes into (#9: the master user's Library.cfg had
    # <UserLibrary />, so Live never listed FohMixer). Only reads Live's
    # preferences; the fix is the owner's, in Live. $Switch names the
    # install's parameters (Band or Master).
    param(
        [Parameter(Mandatory)][string]$User,
        [Parameter(Mandatory)][string]$Prefs,
        [Parameter(Mandatory)][string]$UserLibrary,
        [Parameter(Mandatory)][string]$Switch
    )
    $cfg = Get-FohLivePrefsFile -Prefs $Prefs
    if (-not $cfg) {
        throw ("no Library.cfg of Live for $User under $Prefs (Live <version>\Preferences\Library.cfg): " +
            "start Live once as $User, or pass the folder of its Live preferences with -${Switch}AbletonPrefs")
    }
    $used = Get-FohLiveUserLibrary -Cfg $cfg
    $parent = Split-Path -Parent $UserLibrary
    if (-not $used) {
        throw ("Live of $User has no User Library set (${cfg}: UserLibrary is empty), so it would not list FohMixer: " +
            "in Live as $User open Settings > Library, set the location of the User Library to $parent, " +
            'restart Live, then run the install again')
    }
    if ($used -ne $UserLibrary.TrimEnd('\')) {
        throw ("Live of $User uses the User Library $used (${cfg}), not ${UserLibrary}: pass -${Switch}UserLibrary " +
            "`"$used`", or set $parent in Live's Settings > Library and restart Live")
    }
}
