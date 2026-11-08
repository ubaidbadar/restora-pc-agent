# Run on Windows PowerShell 7+ from any directory. No GitHub Actions or hosted runner required.
# Prerequisites: stable Rust toolchain with rustfmt and clippy, MSVC build tools.
param([switch]$Release)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    function Invoke-Cargo([string[]]$Arguments) {
        & cargo @Arguments
        if ($LASTEXITCODE -ne 0) { throw "cargo $($Arguments -join ' ') failed ($LASTEXITCODE)" }
    }

    Invoke-Cargo -Arguments @('fmt', '--all', '--', '--check')
    Invoke-Cargo -Arguments @('clippy', '--all-targets', '--locked', '--', '-D', 'warnings')
    Invoke-Cargo -Arguments @('test', '--locked')

    if ($Release) {
        if (-not $IsWindows) { throw 'Release binaries for this agent must be built and tested on Windows.' }
        Invoke-Cargo -Arguments @('build', '--release', '--locked')
        $exe = Join-Path $root 'target/release/restora-pc-agent.exe'
        if (-not (Test-Path $exe)) { throw "Expected Windows executable not found: $exe" }
        $hash = Get-FileHash -Algorithm SHA256 -Path $exe
        Write-Host "Release executable: $exe"
        Write-Host "SHA256: $($hash.Hash)"
        Write-Warning 'This executable is NOT an MSIX package, code-signed release, or Microsoft Store submission.'
    }
    Write-Host 'Local verification passed.'
}
finally {
    Pop-Location
}
