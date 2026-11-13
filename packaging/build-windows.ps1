param([switch]$SkipBuild, [string]$TuiBinary)
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $repoRoot
try {
    if (-not $SkipBuild) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        $vcvars = if (Test-Path -LiteralPath $vswhere) { & $vswhere -latest -prerelease -products '*' -find 'VC\Auxiliary\Build\vcvars64.bat' | Select-Object -First 1 }
        if (-not $vcvars) {
            $installRoot = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio'
            $candidates = foreach ($versionDir in Get-ChildItem -LiteralPath $installRoot -Directory) {
                foreach ($editionDir in Get-ChildItem -LiteralPath $versionDir.FullName -Directory) {
                    $candidate = Join-Path $editionDir.FullName 'VC/Auxiliary/Build/vcvars64.bat'
                    if (Test-Path -LiteralPath $candidate) { $candidate }
                }
            }
            $vcvars = $candidates | Sort-Object -Descending | Select-Object -First 1
        }
        if (-not $vcvars) { throw 'MSVC Build Tools with the C++ workload and Windows SDK are required.' }
        & cmd /d /s /c "`"$vcvars`" && cargo build --release --locked -p melos-app -p melos-tui"
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $binary = Join-Path $repoRoot 'target/release/melos.exe'
    if (-not (Test-Path -LiteralPath $binary)) { throw 'Release binary does not exist.' }
    $version = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion
    $workspaceVersion = [regex]::Match([IO.File]::ReadAllText((Join-Path $repoRoot 'Cargo.toml')), '(?ms)^\[workspace\.package\]\s*\nversion\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $workspaceVersion -or $version -ne $workspaceVersion) {
        throw 'The Windows binary resource version does not match the workspace version. Rebuild with matching resources before packaging.'
    }
    $outputDir = Join-Path $repoRoot 'output/release'
    $stage = Join-Path $repoRoot ('output/package-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $outputDir, $stage | Out-Null
    Copy-Item -LiteralPath $binary -Destination $stage
    $terminalBinary = if ($TuiBinary) {
        (Resolve-Path -LiteralPath $TuiBinary).Path
    } else {
        Join-Path $repoRoot 'target/release/melos-tui.exe'
    }
    if (-not (Test-Path -LiteralPath $terminalBinary)) { throw 'Terminal release binary does not exist.' }
    $terminalVersion = (Get-Item -LiteralPath $terminalBinary).VersionInfo.ProductVersion
    if ($terminalVersion -and $terminalVersion -ne $workspaceVersion) {
        throw 'The terminal binary resource version does not match the workspace version.'
    }
    Copy-Item -LiteralPath $terminalBinary -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repoRoot 'packaging/windows/README.txt') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repoRoot 'packaging/THIRD_PARTY_NOTICES.md') -Destination $stage
    $archive = Join-Path $outputDir "melos-$version-windows-x64.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -Force
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$archive.sha256", "$hash  $([IO.Path]::GetFileName($archive))`n")

    $tuiStage = Join-Path $repoRoot ('output/package-tui-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $tuiStage | Out-Null
    Copy-Item -LiteralPath $terminalBinary -Destination $tuiStage
    Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination $tuiStage
    Copy-Item -LiteralPath (Join-Path $repoRoot 'packaging/windows/TUI-README.txt') -Destination $tuiStage
    Copy-Item -LiteralPath (Join-Path $repoRoot 'packaging/THIRD_PARTY_NOTICES.md') -Destination $tuiStage
    $tuiArchive = Join-Path $outputDir "melos-tui-$version-windows-x64.zip"
    Compress-Archive -Path (Join-Path $tuiStage '*') -DestinationPath $tuiArchive -Force
    $tuiHash = (Get-FileHash -LiteralPath $tuiArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$tuiArchive.sha256", "$tuiHash  $([IO.Path]::GetFileName($tuiArchive))`n")

    [pscustomobject]@{ Archive = $archive; SHA256 = $hash; BinaryMB = [math]::Round((Get-Item $binary).Length / 1000000, 2) }
    [pscustomobject]@{ Archive = $tuiArchive; SHA256 = $tuiHash; BinaryMB = [math]::Round((Get-Item $terminalBinary).Length / 1000000, 2) }
} finally { Pop-Location }
