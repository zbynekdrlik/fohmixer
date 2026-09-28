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
#   Ctrl-Break): a graceful stop within 10 s, exit code 0;
# - the two tasks registered for this user in a test task folder and read back;
# - the data folder's DACL on a test folder.
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
    }
    foreach ($k in $Over.Keys) { $a[$k] = $Over[$k] }
    $list = @()
    foreach ($k in $a.Keys) { $list += ('-' + $k); $list += [string]$a[$k] }
    return , ($list + '-NoTask')
}

function Set-OldStamps([string[]]$Paths) {
    foreach ($p in $Paths) {
        $items = @(Get-Item -LiteralPath $p)
        if ($items[0].PSIsContainer) { $items = @(Get-ChildItem -LiteralPath $p -Recurse -File -Force) }
        foreach ($i in $items) { $i.LastWriteTimeUtc = $script:old }
    }
}

function Get-NewerFiles([string[]]$Paths) {
    # The files under $Paths written since Set-OldStamps.
    $out = @()
    foreach ($p in $Paths) {
        $items = @(Get-Item -LiteralPath $p)
        if ($items[0].PSIsContainer) { $items = @(Get-ChildItem -LiteralPath $p -Recurse -File -Force) }
        foreach ($i in $items) { if ($i.LastWriteTimeUtc -ne $script:old) { $out += $i.FullName } }
    }
    return $out
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
$data = Join-Path $base 'data'
$script:old = [datetime]::SpecifyKind((Get-Date '2001-01-01T00:00:00'), [DateTimeKind]::Utc)
$me = [Security.Principal.WindowsIdentity]::GetCurrent().Name
New-Item -ItemType Directory -Force -Path $bandLib, $masterLib | Out-Null

try {
    # ---- pure helpers ----
    foreach ($v in @('0.1.0', '0.1.0-dev.4', '10.20.30-rc-1.2')) { Assert (Test-FohVersion $v) "version-accepts-$v" }
    foreach ($v in @('', '0.1', '01.0.0', '0.1.0-', '0.1.0-dev.', '0.1.0-a..b', "0.1.0`n", '..\0.1.0', '0.1.0 x')) {
        Assert (-not (Test-FohVersion $v)) ('version-refuses-[' + $v.Replace("`n", '\n') + ']')
    }
    Assert ((Format-FohArg 'C:\a b\') -ceq '"C:\a b"') 'arg-quoted-without-a-trailing-backslash'
    Assert ((ErrorOf { Format-FohArg 'a"b' }) -like '*double quote*') 'arg-refuses-a-quote'
    foreach ($u in @('', ' band', 'band.', 'PC\band', 'a/b', 'a:b')) { Assert ((ErrorOf { Test-FohUserName $u }) -like '*user name refused*') "user-name-refuses-[$u]" }

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
    $refusals = @(
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
    Assert (-not (Test-Path -LiteralPath (Join-Path $bandLib 'Remote Scripts'))) 'refused-installs-wrote-no-user-copy'

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

    # ---- the real hub: the task's launcher starts it, the stop task's script stops it ----
    Assert ((Stop-FohHub -DataDir $data -TaskPath $taskFolder) -ceq 'no hub was running') 'stop-with-no-hub-running-does-nothing'
    $launcher = Start-Process -FilePath 'powershell.exe' -WindowStyle Hidden -PassThru -ArgumentList @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File',
        (Format-FohArg (Join-Path $appV1 'Start-FohmixerHub.ps1')), '-DataDir', (Format-FohArg $data))
    $null = $launcher.Handle
    $answer = Wait-FohHubReady -HttpPort 18481 -Version $v1 -TimeoutSeconds 60
    Assert ($answer.version -ceq $v1) "hub-answers-the-bundle-version-on-the-installed-toml ($($answer.version))"
    $hubs = @(Get-FohHubProcess -DataDir $data)
    Assert ($hubs.Count -eq 1 -and $hubs[0].path -eq (Join-Path $appV1 'fohmixer-hub.exe')) 'hub-runs-from-the-version-folder'
    Assert (Test-Path -LiteralPath (Join-Path $data 'secrets') -PathType Container) 'hub-keeps-its-secrets-in-the-data-folder'
    $e = ErrorOf { Stop-FohHub -DataDir $data -TaskPath $taskFolder }
    Assert ($e -like '*fohmixer-hub-stop is missing*') "stop-without-the-stop-task-refuses ($e)"
    Assert (@(Get-FohHubProcess -DataDir $data).Count -eq 1) 'stop-without-the-stop-task-leaves-the-hub-running'
    $s = Invoke-Ps (Join-Path $appV1 'Stop-FohmixerHub.ps1') @('-DataDir', $data)
    Assert ($s.code -eq 0) 'stop-script-exits-0'
    Assert (@(Wait-FohHubExit -DataDir $data -TimeoutSeconds 10).Count -eq 0) 'hub-exits-within-10-s-of-ctrl-break'
    Assert ($launcher.WaitForExit(10000)) 'launcher-exits-with-the-hub'
    Assert ($launcher.ExitCode -eq 0) "hub-and-launcher-exit-0 ($($launcher.ExitCode))"
    $hubLog = [IO.File]::ReadAllText((Join-Path $data 'logs\hub.out.log'))
    Write-Host $hubLog
    Assert ($hubLog.Contains('Ctrl-Break: stopping') -and $hubLog.Contains('fohmixer-hub stopped')) 'hub-stopped-gracefully-on-ctrl-break'
    Assert (-not $hubLog.Contains([string][char]27)) 'hub-log-without-colour-codes'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'logs\hub-stop.log')) -like "*Ctrl-Break sent to pid $($hubs[0].pid)*") 'stop-log-names-the-pid'
    Assert ([IO.File]::ReadAllText((Join-Path $data 'logs\hub-launch.log')) -like "*exited with 0*") 'launch-log-records-the-exit'

    # ---- install 2: the same bundle and parameters write nothing ----
    New-Item -ItemType Directory -Force -Path (Join-Path $bandCopy 'logs') | Out-Null
    [IO.File]::WriteAllText((Join-Path $bandCopy 'logs\FohMixer.log'), "Live's log`n")
    [IO.File]::WriteAllText((Join-Path $bandCopy 'stale.py'), "# not in the bundle`n")
    $watched = @($appV1, $bandCopy, $masterCopy, (Join-Path $data 'fohmixer-hub.toml'), (Join-Path $data 'layout.json'), (Join-Path $data 'app\current.txt'))
    Set-OldStamps $watched
    $r2 = Invoke-Ps $install (Get-InstallArgs)
    Assert ($r2.code -eq 0) 'install-2-exits-0'
    $newer = @(Get-NewerFiles $watched)
    Assert ($newer.Count -eq 0) "install-2-writes-nothing ($($newer -join ', '))"
    Assert (Test-Path -LiteralPath (Join-Path $bandCopy 'stale.py')) 'install-2-keeps-a-copy-of-the-same-version'

    # ---- install 3: another master port rewrites the master Config.py and the toml only ----
    $r3 = Invoke-Ps $install (Get-InstallArgs @{ MasterPort = 39183 })
    Assert ($r3.code -eq 0) 'install-3-exits-0'
    $newer = @(Get-NewerFiles $watched | ForEach-Object { $_.Substring($base.Length) }) | Sort-Object
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
    $ps = Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
    foreach ($n in @('fohmixer-hub', 'fohmixer-hub-stop')) {
        $t = Get-ScheduledTask -TaskPath $taskFolder -TaskName $n
        $st = $t.Settings
        Assert ("$($t.Principal.LogonType)" -eq 'Interactive' -and "$($t.Principal.RunLevel)" -eq 'Limited') "task-$n-interactive-limited"
        Assert ($st.ExecutionTimeLimit -eq 'PT0S' -and "$($st.MultipleInstances)" -eq 'IgnoreNew') "task-$n-no-time-limit-ignorenew"
        Assert (-not $st.AllowHardTerminate) "task-$n-never-ended-hard"
        Assert (-not $st.DisallowStartIfOnBatteries -and -not $st.StopIfGoingOnBatteries -and -not $st.IdleSettings.StopOnIdleEnd -and $st.Priority -eq 4) "task-$n-no-battery-or-idle-stop-normal-priority"
        Assert ($t.Actions[0].Execute -eq $ps -and $t.Actions[0].WorkingDirectory -eq $data) "task-$n-runs-windows-powershell-in-the-data-folder"
        Assert ("$($t.State)" -ne 'Running') "task-$n-not-started"
    }
    $h = Get-ScheduledTask -TaskPath $taskFolder -TaskName 'fohmixer-hub'
    $wantArgs = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "{0}" -DataDir "{1}"'
    Assert ($h.Actions[0].Arguments -ceq ($wantArgs -f (Join-Path $appV1 'Start-FohmixerHub.ps1'), $data)) "task-hub-runs-the-launcher ($($h.Actions[0].Arguments))"
    Assert (@($h.Triggers).Count -eq 1 -and $h.Triggers[0].CimClass.CimClassName -eq 'MSFT_TaskLogonTrigger' -and
        $h.Triggers[0].UserId -like ('*' + (Get-FohLeafName $me))) 'task-hub-starts-at-the-users-logon'
    Assert ($h.Settings.RestartCount -eq 3 -and $h.Settings.RestartInterval -eq 'PT1M') 'task-hub-restarts-on-failure-3x1min'
    $st = Get-ScheduledTask -TaskPath $taskFolder -TaskName 'fohmixer-hub-stop'
    Assert ($st.Actions[0].Arguments -ceq ($wantArgs -f (Join-Path $appV1 'Stop-FohmixerHub.ps1'), $data)) 'task-stop-runs-the-stop-script'
    Assert ($null -eq $st.Triggers -or @($st.Triggers).Count -eq 0) 'task-stop-has-no-trigger'
    Assert ($st.Settings.RestartCount -eq 0) 'task-stop-never-restarts'
    Register-FohHubTasks -AppDir $appV2 -DataDir $data -User $me -TaskPath $taskFolder
    Assert (@(Get-ScheduledTask -TaskPath $taskFolder).Count -eq 2) 'tasks-updated-in-place-by-the-next-install'
    $h = Get-ScheduledTask -TaskPath $taskFolder -TaskName 'fohmixer-hub'
    Assert ($h.Actions[0].Arguments -ceq ($wantArgs -f (Join-Path $appV2 'Start-FohmixerHub.ps1'), $data)) 'task-hub-runs-the-new-version'

    # ---- the data folder's DACL ----
    $aclDir = Join-Path $base 'acl'
    New-Item -ItemType Directory -Force -Path (Join-Path $aclDir 'secrets') | Out-Null
    [IO.File]::WriteAllText((Join-Path $aclDir 'secrets\key'), 'x')
    Set-FohDataDirAcl -Path $aclDir -User $me
    Assert (@(Test-FohDataDirAcl -Path $aclDir -User $me).Count -eq 0) 'dacl-reads-back'
    $below = Get-Acl -LiteralPath (Join-Path $aclDir 'secrets\key')
    $meSid = (New-Object Security.Principal.NTAccount $me).Translate([Security.Principal.SecurityIdentifier]).Value
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
    try {
        foreach ($t in @(Get-ScheduledTask | Where-Object { $_.TaskPath -eq $taskFolder })) {
            Unregister-ScheduledTask -TaskPath $taskFolder -TaskName $t.TaskName -Confirm:$false
        }
        $sch = New-Object -ComObject 'Schedule.Service'
        $sch.Connect()
        $sch.GetFolder('\').DeleteFolder($taskFolder.Trim('\'), 0)
    } catch { Write-Host "cleanup: task folder $taskFolder ($($_.Exception.Message))" }
    try { Remove-Item -LiteralPath $base -Recurse -Force } catch { Write-Host "cleanup: $base ($($_.Exception.Message))" }
}
Write-Host 'Test-Fohmixer: all passed'
