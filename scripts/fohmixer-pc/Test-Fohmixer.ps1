#Requires -Version 5.1
# Self-test of the S5 PC install (scripts/fohmixer-pc) on Windows PowerShell 5.1
# (CI job windows, an ephemeral administrator runner), against real backends:
# - a bundle built from this checkout (New-FohBundle) with a real hub (-HubExe,
#   the job's debug build);
# - installs with -NoTask, each as its own powershell.exe as on the PC, into a
#   temp data folder and fake users' User Library folders: files, current.txt,
#   the toml, the layout, both FohMixer copies with their Config.py; a second
#   run writes nothing, a port change rewrites one Config.py, a new version
#   replaces the copies (Live's logs stay), bad input changes nothing;
# - the real hub started by the task's launcher (Start-FohmixerHub.ps1) on the
#   installed toml and stopped by the stop task's script (Stop-FohmixerHub.ps1,
#   Ctrl-Break), both run exactly as their tasks run them (conhost --headless):
#   a graceful stop within 10 s, exit code 0, and the stop script itself exits
#   0 (not ended by its own Ctrl-Break); a failed stop prints its diagnostics
#   (the script's exit code, hub-stop.log, the result, the hubs, the hub logs);
# - the two tasks registered for this user in a test task folder and read back;
# - the firewall rule as a disabled test rule, and the data folder's DACL on a
#   test folder;
# - an account named like the computer, as on the PC (#9): a temporary local
#   user (never logged on, removed at the end) through the account check, the
#   DACL, and the tasks' principal and logon trigger with their read-back;
# - Live's own setting of each user's User Library (#9): fake Library.cfg
#   files; an install into a User Library that Live does not use is refused
#   before anything changes.
# Only its own test objects are removed; nothing is ended by force (spec I7).
param([Parameter(Mandatory)][string]$HubExe)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$repo = [IO.Path]::GetFullPath((Join-Path $here '..\..'))
# -Include is ignored next to -LiteralPath in Windows PowerShell 5.1: filter by extension.
foreach ($f in @(Get-ChildItem -LiteralPath $here -File | Where-Object { @('.ps1', '.psm1') -contains $_.Extension })) {
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$tokens, [ref]$errors)
    if (@($errors).Count -gt 0) { throw "parse errors in $($f.Name): $($errors[0].Message) (line $($errors[0].Extent.StartLineNumber))" }
    Write-Host "ok  parses: $($f.Name)"
}
Import-Module (Join-Path $here 'FohmixerPc.psm1') -Force
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Assert($cond, $what) { if (-not $cond) { throw "FAILED: $what" }; Write-Host "ok  $what" }
function ErrorOf([scriptblock]$b) { try { & $b; return '' } catch { return "$_" } }

function Invoke-Ps([string]$File, [string[]]$ArgList) {
    # A script in its own Windows PowerShell 5.1, as on the PC: exit code and output.
    $ErrorActionPreference = 'Continue'
    $said = (& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $File @ArgList 2>&1 | ForEach-Object { "$_" }) -join "`n"
    $code = $LASTEXITCODE
    Write-Host ("    [{0} exit {1}]`n    {2}" -f (Split-Path -Leaf $File), $code, ($said -replace "`n", "`n    "))
    return [pscustomobject]@{ code = $code; out = $said }
}

function Get-InstallArgs([hashtable]$Over = @{}) {
    # The self-test's install arguments, with overrides; always -NoTask.
    $a = [ordered]@{
        BundleZip = $b1.zip; BandUser = 'band-user'; MasterUser = 'master-user'; DataDir = $data
        HttpPort = 18481; BandPort = 39181; MasterPort = 39182; Layout = $layout
        BandUserLibrary = $bandLib; MasterUserLibrary = $masterLib
        BandAbletonPrefs = $bandPrefs; MasterAbletonPrefs = $masterPrefs
    }
    foreach ($k in $Over.Keys) { $a[$k] = $Over[$k] }
    $list = @()
    foreach ($k in $a.Keys) { $list += ('-' + $k); $list += [string]$a[$k] }
    return , ($list + '-NoTask')
}

function Get-StampedFiles([string[]]$Paths) {
    $out = @()
    foreach ($p in $Paths) {
        $items = @(Get-Item -LiteralPath $p)
        if ($items[0].PSIsContainer) { $items = @(Get-ChildItem -LiteralPath $p -Recurse -File -Force) }
        $out += $items
    }
    return $out
}

function Set-Stamps([hashtable]$Watched) {
    # Every file under each path gets that path's time. The app folder gets
    # another time than the copies made from it: Copy-Item keeps a file's time,
    # so a needless re-copy would otherwise go unseen.
    foreach ($p in $Watched.Keys) { foreach ($i in @(Get-StampedFiles @($p))) { $i.LastWriteTimeUtc = $Watched[$p] } }
}

function Get-ChangedFiles([hashtable]$Watched) {
    # The files under each path whose time is no longer that path's (written since Set-Stamps).
    $out = @()
    foreach ($p in $Watched.Keys) {
        foreach ($i in @(Get-StampedFiles @($p))) { if ($i.LastWriteTimeUtc -ne $Watched[$p]) { $out += $i.FullName } }
    }
    return $out
}

function Start-AsTask([string]$Script) {
    # One of the bundle's scripts started exactly as its task starts it
    # (conhost --headless, the task command); the process to wait for.
    $cmd = Get-FohTaskCommand -Script $Script -DataDir $data
    $p = Start-Process -FilePath $cmd.execute -ArgumentList $cmd.arguments -WorkingDirectory $data -PassThru
    $null = $p.Handle
    return $p
}

function Get-ScriptProcess([string]$Script) {
    # The powershell.exe that runs $Script (the task's conhost starts it), with
    # its handle held, so its exit code stays readable after it exits: conhost
    # does not pass it on.
    $deadline = (Get-Date).AddSeconds(10)
    while ($true) {
        foreach ($c in @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'powershell.exe'")) {
            if (([string]$c.CommandLine).Contains($Script)) {
                $p = Get-Process -Id ([int]$c.ProcessId)
                $null = $p.Handle
                return $p
            }
        }
        if ((Get-Date) -ge $deadline) { throw "no powershell.exe runs $Script within 10 s" }
        Start-Sleep -Milliseconds 100
    }
}

function Show-StopDiagnostics([string]$DataDir, $StopPs) {
    # What a failed stop left behind: the stop script's exit, its log and
    # result, the hubs still running, the hub's and the launcher's logs.
    Write-Host '---- stop diagnostics ----'
    if ($null -eq $StopPs) {
        Write-Host 'stop script: its process was not found'
    } elseif ($StopPs.HasExited) {
        Write-Host ('stop script: pid {0} exited with {1} (0x{1:X8}; 0xC000013A = ended by a Ctrl event)' -f $StopPs.Id, $StopPs.ExitCode)
    } else {
        Write-Host "stop script: pid $($StopPs.Id) still runs"
    }
    foreach ($n in @('hub-stop.log', 'hub-stop.result.json')) {
        $p = Join-Path $DataDir "logs\$n"
        if (Test-Path -LiteralPath $p -PathType Leaf) {
            Write-Host ('--- {0}' -f $n)
            Write-Host ([IO.File]::ReadAllText($p))
        } else {
            Write-Host ('--- {0}: none' -f $n)
        }
    }
    Write-Host ('hubs still running: [{0}]' -f ((@(Get-FohHubProcess -DataDir $DataDir) | ForEach-Object { $_.pid }) -join ', '))
    Write-Host (Get-FohLogTail -DataDir $DataDir -Lines 20)
    Write-Host '---- end of stop diagnostics ----'
}

function Assert-Copy([string]$Copy, [string]$Bundle, [string]$Instance, [int]$Port, [string]$What) {
    # A user's FohMixer copy: the bundle's files, Config.py with this user's INSTANCE and PORT.
    $cfg = [IO.File]::ReadAllText((Join-Path $Copy 'Config.py'))
    Assert ($cfg -cmatch ('(?m)^INSTANCE = "' + $Instance + '"\r?$') -and $cfg -cmatch ('(?m)^PORT = ' + $Port + '\r?$')) "$What-config-py-instance-$Instance-port-$Port"
    $other = @()
    foreach ($f in @(Get-FohRelativeFiles -Root $Bundle)) {
        if ($f.rel -ceq 'Config.py') { continue }
        $c = Join-Path $Copy $f.rel.Replace('/', '\')
        if (-not (Test-Path -LiteralPath $c -PathType Leaf) -or (Get-FohSha256 $c) -cne (Get-FohSha256 $f.full)) { $other += $f.rel }
    }
    Assert ($other.Count -eq 0) "$What-copy-holds-the-bundle-files ($($other -join ', '))"
}

function New-FakeLiveCfg([string]$Prefs, [string]$Version, [string]$UserLibrary) {
    # Live's Library.cfg of one Live version, shaped like the real file: the
    # User Library is ProjectPath (forward slashes) + ProjectName; an empty
    # $UserLibrary writes <UserLibrary /> (no User Library set, #9).
    $dir = Join-Path $Prefs "Live $Version\Preferences"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $lib = "`t`t<UserLibrary />"
    if ($UserLibrary) {
        $parent = (Split-Path -Parent $UserLibrary).Replace('\', '/')
        $name = Split-Path -Leaf $UserLibrary
        $lib = "`t`t<UserLibrary>`r`n`t`t`t<LibraryProject Id=`"0`">`r`n`t`t`t`t<ProjectLocation />`r`n" +
            "`t`t`t`t<ProjectName Value=`"$name`" />`r`n`t`t`t`t<ProjectPath Value=`"$parent`" />`r`n" +
            "`t`t`t</LibraryProject>`r`n`t`t</UserLibrary>"
    }
    $xml = "<?xml version=`"1.0`" encoding=`"UTF-8`"?>`r`n" +
        "<Ableton MajorVersion=`"5`" MinorVersion=`"12.0_12203`" SchemaChangeCount=`"3`" Creator=`"Ableton Live $Version`" Revision=`"0`">`r`n" +
        "`t<ContentLibrary>`r`n$lib`r`n`t`t<SliceInfoList />`r`n`t</ContentLibrary>`r`n</Ableton>`r`n"
    [IO.File]::WriteAllText((Join-Path $dir 'Library.cfg'), $xml)
}

$HubExe = (Resolve-Path -LiteralPath $HubExe).ProviderPath
$id = [guid]::NewGuid().ToString('N').Substring(0, 8)
$tempRoot = [IO.Path]::GetTempPath()
if ($env:RUNNER_TEMP) { $tempRoot = $env:RUNNER_TEMP }
$base = Join-Path $tempRoot ('fohmixer-pc-test-' + $id)
$taskFolder = '\fohmixer-selftest-' + $id + '\'
$sha = '0123456789abcdef0123456789abcdef01234567'
$install = Join-Path $here 'Install-Fohmixer.ps1'
$layout = Join-Path $repo 'crates\fohmixer-hub\tests\fixtures\layout-ok.json'
$bandLib = Join-Path $base 'Users\band-user\Documents\Ableton\User Library'
$masterLib = Join-Path $base 'Users\master-user\Documents\Ableton\User Library'
$bandPrefs = Join-Path $base 'Users\band-user\AppData\Roaming\Ableton'
$masterPrefs = Join-Path $base 'Users\master-user\AppData\Roaming\Ableton'
$data = Join-Path $base 'data'
$old = New-Object DateTime 2001, 1, 1, 0, 0, 0, ([DateTimeKind]::Utc)
$oldApp = $old.AddDays(1)
$fwName = 'fohmixer-selftest-' + $id
$me = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$clashFolder = '\fohmixer-selftest-' + $id + '-clash\'
$clashSid = $null
New-Item -ItemType Directory -Force -Path $bandLib, $masterLib | Out-Null
# Live uses the Library.cfg of its newest version; an older one says nothing.
New-FakeLiveCfg -Prefs $bandPrefs -Version '11.3.35' -UserLibrary ''
New-FakeLiveCfg -Prefs $bandPrefs -Version '12.2' -UserLibrary $bandLib
New-FakeLiveCfg -Prefs $masterPrefs -Version '12.2' -UserLibrary $masterLib

try {
    # ---- pure helpers ----
    foreach ($v in @('0.1.0', '0.1.0-dev.4', '10.20.30-rc-1.2')) { Assert (Test-FohVersion $v) "version-accepts-$v" }
    foreach ($v in @('', '0.1', '01.0.0', '0.1.0-', '0.1.0-dev.', '0.1.0-a..b', "0.1.0`n", '..\0.1.0', '0.1.0 x')) {
        Assert (-not (Test-FohVersion $v)) ('version-refuses-[' + $v.Replace("`n", '\n') + ']')
    }
    Assert ((Format-FohArg 'C:\a b\') -ceq '"C:\a b"') 'arg-quoted-without-a-trailing-backslash'
    Assert ((ErrorOf { Format-FohArg 'a"b' }) -like '*double quote*') 'arg-refuses-a-quote'
    foreach ($u in @('', ' band', 'band.', 'PC\band', 'a/b', 'a:b')) { Assert ((ErrorOf { Test-FohUserName $u }) -like '*user name refused*') "user-name-refuses-[$u]" }
    $meSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    Assert ((Get-FohSid $me) -ceq $meSid -and (Get-FohSid (Get-FohLeafName $me)) -ceq $meSid) 'sid-of-an-account-with-or-without-its-computer'
    Assert ((Get-FohSid 'S-1-5-18') -ceq 'S-1-5-18' -and (Get-FohSid ('no-such-account-' + $id)) -ceq '') 'sid-taken-as-it-is-or-empty-for-an-unknown-account'

    # ---- Live's Library.cfg (#9) ----
    # Live uses the Library.cfg of its newest version (by number: 12.10 is
    # newer than 12.2); the User Library is ProjectPath + ProjectName, none
    # for <UserLibrary />.
    $prefsPick = Join-Path $base 'prefs-pick'
    New-FakeLiveCfg -Prefs $prefsPick -Version '12.2' -UserLibrary $bandLib
    New-FakeLiveCfg -Prefs $prefsPick -Version '12.10' -UserLibrary $masterLib
    New-FakeLiveCfg -Prefs $prefsPick -Version '9.7.7' -UserLibrary ''
    New-Item -ItemType Directory -Force -Path (Join-Path $prefsPick 'Live 13 Beta\Preferences'), (Join-Path $prefsPick 'Live 14') | Out-Null
    Assert ((Get-FohLivePrefsFile -Prefs $prefsPick) -eq (Join-Path $prefsPick 'Live 12.10\Preferences\Library.cfg')) 'live-prefs-of-the-newest-version-by-number'
    Assert ((Get-FohLivePrefsFile -Prefs (Join-Path $base 'no-such-prefs')) -ceq '') 'live-prefs-none-without-the-folder'
    Assert ((Get-FohLiveUserLibrary -Cfg (Join-Path $prefsPick 'Live 12.2\Preferences\Library.cfg')) -eq $bandLib) 'live-user-library-is-projectpath-and-projectname'
    Assert ((Get-FohLiveUserLibrary -Cfg (Join-Path $prefsPick 'Live 9.7.7\Preferences\Library.cfg')) -ceq '') 'live-user-library-none-for-an-empty-userlibrary'
    Assert ((ErrorOf { Test-FohLiveUserLibrary -User 'u' -Prefs $prefsPick -UserLibrary $bandLib -Switch 'Band' }) -like '*uses the User Library*-BandUserLibrary*') 'live-user-library-elsewhere-names-the-switch'
    Assert ((ErrorOf { Test-FohLiveUserLibrary -User 'u' -Prefs $prefsPick -UserLibrary $masterLib -Switch 'Master' }) -ceq '') 'live-user-library-in-use-passes'

    # ---- an account named like the computer (#9) ----
    # On the PC an account's name equals the computer's. Windows looks a bare
    # name up as a domain name first, so there the bare name is the PC's
    # account domain and does not translate to the account's SID, while
    # <computer>\<name> does. The install must look its accounts up that way
    # wherever it resolves or names one: the account check, the DACL, the
    # tasks' principal and logon trigger, and the tasks' read-back. A local
    # user of that name, created here with a random password and never logged
    # on, reproduces it; it is removed at the end.
    $clash = $env:COMPUTERNAME
    $clashPassword = ConvertTo-SecureString ('Aa1!' + [guid]::NewGuid().ToString()) -AsPlainText -Force
    $clashSid = (New-LocalUser -Name $clash -Password $clashPassword -AccountNeverExpires -Description 'fohmixer self-test, removed at its end').SID.Value
    Write-Host "    [a local user named like the computer: $clashSid]"
    Assert ((Get-FohSid $clash) -ceq $clashSid) 'sid-of-an-account-named-like-the-computer-by-its-bare-name'
    Assert ((Get-FohSid ($env:COMPUTERNAME + '\' + $clash)) -ceq $clashSid) 'sid-of-an-account-named-like-the-computer-with-its-computer'
    Assert ((Get-FohLocalAccountName $clash) -ceq ($env:COMPUTERNAME + '\' + $clash) -and
        (Get-FohLocalAccountName 'PC\band') -ceq 'PC\band') 'local-account-name-qualifies-a-bare-name-only'
    Assert ((Get-FohAccountSid $clash).Value -ceq $clashSid) 'install-account-check-finds-an-account-named-like-the-computer'
    Assert ((ErrorOf { Get-FohAccountSid ('no-such-account-' + $id) }) -like "*no account no-such-account-$id on this PC*") 'install-account-check-refuses-an-unknown-account'
    $clashAcl = Join-Path $base 'acl-clash'
    New-Item -ItemType Directory -Force -Path $clashAcl | Out-Null
    Set-FohDataDirAcl -Path $clashAcl -User $clash
    Assert (@(Test-FohDataDirAcl -Path $clashAcl -User $clash).Count -eq 0) 'dacl-for-an-account-named-like-the-computer-reads-back'
    $clashRules = @([IO.Directory]::GetAccessControl($clashAcl).GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]) |
            ForEach-Object { $_.IdentityReference.Value })
    Assert ($clashRules -ccontains $clashSid) 'dacl-names-the-account-named-like-the-computer'
    Register-FohHubTasks -AppDir $here -DataDir (Join-Path $base 'data-clash') -User $clash -TaskPath $clashFolder
    $ct = Get-ScheduledTask -TaskPath $clashFolder -TaskName 'fohmixer-hub'
    Assert ((Get-FohSid ([string]$ct.Principal.UserId)) -ceq $clashSid -and (Get-FohSid ([string]$ct.Triggers[0].UserId)) -ceq $clashSid) `
        "tasks-run-as-and-start-at-the-logon-of-the-account-named-like-the-computer (read back: $($ct.Principal.UserId) / $($ct.Triggers[0].UserId))"
    $cs = Get-ScheduledTask -TaskPath $clashFolder -TaskName 'fohmixer-hub-stop'
    Assert ((Get-FohSid ([string]$cs.Principal.UserId)) -ceq $clashSid) 'stop-task-runs-as-the-account-named-like-the-computer'

    $cargo = Join-Path $base 'Cargo.toml'
    [IO.File]::WriteAllText($cargo, "[package]`nversion = `"9.9.9`"`n[workspace.package]`nversion = `"1.2.3-dev.4`"`nedition = `"2024`"`n")
    Assert ((Get-FohWorkspaceVersion -CargoToml $cargo) -ceq '1.2.3-dev.4') 'workspace-version-from-its-own-table'

    $realConfig = [IO.File]::ReadAllText((Join-Path $repo 'live-script\FohMixer\Config.py'))
    $c = Set-FohConfigText -Text $realConfig -Instance 'master' -Port 39102
    Assert ($c -cmatch '(?m)^INSTANCE = "master"\r?$' -and $c -cmatch '(?m)^PORT = 39102\r?$') 'config-sets-instance-and-port-in-the-real-config-py'
    $rest = { param($t) (($t -split "`n") | Where-Object { $_ -notmatch '^(INSTANCE|PORT)[ \t]*=' }) -join "`n" }
    Assert ((& $rest $c) -ceq (& $rest $realConfig)) 'config-keeps-every-other-line'
    $crlf = Set-FohConfigText -Text "A = 1`r`nINSTANCE = 'x'`r`nPORT = 1`r`nPORT_B = 2`r`n" -Instance 'band' -Port 39101
    Assert ($crlf -ceq "A = 1`r`nINSTANCE = `"band`"`r`nPORT = 39101`r`nPORT_B = 2`r`n") 'config-keeps-crlf-and-similar-names'
    Assert ((ErrorOf { Set-FohConfigText -Text "INSTANCE = 'x'`n" -Instance 'band' -Port 1 }) -like '*0 assignments of PORT*') 'config-refuses-a-missing-port'
    Assert ((ErrorOf { Set-FohConfigText -Text "INSTANCE = 1`nINSTANCE = 2`nPORT = 1`n" -Instance 'band' -Port 1 }) -like '*2 assignments of INSTANCE*') 'config-refuses-two-instances'
    Assert ((ErrorOf { Set-FohConfigText -Text "INSTANCE = 1`nPORT = 1`n" -Instance 'Band"' -Port 1 }) -like '*instance name refused*') 'config-refuses-a-bad-instance-name'

    $wantToml = "# fohmixer-hub configuration, written by Install-Fohmixer.ps1: run the install again to change it.`r`n" +
        "http_port = 18481`r`nlayout = `"layout.json`"`r`n`r`n[[instances]]`r`nname = `"band`"`r`nport = 39181`r`n`r`n" +
        "[[instances]]`r`nname = `"master`"`r`nport = 39182`r`n"
    Assert ((New-FohHubToml -HttpPort 18481 -BandPort 39181 -MasterPort 39182) -ceq $wantToml) 'toml-is-the-hub-config'

    # ---- the bundle ----
    $out = Join-Path $base 'out'
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    $b1 = New-FohBundle -RepoRoot $repo -HubExe $HubExe -OutDir $out -Sha $sha
    $v1 = $b1.version
    Assert ($v1 -ceq (Get-FohScriptVersion -Path (Join-Path $repo 'live-script\FohMixer\version.py'))) 'bundle-version-is-the-workspace-version'
    Assert ($b1.name -ceq "fohmixer-windows-$v1-$sha" -and (Test-Path -LiteralPath $b1.zip -PathType Leaf)) 'bundle-named-by-version-and-commit'
    $zip = [IO.Compression.ZipFile]::OpenRead($b1.zip)
    try { $entries = @($zip.Entries | ForEach-Object { $_.FullName }) } finally { $zip.Dispose() }
    foreach ($n in @('VERSION', 'SHA256SUMS', 'fohmixer-hub.exe', 'Install-Fohmixer.ps1', 'FohmixerPc.psm1', 'Start-FohmixerHub.ps1',
            'Stop-FohmixerHub.ps1', 'FohMixer/__init__.py', 'FohMixer/Config.py', 'FohMixer/version.py', 'FohMixer/transport/server.py')) {
        Assert ($entries -ccontains $n) "bundle-holds-$n"
    }
    Assert (@($entries | Where-Object { $_ -match '(^|/)(__pycache__|logs|tests)/|\.pyc$' }).Count -eq 0) 'bundle-without-tests-caches-or-logs'
    Assert (@($entries | Where-Object { $_.Contains('\') }).Count -eq 0) 'bundle-entries-use-forward-slashes'
    Assert ((Test-FohBundle -Root $b1.stage) -ceq $v1) 'bundle-reads-back-against-its-sums'

    # ---- bad input changes nothing ----
    $none = Join-Path $base 'data-refused'
    # Live does not see a User Library it is not set to (#9: the master user's
    # Library.cfg had <UserLibrary />, so Live never listed FohMixer).
    $noLibrary = Join-Path $base 'prefs-no-user-library'
    New-FakeLiveCfg -Prefs $noLibrary -Version '11.3.35' -UserLibrary $masterLib
    New-FakeLiveCfg -Prefs $noLibrary -Version '12.2' -UserLibrary ''
    $elsewhere = Join-Path $base 'prefs-elsewhere'
    New-FakeLiveCfg -Prefs $elsewhere -Version '12.2' -UserLibrary (Join-Path $base 'Users\band-user\OneDrive\Ableton\User Library')
    $noPrefs = Join-Path $base 'prefs-none'
    New-Item -ItemType Directory -Force -Path $noPrefs | Out-Null
    $refusals = @(
        @{ over = @{ MasterAbletonPrefs = $noLibrary }; says = '*has no User Library set*Settings > Library*'; what = 'a-live-user-without-a-user-library' },
        @{ over = @{ BandAbletonPrefs = $elsewhere }; says = '*uses the User Library*OneDrive*'; what = 'a-user-library-live-does-not-use' },
        @{ over = @{ BandAbletonPrefs = $noPrefs }; says = '*no Library.cfg of Live*'; what = 'a-user-without-live-preferences' },
        @{ over = @{ BandUserLibrary = (Join-Path $base 'Users\nobody\Documents\Ableton\User Library') }; says = "*Live's User Library not found*"; what = 'a-missing-user-library' },
        @{ over = @{ MasterPort = 39181 }; says = '*the ports must differ*'; what = 'two-instances-on-one-port' },
        @{ over = @{ MasterUser = 'band-user' }; says = '*two accounts*'; what = 'one-user-for-both' },
        @{ over = @{ BandUser = 'PC\band' }; says = '*user name refused*'; what = 'a-domain-user-name' },
        @{ over = @{ BundleZip = (Join-Path $base 'missing.zip') }; says = '*bundle not found*'; what = 'a-missing-bundle' })
    $badLayout = Join-Path $base 'bad-layout.json'
    [IO.File]::WriteAllText($badLayout, '{ not json')
    $refusals += @{ over = @{ Layout = $badLayout }; says = '*is not JSON*'; what = 'a-layout-that-is-not-json' }
    foreach ($r in $refusals) {
        $o = $r.over
        $o['DataDir'] = $none
        $res = Invoke-Ps $install (Get-InstallArgs $o)
        Assert ($res.code -ne 0 -and $res.out -like $r.says) "install-refuses-$($r.what)"
        Assert (-not (Test-Path -LiteralPath $none)) "install-refused-$($r.what)-before-any-change"
    }

    # A bundle changed after its sums: refused naming the file, nothing installed.
    $stage3 = Join-Path $base 'stage-tampered'
    [IO.Compression.ZipFile]::ExtractToDirectory($b1.zip, $stage3)
    Add-Content -LiteralPath (Join-Path $stage3 'FohMixer\surface.py') -Value '# changed after the sums'
    $zip3 = Join-Path $base 'tampered.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($stage3, $zip3)
    $data3 = Join-Path $base 'data-tampered'
    $res = Invoke-Ps $install (Get-InstallArgs @{ BundleZip = $zip3; DataDir = $data3 })
    Assert ($res.code -ne 0 -and $res.out -cmatch 'FohMixer/surface\.py: sha256 [0-9a-f]{64}, SHA256SUMS says [0-9a-f]{64}') 'install-refuses-a-tampered-bundle-naming-the-file'
    Assert (@(Get-ChildItem -LiteralPath (Join-Path $data3 'app') -Force).Count -eq 0 -and
        -not (Test-Path -LiteralPath (Join-Path $data3 'fohmixer-hub.toml'))) 'tampered-bundle-installs-nothing-and-leaves-no-unpack-folder'
    Assert (-not (Test-Path -LiteralPath (Join-Path $bandLib 'Remote Scripts')) -and
        -not (Test-Path -LiteralPath (Join-Path $masterLib 'Remote Scripts'))) 'refused-installs-wrote-no-user-copy'

    # ---- install 1 ----
    $r1 = Invoke-Ps $install (Get-InstallArgs)
    Assert ($r1.code -eq 0) 'install-1-exits-0'
    $appV1 = Join-Path $data "app\$v1"
    Assert (Test-FohSameTree -A $b1.stage -B $appV1) 'install-1-app-folder-is-the-bundle'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'app\current.txt')) -ceq ($appV1 + "`r`n")) 'install-1-current-txt-points-at-the-version'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'fohmixer-hub.toml')) -ceq $wantToml) 'install-1-writes-the-toml'
    Assert ((Get-FohSha256 (Join-Path $data 'layout.json')) -ceq (Get-FohSha256 $layout)) 'install-1-copies-the-layout'
    $bandCopy = Join-Path $bandLib 'Remote Scripts\FohMixer'
    $masterCopy = Join-Path $masterLib 'Remote Scripts\FohMixer'
    Assert-Copy -Copy $bandCopy -Bundle (Join-Path $appV1 'FohMixer') -Instance 'band' -Port 39181 -What 'install-1-band'
    Assert-Copy -Copy $masterCopy -Bundle (Join-Path $appV1 'FohMixer') -Instance 'master' -Port 39182 -What 'install-1-master'

    # ---- the real hub, started and stopped exactly as the tasks do it ----
    $idle = Stop-FohHub -DataDir $data -TaskPath $taskFolder
    Assert (-not $idle.stopped -and $idle.text -ceq 'no hub was running') 'stop-with-no-hub-running-does-nothing'
    $s0 = Invoke-Ps (Join-Path $appV1 'Stop-FohmixerHub.ps1') @('-DataDir', $data)
    Assert ($s0.code -eq 0 -and (Read-FohStopResult -DataDir $data -TimeoutSeconds 1).message -ceq 'no hub running') 'stop-script-with-no-hub-running-reports-code-0'
    Remove-Item -LiteralPath (Join-Path $data 'logs\hub-stop.result.json')
    $launcher = Start-AsTask (Join-Path $appV1 'Start-FohmixerHub.ps1')
    $answer = Wait-FohHubReady -HttpPort 18481 -Version $v1 -TimeoutSeconds 60
    Assert ($answer.version -ceq $v1) "hub-answers-the-bundle-version-on-the-installed-toml ($($answer.version))"
    $hubs = @(Get-FohHubProcess -DataDir $data)
    Assert ($hubs.Count -eq 1 -and $hubs[0].path -eq (Join-Path $appV1 'fohmixer-hub.exe')) 'hub-runs-from-the-version-folder'
    Assert (Test-Path -LiteralPath (Join-Path $data 'secrets') -PathType Container) 'hub-keeps-its-secrets-in-the-data-folder'
    $e = ErrorOf { Stop-FohHub -DataDir $data -TaskPath $taskFolder }
    Assert ($e -like '*fohmixer-hub-stop is missing*') "stop-without-the-stop-task-refuses ($e)"
    Assert (@(Get-FohHubProcess -DataDir $data).Count -eq 1) 'stop-without-the-stop-task-leaves-the-hub-running'
    $stopScript = Join-Path $appV1 'Stop-FohmixerHub.ps1'
    $stopper = Start-AsTask $stopScript
    $stopPs = $null
    try {
        $stopPs = Get-ScriptProcess $stopScript
        Assert ($stopPs.WaitForExit(60000)) 'stop-script-ends'
        # The script's own exit code (conhost drops it): 0xC000013A would be
        # the stop script ended by its own Ctrl-Break before it wrote a result.
        Assert ($stopPs.ExitCode -eq 0) ('stop-script-exits-0-it-is-not-a-target-of-its-own-ctrl-break (exit 0x{0:X8})' -f $stopPs.ExitCode)
        $result = Read-FohStopResult -DataDir $data -TimeoutSeconds 1
        Assert ([int]$result.code -eq 0 -and $result.message -like "Ctrl-Break sent to pid $($hubs[0].pid) *") "stop-script-in-its-own-console-reports-code-0 ($($result.message))"
        Assert ($stopper.WaitForExit(30000)) 'stop-task-console-ends'
        Assert (@(Wait-FohHubExit -DataDir $data -TimeoutSeconds 10).Count -eq 0) 'hub-exits-within-10-s-of-ctrl-break'
        Assert ($launcher.WaitForExit(15000)) 'launcher-ends-with-the-hub'
    } catch {
        Show-StopDiagnostics -DataDir $data -StopPs $stopPs
        throw
    }
    $hubLog = [IO.File]::ReadAllText((Join-Path $data 'logs\hub.out.log'))
    Write-Host $hubLog
    Assert ($hubLog.Contains('Ctrl-Break: stopping') -and $hubLog.Contains('fohmixer-hub stopped')) 'hub-stopped-gracefully-on-ctrl-break'
    Assert (-not $hubLog.Contains([string][char]27)) 'hub-log-without-colour-codes'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'logs\hub-stop.log')) -like "*Ctrl-Break sent to pid $($hubs[0].pid)*") 'stop-log-names-the-pid'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'logs\hub-launch.log')) -like "*pid $($hubs[0].pid) exited with 0*") 'hub-exit-code-0-in-the-launch-log'

    # ---- install 2: the same bundle and parameters write nothing ----
    New-Item -ItemType Directory -Force -Path (Join-Path $bandCopy 'logs') | Out-Null
    [IO.File]::WriteAllText((Join-Path $bandCopy 'logs\FohMixer.log'), "Live's log`n")
    [IO.File]::WriteAllText((Join-Path $bandCopy 'stale.py'), "# not in the bundle`n")
    $watched = @{}
    $watched[$appV1] = $oldApp
    foreach ($p in @($bandCopy, $masterCopy, (Join-Path $data 'fohmixer-hub.toml'), (Join-Path $data 'layout.json'), (Join-Path $data 'app\current.txt'))) {
        $watched[$p] = $old
    }
    Set-Stamps $watched
    $r2 = Invoke-Ps $install (Get-InstallArgs)
    Assert ($r2.code -eq 0) 'install-2-exits-0'
    $newer = @(Get-ChangedFiles $watched)
    Assert ($newer.Count -eq 0) "install-2-writes-nothing ($($newer -join ', '))"
    Assert (Test-Path -LiteralPath (Join-Path $bandCopy 'stale.py')) 'install-2-keeps-a-copy-of-the-same-version'

    # ---- install 3: another master port rewrites the master Config.py and the toml only ----
    $r3 = Invoke-Ps $install (Get-InstallArgs @{ MasterPort = 39183 })
    Assert ($r3.code -eq 0) 'install-3-exits-0'
    $newer = @(Get-ChangedFiles $watched | ForEach-Object { $_.Substring($base.Length) }) | Sort-Object
    $wantNewer = @(
        (Join-Path $masterCopy 'Config.py').Substring($base.Length),
        (Join-Path $data 'fohmixer-hub.toml').Substring($base.Length)) | Sort-Object
    Assert (($newer -join ',') -ceq ($wantNewer -join ',')) "install-3-writes-the-master-config-py-and-the-toml-only ($($newer -join ', '))"
    Assert-Copy -Copy $masterCopy -Bundle (Join-Path $appV1 'FohMixer') -Instance 'master' -Port 39183 -What 'install-3-master'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'fohmixer-hub.toml')) -like '*port = 39183*') 'install-3-toml-has-the-new-port'

    # ---- install 4: a new version replaces both copies; Live's logs stay ----
    $v2 = '9.9.9-selftest.2'
    $stage2 = Join-Path $base 'stage-v2'
    [IO.Compression.ZipFile]::ExtractToDirectory($b1.zip, $stage2)
    [IO.File]::WriteAllText((Join-Path $stage2 'VERSION'), "$v2`n")
    $vp = Join-Path $stage2 'FohMixer\version.py'
    [IO.File]::WriteAllText($vp, [regex]::Replace([IO.File]::ReadAllText($vp), '(?m)^VERSION[ \t]*=[^\r\n]*', "VERSION = `"$v2`""))
    $zip2 = Join-Path $base "fohmixer-windows-$v2-$sha.zip"
    New-FohBundleZip -Stage $stage2 -Zip $zip2
    $r4 = Invoke-Ps $install (Get-InstallArgs @{ BundleZip = $zip2; MasterPort = 39183 })
    Assert ($r4.code -eq 0) 'install-4-exits-0'
    $appV2 = Join-Path $data "app\$v2"
    Assert (Test-FohSameTree -A $stage2 -B $appV2) 'install-4-app-folder-is-the-new-bundle'
    Assert (Test-Path -LiteralPath (Join-Path $appV1 'fohmixer-hub.exe')) 'install-4-keeps-the-earlier-version'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'app\current.txt')) -ceq ($appV2 + "`r`n")) 'install-4-current-txt-points-at-the-new-version'
    Assert-Copy -Copy $bandCopy -Bundle (Join-Path $appV2 'FohMixer') -Instance 'band' -Port 39181 -What 'install-4-band'
    Assert-Copy -Copy $masterCopy -Bundle (Join-Path $appV2 'FohMixer') -Instance 'master' -Port 39183 -What 'install-4-master'
    Assert (-not (Test-Path -LiteralPath (Join-Path $bandCopy 'stale.py'))) 'install-4-removes-a-file-the-bundle-dropped'
    Assert ([IO.File]::ReadAllText((Join-Path $bandCopy 'logs\FohMixer.log')) -ceq "Live's log`n") 'install-4-keeps-lives-logs'

    # ---- the tasks, registered for this user in a test folder (never started) ----
    Register-FohHubTasks -AppDir $appV1 -DataDir $data -User $me -TaskPath $taskFolder
    $system = Join-Path $env:SystemRoot 'System32'
    $conhost = Join-Path $system 'conhost.exe'
    $ps = Join-Path $system 'WindowsPowerShell\v1.0\powershell.exe'
    foreach ($n in @('fohmixer-hub', 'fohmixer-hub-stop')) {
        $t = Get-ScheduledTask -TaskPath $taskFolder -TaskName $n
        $st = $t.Settings
        Assert ("$($t.Principal.LogonType)" -eq 'Interactive' -and "$($t.Principal.RunLevel)" -eq 'Limited') "task-$n-interactive-limited"
        Assert ($st.ExecutionTimeLimit -eq 'PT0S' -and "$($st.MultipleInstances)" -eq 'IgnoreNew') "task-$n-no-time-limit-ignorenew"
        Assert (-not $st.AllowHardTerminate) "task-$n-never-ended-hard"
        Assert (-not $st.DisallowStartIfOnBatteries -and -not $st.StopIfGoingOnBatteries -and -not $st.IdleSettings.StopOnIdleEnd -and $st.Priority -eq 4) "task-$n-no-battery-or-idle-stop-normal-priority"
        Assert ($t.Actions[0].Execute -eq $conhost -and $t.Actions[0].WorkingDirectory -eq $data) "task-$n-runs-in-a-headless-console-in-the-data-folder"
        Assert ("$($t.State)" -ne 'Running') "task-$n-not-started"
    }
    $h = Get-ScheduledTask -TaskPath $taskFolder -TaskName 'fohmixer-hub'
    $wantArgs = '--headless "{0}" -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "{1}" -DataDir "{2}"'
    Assert ($h.Actions[0].Arguments -ceq ($wantArgs -f $ps, (Join-Path $appV1 'Start-FohmixerHub.ps1'), $data)) "task-hub-runs-the-launcher ($($h.Actions[0].Arguments))"
    Assert (@($h.Triggers).Count -eq 1 -and $h.Triggers[0].CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' -and
        (Get-FohSid ([string]$h.Triggers[0].UserId)) -ceq $meSid) 'task-hub-starts-at-the-users-logon'
    Assert ($h.Settings.RestartCount -eq 3 -and $h.Settings.RestartInterval -eq 'PT1M') 'task-hub-restarts-after-a-failed-start-3x1min'
    $st = Get-ScheduledTask -TaskPath $taskFolder -TaskName 'fohmixer-hub-stop'
    Assert ($st.Actions[0].Arguments -ceq ($wantArgs -f $ps, (Join-Path $appV1 'Stop-FohmixerHub.ps1'), $data)) 'task-stop-runs-the-stop-script'
    Assert ($null -eq $st.Triggers -or @($st.Triggers).Count -eq 0) 'task-stop-has-no-trigger'
    Assert ($st.Settings.RestartCount -eq 0) 'task-stop-never-restarts'
    Register-FohHubTasks -AppDir $appV2 -DataDir $data -User $me -TaskPath $taskFolder
    Assert (@(Get-ScheduledTask -TaskPath $taskFolder).Count -eq 2) 'tasks-updated-in-place-by-the-next-install'
    $h = Get-ScheduledTask -TaskPath $taskFolder -TaskName 'fohmixer-hub'
    Assert ($h.Actions[0].Arguments -ceq ($wantArgs -f $ps, (Join-Path $appV2 'Start-FohmixerHub.ps1'), $data)) 'task-hub-runs-the-new-version'

    # ---- the firewall rule, as a disabled test rule ----
    Assert (Set-FohFirewallRule -Port 18481 -Name $fwName -Disabled) 'firewall-rule-created'
    $fr = Get-NetFirewallRule -Name $fwName
    $pf = $fr | Get-NetFirewallPortFilter
    Assert ("$($fr.Direction)" -eq 'Inbound' -and "$($fr.Action)" -eq 'Allow' -and "$($fr.Enabled)" -eq 'False' -and
        "$($pf.Protocol)" -eq 'TCP' -and "$(@($pf.LocalPort))" -eq '18481') 'firewall-rule-inbound-tcp-on-the-http-port'
    Assert ((((@("$($fr.Profile)" -split ',\s*') | Sort-Object)) -join ',') -eq 'Domain,Private') 'firewall-rule-on-the-domain-and-private-profiles'
    Assert (-not (Set-FohFirewallRule -Port 18481 -Name $fwName -Disabled)) 'firewall-rule-second-run-changes-nothing'
    Set-NetFirewallRule -Name $fwName -LocalPort 18482
    Assert ((Set-FohFirewallRule -Port 18481 -Name $fwName -Disabled) -and "$(@(($fr | Get-NetFirewallPortFilter).LocalPort))" -eq '18481') 'firewall-rule-drift-is-repaired'

    # ---- the data folder's DACL ----
    $aclDir = Join-Path $base 'acl'
    New-Item -ItemType Directory -Force -Path (Join-Path $aclDir 'secrets') | Out-Null
    [IO.File]::WriteAllText((Join-Path $aclDir 'secrets\key'), 'x')
    Set-FohDataDirAcl -Path $aclDir -User $me
    Assert (@(Test-FohDataDirAcl -Path $aclDir -User $me).Count -eq 0) 'dacl-reads-back'
    $below = Get-Acl -LiteralPath (Join-Path $aclDir 'secrets\key')
    $sids = @($below.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]) | Where-Object { $_.IsInherited } |
            ForEach-Object { $_.IdentityReference.Value } | Sort-Object)
    Assert (($sids -join ',') -ceq ((@('S-1-5-18', 'S-1-5-32-544', $meSid) | Sort-Object) -join ',')) "dacl-inherited-by-the-files-below ($($sids -join ','))"
    $acl = [IO.Directory]::GetAccessControl($aclDir)
    $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule((New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-545'),
        [Security.AccessControl.FileSystemRights]::ReadAndExecute, [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
        [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow)))
    [IO.Directory]::SetAccessControl($aclDir, $acl)
    Assert ((@(Test-FohDataDirAcl -Path $aclDir -User $me) -join '; ') -like '*a rule for S-1-5-32-545*') 'dacl-read-back-reports-another-users-rule'
} finally {
    # A hub still running after a failure above is asked to stop once more, never ended.
    try {
        if (Test-Path -LiteralPath $data) {
            if (@(Get-FohHubProcess -DataDir $data).Count -gt 0) {
                $ErrorActionPreference = 'Continue'
                & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $here 'Stop-FohmixerHub.ps1') -DataDir $data
                $left = @(Wait-FohHubExit -DataDir $data -TimeoutSeconds 10)
                if ($left.Count -gt 0) { Write-Host "cleanup: a hub still runs (pid $($left[0].pid)); not ended by force" }
                $ErrorActionPreference = 'Stop'
            }
        }
    } catch { Write-Host "cleanup: hub ($($_.Exception.Message))" }
    foreach ($tf in @($taskFolder, $clashFolder)) {
        try {
            $inFolder = @(Get-ScheduledTask | Where-Object { $_.TaskPath -eq $tf })
            foreach ($t in $inFolder) {
                Unregister-ScheduledTask -TaskPath $tf -TaskName $t.TaskName -Confirm:$false
            }
            if ($inFolder.Count -gt 0) {
                $sch = New-Object -ComObject 'Schedule.Service'
                $sch.Connect()
                $sch.GetFolder('\').DeleteFolder($tf.Trim('\'), 0)
            }
        } catch { Write-Host "cleanup: task folder $tf ($($_.Exception.Message))" }
    }
    try {
        if ($null -ne (Get-FohFirewallRule -Name $fwName)) { Remove-NetFirewallRule -Name $fwName }
    } catch { Write-Host "cleanup: firewall rule $fwName ($($_.Exception.Message))" }
    try {
        if ($clashSid) { Remove-LocalUser -SID (New-Object Security.Principal.SecurityIdentifier $clashSid) }
    } catch { Write-Host "cleanup: the local user $clashSid ($($_.Exception.Message))" }
    try { Remove-Item -LiteralPath $base -Recurse -Force } catch { Write-Host "cleanup: $base ($($_.Exception.Message))" }
}
Write-Host 'Test-Fohmixer: all passed'
