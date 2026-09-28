#Requires -Version 5.1
# fohmixer's inbound firewall rules on the Ableton PC (spec D7: the LAN only),
# dot-sourced by FohmixerPc.psm1 and shipped in the bundle next to it: the
# hub's HTTP port (fohmixer-hub-http) and, with remote access (#17), its HTTPS
# port (fohmixer-hub-https). Windows PowerShell 5.1, ASCII only.

function Get-FohFirewallRule {
    # A rule as plain values, or $null.
    param([Parameter(Mandatory)][string]$Name)
    $r = Get-NetFirewallRule -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $r) { return $null }
    $pf = $r | Get-NetFirewallPortFilter
    return [pscustomobject]@{
        direction = "$($r.Direction)"
        action = "$($r.Action)"
        profile = ((@("$($r.Profile)" -split ',\s*') | Sort-Object) -join ',')
        enabled = "$($r.Enabled)"
        protocol = "$($pf.Protocol)"
        ports = ((@($pf.LocalPort) | ForEach-Object { "$_" } | Sort-Object) -join ',')
    }
}

function Test-FohFirewallRule {
    param($Rule, [Parameter(Mandatory)][int]$Port, [Parameter(Mandatory)][string]$Enabled)
    if ($null -eq $Rule) { return $false }
    return ($Rule.direction -eq 'Inbound' -and $Rule.action -eq 'Allow' -and $Rule.profile -eq 'Domain,Private' -and
            $Rule.enabled -eq $Enabled -and $Rule.protocol -eq 'TCP' -and $Rule.ports -eq "$Port")
}

function Set-FohFirewallRule {
    # The hub's one inbound rule (iemmixer's pattern): TCP <Port> on the private
    # and domain profiles, whichever version's exe listens (each has its own
    # path). Created or repaired, then read back. The readiness poll uses
    # 127.0.0.1, which no rule blocks: without this the iPads may be refused
    # while the install looks fine. -Disabled only for the self-test. Returns
    # whether it changed anything.
    param([Parameter(Mandatory)][int]$Port, [string]$Name = 'fohmixer-hub-http', [switch]$Disabled)
    $enabled = 'True'
    if ($Disabled) { $enabled = 'False' }
    $before = Get-FohFirewallRule -Name $Name
    $changed = $false
    if ($null -eq $before) {
        New-NetFirewallRule -Name $Name -DisplayName $Name -Description 'fohmixer: the hub on the LAN (Install-Fohmixer.ps1)' `
            -Direction Inbound -Action Allow -Protocol TCP -LocalPort $Port -Profile Domain, Private -Enabled $enabled | Out-Null
        $changed = $true
    } elseif (-not (Test-FohFirewallRule -Rule $before -Port $Port -Enabled $enabled)) {
        Set-NetFirewallRule -Name $Name -Direction Inbound -Action Allow -Protocol TCP -LocalPort $Port -Profile Domain, Private -Enabled $enabled
        $changed = $true
    }
    $after = Get-FohFirewallRule -Name $Name
    if (-not (Test-FohFirewallRule -Rule $after -Port $Port -Enabled $enabled)) {
        throw ('firewall rule {0} reads back as {1}' -f $Name, (ConvertTo-Json -InputObject $after -Compress))
    }
    return $changed
}
