#Requires -Version 5.1
# Self-test of the MBC PC's install and the FOH install's link (#184 PR D,
# FohmixerMbc.ps1), dot-sourced by Test-Fohmixer.ps1 inside its try (its
# helpers and variables: Assert, ErrorOf, Invoke-Ps, Get-InstallArgs,
# Get-StampedFiles, Set-Stamps, Get-ChangedFiles, $install, $b1, $base,
# $HubExe, $checkDir, $dataR, $old). Covered:
# - the linked instance's table (-MbcLink) on the FOH install: written after
#   the band and master instances, read back from an installed toml (never
#   after [tls]), checked by the hub, kept by an update without -MbcLink, and
#   a table written by hand at the end (the FOH PC's [autotune]) kept by both;
# - the MBC hub's toml (-Site Mbc): one instance and [link], none of the FOH
#   tables, checked by the hub, which refuses an unspecified -LinkListen
#   before anything changes;
# - the FohMixer copy in Live's built-in MIDI Remote Scripts folder (a fake
#   one): Config.py with INSTANCE mbc, the port and the log folder in the
#   data folder; a copy it replaces kept, dated, under the data folder; a
#   folder that is not Live's refused;
# - the layout: the bundle's MBC frame only when none is installed;
# - the parameters of one site refused on the other;
# - the app window (#280): the [tray] table (literal strings, after [link],
#   before the kept tables), checked by the hub; the shortcut's argument
#   line; Brave and the Startup folder refused before any change when
#   missing; the Startup shortcut "MBC mixer" written, kept by an install
#   that changes nothing, rewritten for another profile.
Set-StrictMode -Version Latest

# ---- the linked instance's table (#184) ----
$link = 'ws://10.0.0.5:39201'
$wantLinked = "`r`n[[instances]]`r`nname = `"mbc`"`r`nlink = `"$link`"`r`n"
Assert ((Get-FohLinkedToml -Link $link) -ceq $wantLinked) 'mbc-linked-toml'
Assert ((Get-FohLinkedToml -Link '') -ceq '') 'mbc-linked-toml-empty-without-a-link'
foreach ($bad in @('ws://10.0.0.5 :39201', 'ws://"x":1', 'http://10.0.0.5:39201', 'ws://a\b:1')) {
    Assert ((ErrorOf { Get-FohLinkedToml -Link $bad }) -like '*MBC link refused*') "mbc-linked-toml-refuses-[$bad]"
}
$withLinked = New-FohHubToml -HttpPort 1 -BandPort 2 -MasterPort 3 -Linked $wantLinked -Eq "`r`n[eq]`r`nbackend = `"windows`"`r`n"
Assert ($withLinked.Contains("port = 3`r`n" + $wantLinked + "`r`n[eq]")) 'mbc-linked-instance-after-the-local-ones-before-eq'
Assert ((Get-FohInstalledLinkedToml -Text $withLinked) -ceq $wantLinked) 'mbc-installed-linked-instance-read-back'
Assert ((Get-FohInstalledLinkedToml -Text (New-FohHubToml -HttpPort 1 -BandPort 2 -MasterPort 3)) -ceq '') 'mbc-installed-linked-instance-none-without-one'
Assert ((Get-FohInstalledLinkedToml -Text ((New-FohHubToml -HttpPort 1 -BandPort 2 -MasterPort 3) + "`r`n[tls]`r`nname = `"x`"`r`n" + $wantLinked)) -ceq '') 'mbc-installed-linked-instance-never-after-tls'
$fohLinked = New-FohHubToml -HttpPort 18481 -BandPort 39181 -MasterPort 39182 -Linked $wantLinked
Assert ((Test-FohHubToml -Exe $HubExe -Text $fohLinked -Dir $checkDir) -like '*: OK*') 'mbc-hub-toml-check-accepts-the-linked-instance'

# ---- the MBC hub's toml (#184) ----
$wantMbc = (@(
        '# fohmixer-hub configuration, written by Install-Fohmixer.ps1: run the install again to change it.',
        'http_port = 18491',
        'layout = "layout.json"',
        '',
        '[[instances]]',
        'name = "mbc"',
        'port = 39183',
        '',
        '[link]',
        'listen = "10.0.0.5:39201"',
        'instance = "mbc"') -join "`r`n") + "`r`n"
$gotMbc = New-FohMbcHubToml -HttpPort 18491 -MbcPort 39183 -LinkListen '10.0.0.5:39201'
Assert ($gotMbc -ceq $wantMbc) 'mbc-hub-toml'
Assert (([regex]::Matches($gotMbc, '\[\[instances\]\]')).Count -eq 1) 'mbc-hub-toml-has-one-instance'
foreach ($t in @('[companion]', '[ableset]', '[iem]', '[presenter]', '[eq]', '[tls]')) { Assert (-not $gotMbc.Contains($t)) "mbc-hub-toml-has-no-$t" }
Assert ((Test-FohHubToml -Exe $HubExe -Text $gotMbc -Dir $checkDir) -like '*: OK*') 'mbc-hub-toml-check-accepts-it'
$everyAddress = ErrorOf { Test-FohHubToml -Exe $HubExe -Text (New-FohMbcHubToml -HttpPort 18491 -MbcPort 39183 -LinkListen '0.0.0.0:39201') -Dir $checkDir }
Assert ($everyAddress -like '*refused by the hub*' -and $everyAddress.Contains('[link] listen')) 'mbc-hub-toml-check-refuses-every-address'
Assert ((New-FohMbcHubToml -HttpPort 18491 -MbcPort 39183 -LinkListen '10.0.0.5:39201' -Remote "`r`n[tls]`r`nname = `"x`"`r`n").EndsWith("instance = `"mbc`"`r`n`r`n[tls]`r`nname = `"x`"`r`n")) 'mbc-hub-toml-keeps-the-tables-from-tls-on-last'
foreach ($bad in @('10.0.0.5', '10.0.0.5 :39201', '"x":1', '10.0.0.5:port')) {
    Assert ((ErrorOf { New-FohMbcHubToml -HttpPort 1 -MbcPort 2 -LinkListen $bad }) -like '*link address refused*') "mbc-hub-toml-refuses-a-link-address-[$bad]"
}
# Config.py's log folder: the script's own folder is Live's (Program Files), which Live's user cannot write.
$cfg = "INSTANCE = `"band`"`r`nPORT = 39101`r`nLOG_DIR = None`r`n"
Assert ((Set-FohConfigText -Text $cfg -Instance 'mbc' -Port 39103 -LogDir 'C:\ProgramData\fohmixer\logs') -ceq "INSTANCE = `"mbc`"`r`nPORT = 39103`r`nLOG_DIR = `"C:/ProgramData/fohmixer/logs`"`r`n") 'mbc-config-py-log-dir'
Assert ((Set-FohConfigText -Text $cfg -Instance 'band' -Port 39101) -ceq $cfg) 'config-py-log-dir-untouched-without-one'
Assert ((ErrorOf { Set-FohConfigText -Text $cfg -Instance 'mbc' -Port 1 -LogDir 'C:\a"b' }) -like '*log folder refused*') 'mbc-config-py-refuses-a-quote-in-the-log-dir'

# ---- the app window (#280) ----
$braveDir = Join-Path $base 'Brave Browser'
New-Item -ItemType Directory -Force -Path $braveDir | Out-Null
$brave = Join-Path $braveDir 'brave.exe'
[IO.File]::WriteAllText($brave, '')
$startupM = Join-Path $base 'mbc-startup'
New-Item -ItemType Directory -Force -Path $startupM | Out-Null
$wantApp = @($brave, '--profile-directory=Profile 1', '--app=http://localhost:18491/', '--start-maximized')
$gotApp = Get-FohTrayApp -Browser $brave -AppProfile 'Profile 1' -HttpPort 18491
Assert (($gotApp.Count -eq 4) -and (($gotApp -join '|') -ceq ($wantApp -join '|'))) 'mbc-tray-app-is-brave-app-window-on-the-local-hub'
$wantTray = "`r`n[tray]`r`napp = ['$brave', '--profile-directory=Profile 1', '--app=http://localhost:18491/', '--start-maximized']`r`n"
Assert ((Get-FohTrayToml -App $wantApp) -ceq $wantTray) 'mbc-tray-toml-literal-strings'
foreach ($bad in @(@("C:\it's\brave.exe"), @('a.exe', ''), @("a.exe", "x`ty"))) {
    Assert ((ErrorOf { Get-FohTrayToml -App $bad }) -like '*app command word refused*') "mbc-tray-toml-refuses-[$($bad -join ' ')]"
}
Assert ((ErrorOf { Get-FohTrayToml -App @() }) -like '*no program*') 'mbc-tray-toml-refuses-no-program'
Assert ((Format-FohAppArgs -Words @('--profile-directory=Profile 1', '--app=http://localhost:8480/', '--start-maximized')) -ceq '"--profile-directory=Profile 1" --app=http://localhost:8480/ --start-maximized') 'mbc-app-args-quote-a-word-with-a-space'
Assert ((Format-FohAppArgs -Words @()) -ceq '') 'mbc-app-args-none'
Assert ((ErrorOf { Format-FohAppArgs -Words @('a"b') }) -like '*double quote*') 'mbc-app-args-refuse-a-double-quote'
$gotMbcTray = New-FohMbcHubToml -HttpPort 18491 -MbcPort 39183 -LinkListen '10.0.0.5:39201' -Tray $wantTray -Remote "`r`n[tls]`r`nname = `"x`"`r`n"
Assert ($gotMbcTray -ceq ($wantMbc + $wantTray + "`r`n[tls]`r`nname = `"x`"`r`n")) 'mbc-hub-toml-tray-after-link-before-the-kept-tables'
Assert ((Test-FohHubToml -Exe $HubExe -Text ($wantMbc + $wantTray) -Dir $checkDir) -like '*: OK*') 'mbc-hub-toml-check-accepts-the-tray-table'
Assert ((Resolve-FohAppBrowser -AppBrowser $brave -MbcUser 'mbc-user') -ceq $brave) 'mbc-app-browser-given'
Assert ((ErrorOf { Resolve-FohAppBrowser -AppBrowser (Join-Path $base 'no-brave.exe') -MbcUser 'mbc-user' }) -like '*-AppBrowser not found*') 'mbc-app-browser-missing-refused'
Assert ((Resolve-FohStartupFolder -StartupFolder $startupM -MbcUser 'mbc-user') -ceq $startupM) 'mbc-startup-folder-given'
Assert ((ErrorOf { Resolve-FohStartupFolder -StartupFolder (Join-Path $base 'no-startup') -MbcUser 'mbc-user' }) -like '*Startup folder not found*') 'mbc-startup-folder-missing-refused'
Assert ((ErrorOf { Get-FohMbcAppPlan -AppBrowser $brave -AppProfile ' ' -HttpPort 1 -MbcUser 'mbc-user' -StartupFolder $startupM }) -like '*-AppProfile refused*') 'mbc-app-profile-empty-refused'
Assert ((ErrorOf { Get-FohMbcAppPlan -AppBrowser $brave -AppProfile 'P"1' -HttpPort 1 -MbcUser 'mbc-user' -StartupFolder $startupM }) -like '*double quote*') 'mbc-app-profile-with-a-quote-refused'
# The shortcut: written, kept when the same, rewritten when it differs.
$lnkDir = Join-Path $base 'lnk-test'
New-Item -ItemType Directory -Force -Path $lnkDir | Out-Null
Assert ((Set-FohAppShortcut -Folder $lnkDir -App $wantApp) -eq $true) 'mbc-app-shortcut-written'
Assert ((Set-FohAppShortcut -Folder $lnkDir -App $wantApp) -eq $false) 'mbc-app-shortcut-kept-when-the-same'
$shellT = New-Object -ComObject WScript.Shell
$lnkT = $shellT.CreateShortcut((Join-Path $lnkDir 'MBC mixer.lnk'))
Assert (($lnkT.TargetPath -ieq $brave) -and ($lnkT.Arguments -ceq '"--profile-directory=Profile 1" --app=http://localhost:18491/ --start-maximized')) "mbc-app-shortcut-runs-the-app-command ($($lnkT.Arguments))"
$null = [Runtime.InteropServices.Marshal]::ReleaseComObject($shellT)
$otherApp = Get-FohTrayApp -Browser $brave -AppProfile 'Profile 2' -HttpPort 18491
Assert ((Set-FohAppShortcut -Folder $lnkDir -App $otherApp) -eq $true) 'mbc-app-shortcut-rewritten-for-another-profile'

# ---- the bundle carries the MBC frame ----
Assert ((Get-FohSha256 (Join-Path $b1.stage 'mbc-frame.json')) -ceq (Get-FohSha256 (Join-Path $here 'mbc-frame.json'))) 'mbc-bundle-holds-the-mbc-frame'

# ---- install M1: the MBC PC (-Site Mbc, -NoTask) ----
$scripts = Join-Path $base 'Live 12 Suite\Resources\MIDI Remote Scripts'
New-Item -ItemType Directory -Force -Path (Join-Path $scripts '_Framework') | Out-Null
$dataM = Join-Path $base 'data-mbc'
function Get-MbcArgs([hashtable]$Over = @{}) {
    $a = [ordered]@{
        Site = 'Mbc'; BundleZip = $b1.zip; MbcUser = 'mbc-user'; DataDir = $dataM; HttpPort = 18491
        MbcPort = 39183; LinkListen = '10.0.0.5:39201'; ScriptFolder = $scripts; AppBrowser = $brave; StartupFolder = $startupM
    }
    foreach ($k in $Over.Keys) { if ($null -eq $Over[$k]) { $a.Remove($k) } else { $a[$k] = $Over[$k] } }
    $list = @()
    foreach ($k in $a.Keys) { $list += ('-' + $k); $list += [string]$a[$k] }
    return , ($list + '-NoTask')
}
$copyM = Join-Path $scripts 'FohMixer'
# Refused before any change: what the hub refuses, a folder that is not Live's, the other site's parameters.
$noneM = Join-Path $base 'data-mbc-refused'
$plain = Join-Path $base 'not-live-scripts'
New-Item -ItemType Directory -Force -Path $plain | Out-Null
foreach ($r in @(
        @{ over = @{ LinkListen = '0.0.0.0:39201' }; says = '*[link] listen*'; what = 'every-address' },
        @{ over = @{ ScriptFolder = $plain }; says = '*not Live*s MIDI Remote Scripts folder*'; what = 'a-folder-that-is-not-lives' },
        @{ over = @{ ScriptFolder = (Join-Path $base 'no-such-folder') }; says = '*not Live*s MIDI Remote Scripts folder*'; what = 'a-missing-script-folder' },
        @{ over = @{ MbcUser = $null }; says = '*-Site Mbc needs -MbcUser*'; what = 'no-mbc-user' },
        @{ over = @{ LinkListen = $null }; says = '*-Site Mbc needs -LinkListen*'; what = 'no-link-address' },
        @{ over = @{ BandUser = 'band-user' }; says = '*FOH PC*'; what = 'a-band-user' },
        @{ over = @{ MbcLink = $link }; says = '*FOH PC*'; what = 'a-link-to-itself' },
        @{ over = @{ CompanionHost = 'companion.example.org' }; says = '*FOH PC*'; what = 'a-companion' },
        @{ over = @{ MbcPort = 18491 }; says = '*the ports must differ*'; what = 'the-script-on-the-http-port' },
        @{ over = @{ LinkListen = '10.0.0.5:18491' }; says = '*the ports must differ*'; what = 'the-link-on-the-http-port' },
        @{ over = @{ LinkListen = '10.0.0.5:39183' }; says = '*the ports must differ*'; what = 'the-link-on-the-script-port' },
        @{ over = @{ AppBrowser = (Join-Path $base 'no-brave.exe') }; says = '*-AppBrowser not found*'; what = 'a-missing-browser' },
        @{ over = @{ StartupFolder = (Join-Path $base 'no-startup') }; says = '*Startup folder not found*'; what = 'a-missing-startup-folder' })) {
    $o = $r.over.Clone()
    $o['DataDir'] = $noneM
    $res = Invoke-Ps $install (Get-MbcArgs $o)
    # (No '[' in a -like pattern without its pair: it opens a character class.)
    Assert ($res.code -ne 0 -and $res.out -like $r.says.Replace('[', '`[')) "mbc-install-refuses-$($r.what)"
    Assert (-not (Test-Path -LiteralPath (Join-Path $noneM 'fohmixer-hub.toml')) -and -not (Test-Path -LiteralPath $copyM)) "mbc-install-refused-$($r.what)-before-any-change"
}
$m1 = Invoke-Ps $install (Get-MbcArgs)
Assert ($m1.code -eq 0) "mbc-install-1-exits-0 ($($m1.out))"
Assert ([IO.File]::ReadAllText((Join-Path $dataM 'fohmixer-hub.toml')) -ceq ($wantMbc + $wantTray)) 'mbc-install-1-toml-one-instance-the-link-and-the-tray'
$lnkM = Join-Path $startupM 'MBC mixer.lnk'
Assert (Test-Path -LiteralPath $lnkM -PathType Leaf) 'mbc-install-1-writes-the-startup-shortcut'
Assert ($m1.out.Contains('Startup shortcut MBC mixer.lnk (changed: True)')) 'mbc-install-1-says-the-shortcut-changed'
Assert ((Get-FohSha256 (Join-Path $dataM 'layout.json')) -ceq (Get-FohSha256 (Join-Path $here 'mbc-frame.json'))) 'mbc-install-1-layout-is-the-bundles-mbc-frame'
$cfgM = [IO.File]::ReadAllText((Join-Path $copyM 'Config.py'))
$logsM = (Join-Path $dataM 'logs').Replace('\', '/')
Assert ($cfgM.Contains("INSTANCE = `"mbc`"") -and $cfgM.Contains('PORT = 39183') -and $cfgM.Contains("LOG_DIR = `"$logsM`"")) 'mbc-install-1-config-py-mbc-its-port-and-log-folder'
Assert ((Get-FohSha256 (Join-Path $copyM 'version.py')) -ceq (Get-FohSha256 (Join-Path $b1.stage 'FohMixer\version.py'))) 'mbc-install-1-copies-fohmixer-into-lives-folder'
Assert (-not (Test-Path -LiteralPath (Join-Path $dataM 'script-backups'))) 'mbc-install-1-keeps-no-copy-of-nothing'
Assert (-not $m1.out.Contains('mbc-user')) 'mbc-install-1-never-prints-the-user-name'
# Again: nothing written.
$watchM = @{}
foreach ($p in @((Join-Path $dataM 'fohmixer-hub.toml'), (Join-Path $dataM 'layout.json'), $copyM, $lnkM)) { $watchM[$p] = $old }
Set-Stamps $watchM
$m2 = Invoke-Ps $install (Get-MbcArgs)
Assert ($m2.code -eq 0) 'mbc-install-2-again-exits-0'
$newer = @(Get-ChangedFiles $watchM)
Assert ($newer.Count -eq 0) "mbc-install-2-again-writes-nothing ($($newer -join ', '))"
Assert ($m2.out.Contains('Startup shortcut MBC mixer.lnk (changed: False)')) 'mbc-install-2-keeps-the-startup-shortcut'
# A layout in place is kept (the frame is the default only for a first install).
$ownLayout = ([IO.File]::ReadAllText((Join-Path $dataM 'layout.json'))).Replace('"title": "MBC"', '"title": "MBC 2"')
[IO.File]::WriteAllText((Join-Path $dataM 'layout.json'), $ownLayout)
# A Live update that brought an older FohMixer: replaced, the old one kept, dated, under the data folder.
[IO.File]::WriteAllText((Join-Path $copyM 'version.py'), "VERSION = `"0.0.1`"`n")
$m3 = Invoke-Ps $install (Get-MbcArgs)
Assert ($m3.code -eq 0) 'mbc-install-3-exits-0'
Assert ([IO.File]::ReadAllText((Join-Path $dataM 'layout.json')) -ceq $ownLayout) 'mbc-install-3-keeps-the-installed-layout'
Assert ((Get-FohSha256 (Join-Path $copyM 'version.py')) -ceq (Get-FohSha256 (Join-Path $b1.stage 'FohMixer\version.py'))) 'mbc-install-3-replaces-the-old-copy'
$kept = @(Get-ChildItem -LiteralPath (Join-Path $dataM 'script-backups') -Directory)
Assert ($kept.Count -eq 1 -and $kept[0].Name -cmatch '^\d{8}-\d{6}\z') "mbc-install-3-keeps-one-dated-copy ($(($kept | ForEach-Object { $_.Name }) -join ', '))"
Assert ([IO.File]::ReadAllText((Join-Path $kept[0].FullName 'FohMixer\version.py')) -ceq "VERSION = `"0.0.1`"`n") 'mbc-install-3-the-dated-copy-is-the-old-one'
$inLive = @(Get-ChildItem -LiteralPath $scripts -Directory | ForEach-Object { $_.Name })
Assert ($inLive.Count -eq 2 -and $inLive -ccontains '_Framework' -and $inLive -ccontains 'FohMixer') "mbc-install-3-leaves-nothing-else-in-lives-folder ($($inLive -join ', '))"

# Another profile: the toml's [tray] and the shortcut follow it.
$m3b = Invoke-Ps $install (Get-MbcArgs @{ AppProfile = 'Work' })
Assert ($m3b.code -eq 0) "mbc-install-3b-another-profile-exits-0 ($($m3b.out))"
Assert ([IO.File]::ReadAllText((Join-Path $dataM 'fohmixer-hub.toml')).Contains("'--profile-directory=Work'")) 'mbc-install-3b-tray-names-the-profile'
Assert ($m3b.out.Contains('Startup shortcut MBC mixer.lnk (changed: True)')) 'mbc-install-3b-rewrites-the-startup-shortcut'
foreach ($o in @(@{ AppProfile = 'Work' }, @{ AppBrowser = $brave }, @{ StartupFolder = $startupM })) {
    $what = @($o.Keys)[0]
    $o['DataDir'] = $noneM
    $res = Invoke-Ps $install (Get-InstallArgs $o)
    Assert ($res.code -ne 0 -and $res.out -like '*MBC PC*' -and -not (Test-Path -LiteralPath (Join-Path $noneM 'fohmixer-hub.toml'))) "foh-install-refuses-$what"
}

# ---- install M4: the FOH install's link (-MbcLink): written, kept by an update, the PC's own tables kept ----
$tomlR = Join-Path $dataR 'fohmixer-hub.toml'
$autotune = "`r`n[autotune]`r`nengage_delay_ms = 1000`r`n"
$beforeR = [IO.File]::ReadAllText($tomlR) + $autotune
[IO.File]::WriteAllText($tomlR, $beforeR)
$m4 = Invoke-Ps $install (Get-InstallArgs @{ DataDir = $dataR; MbcLink = $link })
Assert ($m4.code -eq 0) "mbc-install-4-foh-with-the-link-exits-0 ($($m4.out))"
$wantR4 = $beforeR.Replace("port = 39182`r`n", "port = 39182`r`n" + $wantLinked)
Assert ([IO.File]::ReadAllText($tomlR) -ceq $wantR4) 'mbc-install-4-toml-has-the-linked-instance-and-keeps-autotune'
$m5 = Invoke-Ps $install (Get-InstallArgs @{ DataDir = $dataR })
Assert ($m5.code -eq 0) 'mbc-install-5-foh-without-the-link-exits-0'
Assert ([IO.File]::ReadAllText($tomlR) -ceq $wantR4) 'mbc-install-5-keeps-the-linked-instance-and-autotune'
$res = Invoke-Ps $install (Get-InstallArgs @{ DataDir = $dataR; MbcLink = 'ws://10.0.0.5:0' })
Assert ($res.code -ne 0 -and [IO.File]::ReadAllText($tomlR) -ceq $wantR4) 'mbc-install-refuses-a-link-the-hub-refuses-before-any-change'
foreach ($o in @(@{ MbcUser = 'mbc-user' }, @{ LinkListen = '10.0.0.5:39201' }, @{ ScriptFolder = $scripts })) {
    $o['DataDir'] = $noneM
    $res = Invoke-Ps $install (Get-InstallArgs $o)
    Assert ($res.code -ne 0 -and $res.out -like '*MBC PC*' -and -not (Test-Path -LiteralPath (Join-Path $noneM 'fohmixer-hub.toml'))) "foh-install-refuses-$(@($o.Keys | Where-Object { $_ -ne 'DataDir' })[0])"
}
