param(
    [string]$Executable,
    [int]$Runs = 3
)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Executable) { $Executable = Join-Path $repoRoot 'target/release/melos.exe' }
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$outputDir = Join-Path $repoRoot 'output'
New-Item -ItemType Directory -Force $outputDir | Out-Null
$oldReport = $env:MELOS_SMOKE_REPORT
$samples = @()
try {
    for ($index = 0; $index -lt $Runs; $index++) {
        $report = Join-Path $outputDir ('smoke-' + [guid]::NewGuid().ToString('N') + '.json')
        $env:MELOS_SMOKE_REPORT = $report
        $timer = [Diagnostics.Stopwatch]::StartNew()
        $process = Start-Process -FilePath $Executable -WindowStyle Hidden -PassThru
        try {
            while (-not (Test-Path -LiteralPath $report)) {
                $process.Refresh()
                if ($process.HasExited) { throw 'Application exited before producing a smoke report.' }
                if ($timer.Elapsed.TotalSeconds -gt 15) { throw 'Application startup timed out.' }
                Start-Sleep -Milliseconds 10
            }
            $startupMs = $timer.Elapsed.TotalMilliseconds
            Start-Sleep -Milliseconds 250
            $process.Refresh()
            $memoryMB = $process.WorkingSet64 / 1000000
            $smoke = Get-Content -Raw -LiteralPath $report | ConvertFrom-Json
            $samples += [pscustomobject]@{
                StartupToReportMs = [math]::Round($startupMs, 1)
                WorkingSetMB = [math]::Round($memoryMB, 2)
                SmtcAvailable = $smoke.smtc_available
                MetadataPublished = $smoke.metadata_published
                AudioDeviceAvailable = $smoke.audio_device_available
            }
            if (-not $process.WaitForExit(5000)) { throw 'Smoke process did not exit.' }
        } finally {
            $process.Refresh()
            if (-not $process.HasExited) { Stop-Process -Id $process.Id }
            $process.Dispose()
        }
    }
    $result = [pscustomobject]@{
        Executable = $Executable
        BinaryMB = [math]::Round((Get-Item $Executable).Length / 1000000, 2)
        Measurement = 'Process start to view-initialization smoke report (hidden window); repeated runs, not time to a visible first frame or a controlled cold-start benchmark.'
        Samples = $samples
    }
    $result | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $outputDir 'performance.json')
    $result
} finally {
    if ($null -eq $oldReport) { Remove-Item Env:MELOS_SMOKE_REPORT -ErrorAction SilentlyContinue }
    else { $env:MELOS_SMOKE_REPORT = $oldReport }
}
