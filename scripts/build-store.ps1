# Requires Windows, Rust's MSVC toolchain, C++ Build Tools and the Windows SDK.
# Builds an unsigned submission package. Microsoft signs it after certification.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9.-]{3,50}$')][string]$IdentityName,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Publisher,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$PublisherDisplayName
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Run this script on Windows.' }
$root = Split-Path $PSScriptRoot -Parent
$sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$makeappx = Get-ChildItem -Path "$sdkRoot\*\x64\makeappx.exe" |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if (-not $makeappx) { throw 'Install the Windows SDK (MakeAppx.exe is required).' }
$cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"\s*$') {
    throw 'Cargo.toml must contain a three-part release version.'
}
$parts = @([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
if (@($parts | Where-Object { $_ -gt 65535 }).Count -gt 0 -or $parts[0] -eq 0) {
    throw 'Store versions require a nonzero major number and each part <= 65535. Update Cargo.toml first (for example, 1.0.0).'
}
$version = ($parts -join '.') + '.0'
$outputDir = Join-Path $root 'target\msix'
$null = New-Item -ItemType Directory -Path $outputDir -Force
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
$originalFlags = $env:RUSTFLAGS
Push-Location $root
try {
    # Avoid requiring a separately installed Visual C++ runtime on customer PCs.
    $env:RUSTFLAGS = "$originalFlags -C target-feature=+crt-static".Trim()
    & cargo build --release --locked --features store --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    $null = New-Item -ItemType Directory -Path (Join-Path $stage 'Assets') -Force
    Copy-Item 'target\x86_64-pc-windows-msvc\release\restora-pc-agent.exe' $stage
    Copy-Item LICENSE $stage

    # Generate package assets from the existing app icon; no new branding.
    Add-Type -AssemblyName System.Drawing
    $icon = [System.Drawing.Icon]::new((Join-Path $root 'assets\restora.ico'), 256, 256)
    $source = $icon.ToBitmap()
    try {
        foreach ($asset in @(@('StoreLogo', 50), @('Square44x44Logo', 44), @('Square150x150Logo', 150))) {
            $bitmap = [System.Drawing.Bitmap]::new([int]$asset[1], [int]$asset[1])
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.Clear([System.Drawing.Color]::Transparent)
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.DrawImage($source, 0, 0, [int]$asset[1], [int]$asset[1])
                $bitmap.Save((Join-Path $stage ("Assets\{0}.png" -f $asset[0])), [System.Drawing.Imaging.ImageFormat]::Png)
            } finally { $graphics.Dispose(); $bitmap.Dispose() }
        }
    } finally { $source.Dispose(); $icon.Dispose() }

    [xml]$manifest = Get-Content (Join-Path $root 'packaging\AppxManifest.xml') -Raw
    $manifest.Package.Identity.SetAttribute('Name', $IdentityName)
    $manifest.Package.Identity.SetAttribute('Publisher', $Publisher)
    $manifest.Package.Identity.SetAttribute('Version', $version)
    $manifest.Package.Properties.PublisherDisplayName = $PublisherDisplayName
    $manifest.Save((Join-Path $stage 'AppxManifest.xml'))
    $package = Join-Path $outputDir "RestoraPrint_$($version)_x64.msix"
    & $makeappx.FullName pack /o /h SHA256 /d $stage /p $package
    if ($LASTEXITCODE -ne 0) { throw 'MSIX validation or packaging failed.' }
    Get-FileHash -Algorithm SHA256 $package
    Write-Host "Store submission package: $package"
    Write-Host 'This is unsigned. Do not distribute it as a customer installer.'
} finally {
    $env:RUSTFLAGS = $originalFlags
    Pop-Location
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
}
