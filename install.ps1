<#
.SYNOPSIS
  Installs (or upgrades / removes) the mpv-remote plugin for mpv on Windows.

.EXAMPLE
  irm https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.ps1 | iex

.EXAMPLE
  # With options (a piped script can't take parameters directly):
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/BlueishTint/mpv-remote/main/install.ps1))) -ConfigDir D:\mpv\portable_config -OpenFirewall

.PARAMETER ConfigDir
  mpv config folder to install into. Auto-detected (scoop, portable_config next to mpv.exe, %APPDATA%\mpv) when omitted.
.PARAMETER Version
  Release tag such as v0.1.0. Defaults to the latest release.
.PARAMETER Source
  Folder or base URL containing the release assets (offline installs, testing).
.PARAMETER OpenFirewall
  Add an inbound firewall rule for the helper, limited to your local subnet. Needs an admin shell.
.PARAMETER AllowTailscale
  Like -OpenFirewall, but the rule also admits Tailscale addresses (100.64.0.0/10), for reaching the PC over a tailnet.
.PARAMETER Uninstall
  Remove the plugin. Your settings and saved marks are kept.
#>
[CmdletBinding()]
param(
    [string]$ConfigDir = $env:MPV_REMOTE_CONFIG_DIR,
    [string]$Version = $(if ($env:MPV_REMOTE_VERSION) { $env:MPV_REMOTE_VERSION } else { 'latest' }),
    [string]$Repo = $(if ($env:MPV_REMOTE_REPO) { $env:MPV_REMOTE_REPO } else { 'BlueishTint/mpv-remote' }),
    [string]$Source = $env:MPV_REMOTE_SOURCE,
    [switch]$OpenFirewall,
    [switch]$AllowTailscale,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$Port = 8765
$RuleName = 'mpv-remote'

function Write-Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Find-ConfigDirs {
    $found = [System.Collections.Generic.List[string]]::new()
    foreach ($root in @($env:SCOOP, (Join-Path $env:USERPROFILE 'scoop')) | Where-Object { $_ }) {
        Get-ChildItem (Join-Path $root 'persist') -Directory -Filter 'mpv*' -ErrorAction SilentlyContinue | ForEach-Object {
            $p = Join-Path $_.FullName 'portable_config'
            if (Test-Path $p) { $found.Add($p) }
        }
    }
    $mpv = Get-Command mpv -ErrorAction SilentlyContinue
    if ($mpv) {
        $p = Join-Path (Split-Path $mpv.Source) 'portable_config'
        if (Test-Path $p) { $found.Add($p) }
    }
    $appdata = Join-Path $env:APPDATA 'mpv'
    if (Test-Path $appdata) { $found.Add($appdata) }
    $found | Select-Object -Unique
}

function Select-ConfigDir {
    if ($ConfigDir) { return $ConfigDir }
    $dirs = @(Find-ConfigDirs)
    if ($dirs.Count -eq 0) { return (Join-Path $env:APPDATA 'mpv') }
    if ($dirs.Count -eq 1 -or -not [Environment]::UserInteractive) { return $dirs[0] }
    Write-Host 'Found several mpv config folders:'
    for ($i = 0; $i -lt $dirs.Count; $i++) { Write-Host ("  [{0}] {1}" -f ($i + 1), $dirs[$i]) }
    $answer = Read-Host "Install into which one? [1]"
    $n = if ($answer) { [int]$answer } else { 1 }
    if ($n -lt 1 -or $n -gt $dirs.Count) { throw "Invalid choice: $answer" }
    $dirs[$n - 1]
}

function Get-Asset($name, $destination) {
    if ($Source -and (Test-Path $Source -PathType Container)) {
        Copy-Item (Join-Path $Source $name) $destination
        return
    }
    $base = if ($Source) { $Source.TrimEnd('/') }
            elseif ($Version -eq 'latest') { "https://github.com/$Repo/releases/latest/download" }
            else { "https://github.com/$Repo/releases/download/$Version" }
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $destination
}

function Set-Firewall($exe) {
    # Local subnet always; the Tailscale range only when asked for.
    $remote = if ($AllowTailscale) { @('LocalSubnet', '100.64.0.0/10') } else { @('LocalSubnet') }
    $remoteText = ($remote | ForEach-Object { if ($_ -eq 'LocalSubnet') { $_ } else { "'$_'" } }) -join ','
    if (-not (Test-Admin)) {
        Write-Warning 'Not running as administrator, so no firewall rule was added. In an admin PowerShell run:'
        Write-Host "  New-NetFirewallRule -DisplayName $RuleName -Direction Inbound -Action Allow -Protocol TCP -LocalPort $Port -RemoteAddress $remoteText -Profile Any"
        return
    }
    Get-NetFirewallRule -DisplayName $RuleName -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -DisplayName $RuleName -Direction Inbound -Action Allow -Protocol TCP `
        -LocalPort $Port -RemoteAddress $remote -Profile Any -Program $exe | Out-Null
    $scope = if ($AllowTailscale) { 'local subnet and Tailscale' } else { 'local subnet only' }
    Write-Host "Firewall rule '$RuleName' added (TCP $Port, $scope)."
}

function Install-MpvRemote {
    $config = Select-ConfigDir
    $target = Join-Path $config 'scripts\mpv-remote'

    # A running helper keeps its exe locked; only the copy we're about to replace matters.
    $exe = [IO.Path]::GetFullPath((Join-Path $target 'mpv-remote-server.exe'))
    $busy = Get-Process mpv-remote-server -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and ([IO.Path]::GetFullPath($_.Path) -ieq $exe) }
    if ($busy) { throw 'mpv-remote-server is running from that folder. Close mpv and try again.' }

    if ($Uninstall) {
        Write-Step "Removing $target"
        if (Test-Path $target) { Remove-Item $target -Recurse -Force }
        if (Test-Admin) { Get-NetFirewallRule -DisplayName $RuleName -ErrorAction SilentlyContinue | Remove-NetFirewallRule }
        Write-Host "Removed. Settings (script-opts\mpv-remote.conf) and marks (mpv-remote\marks) were left in $config."
        return
    }

    $arch = switch ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
        'X64' { 'x86_64' }
        default { throw "No Windows build for $_ yet." }
    }
    $asset = "mpv-remote-windows-$arch.zip"

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("mpv-remote-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory $tmp | Out-Null
    try {
        Write-Step "Downloading $asset ($Version)"
        Get-Asset $asset (Join-Path $tmp $asset)
        Get-Asset 'SHA256SUMS' (Join-Path $tmp 'SHA256SUMS')

        Write-Step 'Verifying checksum'
        $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { $_ -match [regex]::Escape($asset) + '$' } | Select-Object -First 1
        if (-not $line) { throw "SHA256SUMS has no entry for $asset" }
        $expected = ($line -split '\s+')[0]
        $actual = (Get-FileHash (Join-Path $tmp $asset) -Algorithm SHA256).Hash
        if ($actual -ne $expected.ToUpper()) { throw "Checksum mismatch for $asset (expected $expected, got $actual)" }

        Write-Step "Installing to $target"
        New-Item -ItemType Directory -Force $target | Out-Null
        Expand-Archive (Join-Path $tmp $asset) -DestinationPath $target -Force
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    $conf = Join-Path $config 'script-opts\mpv-remote.conf'
    if (-not (Test-Path $conf)) {
        New-Item -ItemType Directory -Force (Split-Path $conf) | Out-Null
        @'
# mpv-remote settings. The pairing token is added automatically on first run.
# port=8765
# host=            # LAN IP shown in the QR code, if the wrong one is picked
# browse_roots=    # folders the phone may browse, separated by |  (default: your home folder)
# autostart=yes
'@ | Set-Content $conf -Encoding utf8
    }

    $wantFirewall = $OpenFirewall -or $AllowTailscale
    if ($wantFirewall) { Set-Firewall (Join-Path $target 'mpv-remote-server.exe') }

    # Ignore VPN-style virtual networks (WARP etc.); only the real LAN matters for the phone.
    $publicNet = Get-NetConnectionProfile -ErrorAction SilentlyContinue | Where-Object {
        $_.NetworkCategory -eq 'Public' -and "$($_.Name) $($_.InterfaceAlias)" -notmatch 'warp|vpn|tailscale|zerotier|vethernet|wsl|hyper-v|vmware|virtualbox'
    }
    Write-Host ''
    Write-Host "Installed mpv-remote $(Get-Content (Join-Path $target 'VERSION') -ErrorAction SilentlyContinue) into $config" -ForegroundColor Green
    Write-Host 'Next: start mpv, play something, press Ctrl+r and scan the QR code with your phone.'
    if ($publicNet -and -not $wantFirewall) {
        Write-Warning "Your network '$($publicNet[0].Name)' is set to Public, so Windows will block your phone. Re-run with -OpenFirewall in an admin shell, or switch the network to Private."
    }
}

try {
    Install-MpvRemote
} catch {
    Write-Host "Error: $($_.Exception.Message)" -ForegroundColor Red
}
