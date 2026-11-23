# Integration test on a disposable GitHub-hosted Windows VM.
# Interacts with the REAL Windows spooler; never sends a print job or installs
# any driver from the internet. Virtual USB ports are LOCAL PORTS, not hardware.
param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [string]$ReportPath = 'printer-integration-report.json'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Import-Module PrintManagement -ErrorAction Stop
$exe = (Resolve-Path $Executable).Path
$report = [ordered]@{
    suite = 'Windows spooler queue integration (no physical hardware)'
    os = [Environment]::OSVersion.VersionString
    result = 'running'
    checks = [ordered]@{}
    hardware = 'not tested: no USB printer connected'
}
$profile = Join-Path $env:RUNNER_TEMP ('restora-printer-' + [guid]::NewGuid().ToString('N'))
$nameLocal = 'Restora CI Virtual Serial'
$nameUsb = 'Restora CI Synthetic USB'
$usbPort = 'USB986'
$localPort = 'NUL:'
$createdQueues = @()
$createdPorts = @()
$engine = $null
$originalAppData = $env:APPDATA
$base = 'http://127.0.0.1:17777'
function Note([string]$key, [string]$detail) {
    $report.checks[$key] = $detail
    Write-Host "EVIDENCE $key : $detail"
}
function Get-Names {
    $response = Invoke-RestMethod -Uri "$base/printers" -TimeoutSec 2
    return @($response)
}
function Get-Row([string]$name) {
    $status = Invoke-RestMethod -Uri "$base/status" -TimeoutSec 2
    return @($status.printers | Where-Object { $_.systemName -eq $name } | Select-Object -First 1)[0]
}
function Until([string]$label, [scriptblock]$condition, [int]$seconds = 50) {
    $deadline = (Get-Date).AddSeconds($seconds)
    $lastError = ''
    do {
        try {
            if (& $condition) {
                Write-Host "PASS: $label"
                return
            }
        } catch {
            $lastError = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 750
    } while ((Get-Date) -lt $deadline)
    throw "Timeout waiting for $label. Last error: $lastError"
}
try {
    # The test runner is dedicated; no human's print settings are modified.
    New-Item -ItemType Directory -Path (Join-Path $profile 'restora') -Force | Out-Null
    $env:APPDATA = $profile
    $spooler = Get-Service -Name Spooler -ErrorAction Stop
    if ($spooler.Status -ne 'Running') {
        Start-Service Spooler -ErrorAction Stop
        (Get-Service Spooler).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    }
    Note 'spooler_service' 'real Windows Print Spooler is running'

    # Require an installed/inbox Windows driver, never download vendor drivers.
    $drivers = @(Get-PrinterDriver -ErrorAction Stop)
    if (-not $drivers) {
        try { Add-PrinterDriver -Name 'Generic / Text Only' -ErrorAction Stop }
        catch { Write-Warning "Inbox driver unavailable: $_" }
        $drivers = @(Get-PrinterDriver -ErrorAction Stop)
    }
    if (-not $drivers) { throw 'No spooler driver is installed; cannot create a virtual queue' }
    $preferred = @('Generic / Text Only', 'Microsoft XPS Document Writer v4', 'Microsoft Print To PDF')
    $driver = $null
    foreach ($choice in $preferred) {
        $driver = @($drivers | Where-Object { $_.Name -eq $choice } | Select-Object -First 1)
        if ($driver.Count -gt 0) { $driver = $driver[0]; break }
    }
    if (-not $driver -or @($driver).Count -eq 0) {
        $driver = @($drivers | Where-Object { $_.Name -and $_.PrinterEnvironment -like '*64*' } | Select-Object -First 1)
        if ($driver.Count -eq 0) { $driver = $drivers[0] } else { $driver = $driver[0] }
    }
    Note 'driver' $driver.Name

    # NUL: has no paper output; USB986 is a local port with a USB-like *name*.
    # This specifically tests the bug where an installed USB queue falsely appeared ready.
    foreach ($port in @($localPort, $usbPort)) {
        if (-not (Get-PrinterPort -Name $port -ErrorAction SilentlyContinue)) {
            Add-PrinterPort -Name $port -ErrorAction Stop
            $createdPorts += $port
        }
    }

    Add-Printer -Name $nameLocal -DriverName $driver.Name -PortName $localPort -ErrorAction Stop
    $createdQueues += $nameLocal
    Note 'virtual_queue' "Created '$nameLocal' on NUL: (no physical printer)"

    # The app has no console or native setup UI; launch as a headless engine.
    $engine = Start-Process -FilePath $exe -ArgumentList @('--headless','--background') -PassThru -WindowStyle Hidden
    Until 'agent listening' {
        if ($engine.HasExited) { throw "Agent exited with $($engine.ExitCode)" }
        (Invoke-RestMethod -Uri "$base/status" -TimeoutSec 2).version -ne $null
    } 25
    Until 'virtual local queue discovered' { (Get-Names) -contains $nameLocal } 50
    $row = Get-Row $nameLocal
    if (-not $row -or $row.found -ne $true -or $row.source -ne 'spooler') {
        throw "Wrong virtual queue status: $($row | ConvertTo-Json -Compress -Depth 5)"
    }
    Note 'real_enumprinters_level2' "Queue found=$($row.found), state=$($row.state), source=$($row.source)"

    # GET must remain cache-only and responsive; 50 requests even when no printer is present.
    $clock = [Diagnostics.Stopwatch]::StartNew()
    for ($i=0; $i -lt 50; $i++) {
        $snapshot = Invoke-RestMethod -Uri "$base/status" -TimeoutSec 2
        if ($snapshot.version -eq $null) { throw 'GET /status returned invalid JSON' }
    }
    $clock.Stop()
    if ($clock.Elapsed.TotalSeconds -gt 8) { throw "/status latency: $($clock.Elapsed.TotalSeconds)s for 50 calls" }
    Note 'cached_status_latency' "50 real HTTP snapshots in $([math]::Round($clock.Elapsed.TotalSeconds, 2)) seconds"

    # Add a USB-looking queue WHILE the engine is running; proves change notification
    # or 30s fallback actually updates the cache on a hosted Windows machine.
    Add-Printer -Name $nameUsb -DriverName $driver.Name -PortName $usbPort -ErrorAction Stop
    $createdQueues += $nameUsb
    Until 'new USB-like queue discovered' { (Get-Names) -contains $nameUsb } 50
    $usb = Get-Row $nameUsb
    if (-not $usb -or $usb.state -ne 'unknown') {
        throw "False Ready for a local virtual USB-looking port: $($usb | ConvertTo-Json -Compress -Depth 5)"
    }
    Note 'false_ready_prevention' 'Local port named USB986 was installed; agent correctly returns unknown, not ready'

    # Win32_Printer Pause/Resume is a genuine spooler control operation; it is
    # not a simulated paper sensor. Verify visible status when the driver supports it.
    $cim = Get-CimInstance Win32_Printer | Where-Object { $_.Name -eq $nameLocal } | Select-Object -First 1
    if (-not $cim) { throw 'Win32_Printer did not list our temporary queue' }
    try {
        $pause = Invoke-CimMethod -InputObject $cim -MethodName Pause -ErrorAction Stop
        if ($pause.ReturnValue -ne 0) { throw "Pause returned $($pause.ReturnValue)" }
        Until 'paused queue is not ready' {
            $row = Get-Row $nameLocal
            $row -and $row.state -ne 'ready'
        } 50
        Note 'queue_paused' 'Real Windows spooler Pause reflected in agent health (not Ready)'
    } finally {
        $cimAgain = Get-CimInstance Win32_Printer | Where-Object { $_.Name -eq $nameLocal } | Select-Object -First 1
        if ($cimAgain) { $null = Invoke-CimMethod -InputObject $cimAgain -MethodName Resume -ErrorAction Stop }
    }
    Note 'queue_resumed' 'Real Windows spooler Resume invoked'

    Remove-Printer -Name $nameUsb -ErrorAction Stop
    $createdQueues = @($createdQueues | Where-Object { $_ -ne $nameUsb })
    Until 'deleted USB-like queue disappears' { (Get-Names) -notcontains $nameUsb } 50
    Note 'event_or_fallback_removal' 'Removed printer disappears from cache without restarting agent'

    # Fault injection against the REAL OS service, not a mocked status flag.
    # This is safe only because the CI runner is disposable and dedicated.
    try {
        Stop-Service -Name Spooler -Force -ErrorAction Stop
        (Get-Service Spooler).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        $clock = [Diagnostics.Stopwatch]::StartNew()
        for ($i=0; $i -lt 20; $i++) {
            $snapshot = Invoke-RestMethod -Uri "$base/status" -TimeoutSec 2
            if (-not $snapshot.version) { throw 'Cached GET failed with Print Spooler stopped' }
        }
        $clock.Stop()
        if ($clock.Elapsed.TotalSeconds -gt 8) {
            throw "Cached /status blocked by stopped spooler: $($clock.Elapsed.TotalSeconds)s"
        }
        Note 'spooler_stopped' "Agent /status served 20 snapshots in $([math]::Round($clock.Elapsed.TotalSeconds, 2)) seconds without Windows Print Spooler"
    } finally {
        Start-Service Spooler -ErrorAction Stop
        (Get-Service Spooler).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    }
    Until 'local queue visible after spooler restart' { (Get-Names) -contains $nameLocal } 50
    Note 'spooler_restart' 'Actual Windows Print Spooler stop/restart completed; agent stayed responsive'

    # The WPP query is a real WinRT call. Windows Server 2022 is expected to
    # return unknown; it must never manufacture false from missing policy keys.
    $snapshot = Invoke-RestMethod -Uri "$base/status" -TimeoutSec 2
    if ("$($snapshot.wpp)" -notin @('unknown','True','False')) {
        throw "Invalid WPP result: $($snapshot.wpp)"
    }
    Note 'effective_wpp_result' "$($snapshot.wpp) (not an effective WPP ON/OFF simulation)"

    # No real print job is emitted in this suite; printed.log must remain absent.
    if (Test-Path (Join-Path $profile 'restora\printed.log')) {
        throw 'Printer integration test must not modify printed.log'
    }
    Note 'printed_log_preserved' 'No printed.log was written; no test receipt sent'

    $report.result = 'passed'
} catch {
    $report.result = 'failed'
    $report["error"] = $_.Exception.ToString()
    Write-Error -ErrorAction Continue "Windows print integration failed: $_"
    throw
} finally {
    if ($engine -and -not $engine.HasExited) {
        Stop-Process -Id $engine.Id -Force -ErrorAction SilentlyContinue
        $engine.WaitForExit(5000) | Out-Null
    }
    # No leave-behind queues/ports on GitHub runner, even if an assertion failed.
    if ((Get-Service Spooler -ErrorAction SilentlyContinue).Status -ne 'Running') {
        try { Start-Service Spooler -ErrorAction Stop } catch { Write-Warning "Spooler restart cleanup: $_" }
    }
    foreach ($name in $createdQueues) {
        try { Remove-Printer -Name $name -ErrorAction Stop }
        catch { Write-Warning "Printer cleanup $name : $_" }
    }
    foreach ($port in $createdPorts) {
        try { Remove-PrinterPort -Name $port -ErrorAction Stop }
        catch { Write-Warning "Port cleanup $port : $_" }
    }
    $env:APPDATA = $originalAppData
    $target = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ReportPath)
    $report | ConvertTo-Json -Depth 8 | Set-Content -Path $target -Encoding utf8
    Write-Host "Windows printer integration report: $target"
    Remove-Item $profile -Recurse -Force -ErrorAction SilentlyContinue
}
