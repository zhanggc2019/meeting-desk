[CmdletBinding()]
param(
    [switch]$Force,
    [switch]$CheckOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$pythonVersion = "3.12.9"
$pythonArchiveName = "python-$pythonVersion-embed-amd64.zip"
$pythonArchiveUrl = "https://www.python.org/ftp/python/$pythonVersion/$pythonArchiveName"
$pythonArchiveSha256 = "615861FB801E8B04C847598DB4E1E46E4B046295017CAA37CB5486DDE72B5865"
$getPipUrl = "https://bootstrap.pypa.io/get-pip.py"
$getPipSha256 = "FB24E693BAB954209A063D90953621412CCAD4A500905A726286E038F508DDF6"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$runtimeParent = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "src-tauri\runtime"))
$runtimeRoot = [System.IO.Path]::GetFullPath((Join-Path $runtimeParent "python"))
$cacheRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "src-tauri\.runtime-cache"))
$requirementsPath = Join-Path $repoRoot "src-tauri\python\requirements-local-asr.txt"
$markerName = ".meetingdesk-runtime.json"

<# Ensures recursive writes and deletes stay inside the generated runtime directory. #>
function Assert-RuntimePath {
    param([Parameter(Mandatory)][string]$Path)

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $allowedPrefix = $runtimeParent.TrimEnd('\') + '\'
    if (-not $fullPath.StartsWith($allowedPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to operate outside the generated runtime directory"
    }
}

<# Computes SHA256 without relying on optional PowerShell utility modules. #>
function Get-Sha256 {
    param([Parameter(Mandatory)][string]$Path)

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "")
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

<# Downloads a pinned supply-chain input and reuses only a hash-verified cache. #>
function Get-VerifiedDownload {
    param(
        [Parameter(Mandatory)][string]$Uri,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][string]$ExpectedSha256
    )

    if (Test-Path -LiteralPath $Destination) {
        $cachedHash = Get-Sha256 -Path $Destination
        if ($cachedHash -eq $ExpectedSha256) {
            return
        }
        Remove-Item -LiteralPath $Destination -Force
    }
    Invoke-WebRequest -Uri $Uri -OutFile $Destination -TimeoutSec 120
    $downloadedHash = Get-Sha256 -Path $Destination
    if ($downloadedHash -ne $ExpectedSha256) {
        Remove-Item -LiteralPath $Destination -Force
        throw "Download hash verification failed: $([System.IO.Path]::GetFileName($Destination))"
    }
}

<# Returns the requirements SHA256 used to invalidate stale runtime builds. #>
function Get-RequirementsHash {
    return Get-Sha256 -Path $requirementsPath
}

<# Checks whether the runtime marker and interpreter match current build inputs. #>
function Test-CurrentRuntime {
    param([Parameter(Mandatory)][string]$Root)

    $pythonExe = Join-Path $Root "python.exe"
    $markerPath = Join-Path $Root $markerName
    if (-not (Test-Path -LiteralPath $pythonExe -PathType Leaf) -or
        -not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        return $false
    }
    try {
        $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
        return $marker.schemaVersion -eq 1 -and
            $marker.pythonVersion -eq $pythonVersion -and
            $marker.architecture -eq "amd64" -and
            $marker.requirementsSha256 -eq (Get-RequirementsHash)
    } catch {
        return $false
    }
}

<# Imports every core dependency with the bundled interpreter before packaging. #>
function Test-RuntimeImports {
    param([Parameter(Mandatory)][string]$Root)

    $pythonExe = Join-Path $Root "python.exe"
    if (-not (Test-Path -LiteralPath $pythonExe -PathType Leaf)) {
        throw "Bundled Python executable is missing before import validation: $pythonExe"
    }
    $env:HF_HUB_OFFLINE = "1"
    $env:MODELSCOPE_OFFLINE = "1"
    $validationCode = "import sys, funasr, modelscope, torch, torchaudio; assert sys.version_info[:3] == (3, 12, 9); assert funasr.__version__ == '1.4.1'; assert modelscope.__version__ == '1.39.1'; assert torch.__version__.startswith('2.13.0'); assert torchaudio.__version__.startswith('2.11.0'); print('bundled_runtime_ok')"
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $pythonExe
    $startInfo.Arguments = '-c "' + $validationCode + '"'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    try {
        [void]$process.Start()
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        if ($stdout) {
            Write-Host $stdout.Trim()
        }
        if ($process.ExitCode -ne 0) {
            throw "Bundled Python runtime import validation failed: $($stderr.Trim())"
        }
    } finally {
        $process.Dispose()
    }
}

if (-not (Test-Path -LiteralPath $requirementsPath -PathType Leaf)) {
    throw "Local ASR requirements file is missing"
}
Assert-RuntimePath -Path $runtimeRoot

if (-not $Force -and (Test-CurrentRuntime -Root $runtimeRoot)) {
    Test-RuntimeImports -Root $runtimeRoot
    Write-Host "Bundled Python runtime is ready: $runtimeRoot"
    exit 0
}
if ($CheckOnly) {
    throw "Bundled Python runtime is missing or stale"
}

New-Item -ItemType Directory -Force -Path $runtimeParent, $cacheRoot | Out-Null
$pythonArchivePath = Join-Path $cacheRoot $pythonArchiveName
$getPipPath = Join-Path $cacheRoot "get-pip.py"
Get-VerifiedDownload -Uri $pythonArchiveUrl -Destination $pythonArchivePath -ExpectedSha256 $pythonArchiveSha256
Get-VerifiedDownload -Uri $getPipUrl -Destination $getPipPath -ExpectedSha256 $getPipSha256

$stagingRoot = Join-Path $runtimeParent (".python-staging-" + $PID)
Assert-RuntimePath -Path $stagingRoot
if (Test-Path -LiteralPath $stagingRoot) {
    Remove-Item -LiteralPath $stagingRoot -Recurse -Force
}

try {
    New-Item -ItemType Directory -Force -Path $stagingRoot | Out-Null
    Expand-Archive -LiteralPath $pythonArchivePath -DestinationPath $stagingRoot -Force
    @(
        "python312.zip"
        "."
        "Lib\site-packages"
        "import site"
    ) | Set-Content -LiteralPath (Join-Path $stagingRoot "python312._pth") -Encoding Ascii

    $stagingPython = Join-Path $stagingRoot "python.exe"
    & $stagingPython $getPipPath --no-warn-script-location --disable-pip-version-check
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to bootstrap pip in bundled Python"
    }
    $pipCacheRoot = Join-Path $cacheRoot "pip"
    New-Item -ItemType Directory -Force -Path $pipCacheRoot | Out-Null
    & $stagingPython -m pip install --disable-pip-version-check --no-warn-script-location --cache-dir $pipCacheRoot --timeout 60 --retries 3 setuptools==84.0.0 wheel==0.47.0
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to install bundled Python build tooling"
    }
    & $stagingPython -m pip install --disable-pip-version-check --no-warn-script-location --no-build-isolation --cache-dir $pipCacheRoot --timeout 60 --retries 3 -r $requirementsPath
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to install local ASR runtime dependencies"
    }

    Get-ChildItem -LiteralPath $stagingRoot -Recurse -Directory -Filter "__pycache__" |
        Sort-Object FullName -Descending |
        Remove-Item -Recurse -Force
    Get-ChildItem -LiteralPath $stagingRoot -Recurse -File |
        Where-Object { $_.Extension -in @(".pyc", ".pyo") } |
        Remove-Item -Force

    [ordered]@{
        schemaVersion = 1
        pythonVersion = $pythonVersion
        architecture = "amd64"
        requirementsSha256 = (Get-RequirementsHash)
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $stagingRoot $markerName) -Encoding UTF8

    Test-RuntimeImports -Root $stagingRoot
    if (Test-Path -LiteralPath $runtimeRoot) {
        Assert-RuntimePath -Path $runtimeRoot
        Remove-Item -LiteralPath $runtimeRoot -Recurse -Force
    }
    Move-Item -LiteralPath $stagingRoot -Destination $runtimeRoot
    Write-Host "Bundled Python runtime prepared: $runtimeRoot"
} catch {
    if (Test-Path -LiteralPath $stagingRoot) {
        Remove-Item -LiteralPath $stagingRoot -Recurse -Force
    }
    throw
}
