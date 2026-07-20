<#
.SYNOPSIS
  Download and verify the native Orbit Runtime package for this platform.

.EXAMPLE
  .\sdk\runtime\scripts\fetch-native-runtime.ps1

.EXAMPLE
  .\sdk\runtime\scripts\fetch-native-runtime.ps1 -Platform linux-x86_64 -OutputDir .runtime
#>
[CmdletBinding()]
param(
    [string]$Platform,
    [string]$OutputDir = ".runtime",
    [switch]$Force
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$runtimeDir = Split-Path -Parent $scriptDir
$manifestPath = Join-Path $runtimeDir "release_manifest.json"
$manifest = Get-Content -Raw -Path $manifestPath | ConvertFrom-Json

if (-not $Platform) {
    $arch = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq "X64") { "x86_64" } else { "" }
    if ($IsWindows -or $env:OS -eq "Windows_NT") {
        $Platform = "windows-$arch"
    } elseif ($IsLinux) {
        $Platform = "linux-$arch"
    }
}

if (-not $Platform) {
    throw "unsupported platform. Pass -Platform windows-x86_64 or linux-x86_64."
}

$asset = $manifest.assets.$Platform
if (-not $asset) {
    throw "release manifest does not contain platform '$Platform'"
}

$tag = $manifest.release_tag
$repo = $manifest.repository
$url = "https://github.com/$repo/releases/download/$tag/$($asset.archive)"
$outputRoot = Resolve-Path -Path (New-Item -ItemType Directory -Force -Path $OutputDir)
$archiveDir = Join-Path $outputRoot "_downloads"
$packageDir = Join-Path $outputRoot "$Platform\$tag"
$archivePath = Join-Path $archiveDir $asset.archive
$libraryPath = Join-Path $packageDir $asset.library

if ((Test-Path $libraryPath) -and -not $Force) {
    Write-Output $libraryPath
    exit 0
}

New-Item -ItemType Directory -Force -Path $archiveDir | Out-Null
if ($Force -or -not (Test-Path $archivePath)) {
    $headers = @{}
    $token = $env:GITHUB_TOKEN
    if (-not $token) { $token = $env:GH_TOKEN }
    if ($token) {
        $headers["Authorization"] = "Bearer $token"
        $headers["Accept"] = "application/vnd.github+json"
        $headers["User-Agent"] = "orbit-runtime-sdk"
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/tags/$tag" -Headers $headers
        $assetInfo = $release.assets | Where-Object { $_.name -eq $asset.archive } | Select-Object -First 1
        if (-not $assetInfo) {
            throw "release asset not found: $($asset.archive)"
        }
        $downloadHeaders = @{
            "Authorization" = "Bearer $token"
            "Accept" = "application/octet-stream"
            "User-Agent" = "orbit-runtime-sdk"
        }
        Invoke-WebRequest -Uri $assetInfo.url -OutFile $archivePath -Headers $downloadHeaders
    } else {
        Invoke-WebRequest -Uri $url -OutFile $archivePath
    }
}

$actual = (Get-FileHash -Algorithm SHA256 -Path $archivePath).Hash.ToLowerInvariant()
if ($actual -ne $asset.sha256.ToLowerInvariant()) {
    Remove-Item -Force -Path $archivePath
    throw "checksum mismatch for $($asset.archive): expected $($asset.sha256), got $actual"
}

if (Test-Path $packageDir) {
    Remove-Item -Recurse -Force -Path $packageDir
}
New-Item -ItemType Directory -Force -Path $packageDir | Out-Null
Expand-Archive -Force -Path $archivePath -DestinationPath $packageDir

$nested = Get-ChildItem -Directory -Path $packageDir | Select-Object -First 1
if ($nested -and (Test-Path (Join-Path $nested.FullName $asset.library))) {
    Get-ChildItem -Path $nested.FullName | Move-Item -Destination $packageDir
    Remove-Item -Recurse -Force -Path $nested.FullName
}

if (-not (Test-Path $libraryPath)) {
    throw "runtime library missing after extraction: $libraryPath"
}

Write-Output $libraryPath
