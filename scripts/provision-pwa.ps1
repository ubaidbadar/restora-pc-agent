#Requires -Version 5.1
<#
Installs a separately built, production-signed Restora PWA MSIX for CURRENT USER.
For a one-EXE installer, call this with the MSIX embedded in the installer.
No Edge/Chrome enterprise policy, certificate installation or unsigned package bypass.
The SHA-256, package identity, publisher and app entry point must match the release.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackagePath,
    [Parameter(Mandatory = $true)][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][string]$PackageName,
    [Parameter(Mandatory = $true)][string]$ExpectedPublisher,
    [Parameter(Mandatory = $true)][string]$ApplicationId
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($ExpectedSha256 -notmatch '^[a-fA-F0-9]{64}$') {
    throw 'ExpectedSha256 must be a pinned 64-character SHA-256 digest.'
}
if ($PackageName -notmatch '^[A-Za-z0-9._-]+$' -or
    $ApplicationId -notmatch '^[A-Za-z0-9._-]+$') {
    throw 'Invalid PWA package or application identity.'
}
if (-not (Test-Path -LiteralPath $PackagePath -PathType Leaf) -or
    [IO.Path]::GetExtension($PackagePath) -ne '.msix') {
    throw 'The signed Restora PWA .msix file is missing.'
}
$source = (Resolve-Path -LiteralPath $PackagePath).Path
$digest = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
if ($digest -ne $ExpectedSha256) {
    throw 'Restora PWA package digest mismatch. Installation stopped.'
}

# Read the manifest before installation, so an unexpected vendor/app cannot
# be registered even when the package has a valid Windows trust signature.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($source)
try {
    $entry = $zip.GetEntry('AppxManifest.xml')
    if ($null -eq $entry) { throw 'The MSIX does not contain AppxManifest.xml.' }
    $reader = [System.IO.StreamReader]::new($entry.Open())
    try {
        [xml]$manifest = $reader.ReadToEnd()
    } finally {
        $reader.Dispose()
    }
} finally {
    $zip.Dispose()
}
$ns = [System.Xml.XmlNamespaceManager]::new($manifest.NameTable)
$ns.AddNamespace('f', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10')
$identity = $manifest.SelectSingleNode('/f:Package/f:Identity', $ns)
$application = $manifest.SelectSingleNode('/f:Package/f:Applications/f:Application[@Id="' + $ApplicationId + '"]', $ns)
if ($null -eq $identity -or $null -eq $application -or
    $identity.GetAttribute('Name') -cne $PackageName -or
    $identity.GetAttribute('Publisher') -cne $ExpectedPublisher) {
    throw 'Signed Restora PWA package identity or entry point does not match.'
}
$targetVersion = [version]$identity.GetAttribute('Version')

# A Chromium-installed PWA may already exist independently of the MSIX.
# Do not silently create a second Restora entry on an existing restaurant PC.
$installed = Get-AppxPackage -Name $PackageName |
    Where-Object { $_.Name -eq $PackageName } |
    Sort-Object Version -Descending | Select-Object -First 1
if ($null -eq $installed) {
    $existingBrowserPwa = @(Get-StartApps | Where-Object {
        $_.Name -eq 'Restora' -and $_.AppID -match '(MSEdge|Chrome|_crx_)'
    })
    if ($existingBrowserPwa.Count -gt 0) {
        throw 'An existing Edge/Chrome Restora PWA was found. Skipping a duplicate installation; migration needs verification.'
    }
}
if ($null -ne $installed -and $installed.Publisher -cne $ExpectedPublisher) {
    throw 'An installed package with the expected name has a different publisher.'
}

# Add-AppxPackage enforces Windows package signing and trusted publisher checks.
# NEVER use -AllowUnsigned, development certificates, or install a new trust root.
if ($null -eq $installed -or [version]$installed.Version -lt $targetVersion) {
    Add-AppxPackage -Path $source -ErrorAction Stop
}

$installed = Get-AppxPackage -Name $PackageName |
    Where-Object { $_.Name -eq $PackageName -and $_.Publisher -ceq $ExpectedPublisher } |
    Sort-Object Version -Descending | Select-Object -First 1
if ($null -eq $installed) {
    throw 'Windows did not register the Restora PWA for this user.'
}
$registered = Get-AppxPackageManifest -Package $installed.PackageFullName
$registeredApp = $registered.SelectSingleNode(
    '/f:Package/f:Applications/f:Application[@Id="' + $ApplicationId + '"]', $ns)
if ($null -eq $registeredApp) {
    throw 'The installed Restora PWA application entry point was not found.'
}

# The agent uses Windows' stable app identity rather than browser internals.
$aumid = $installed.PackageFamilyName + '!' + $ApplicationId
$folder = Join-Path $env:APPDATA 'restora'
New-Item -ItemType Directory -Path $folder -Force | Out-Null
$target = Join-Path $folder 'pwa-aumid.txt'
$temp = Join-Path $folder 'pwa-aumid.tmp'
Set-Content -LiteralPath $temp -Value $aumid -NoNewline -Encoding ASCII
Move-Item -LiteralPath $temp -Destination $target -Force
Write-Output "Restora PWA installed for this user: $aumid"
