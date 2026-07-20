<#
.SYNOPSIS
  Build the agent_runtime_ffi shared library for Windows and/or Linux.

.DESCRIPTION
  Produces:
    target\release\agent_runtime.dll                           (Windows MSVC)
    target\x86_64-unknown-linux-gnu\release\libagent_runtime.so (Linux, via zig as cross linker)

  After a successful build the artifacts are also staged under
  dist\runtime\ so downstream tooling (docker compose build for the
  runtime-service image) can pick them up by a stable path.

.PARAMETER Targets
  Which targets to build. One of:
    windows   - only the local Windows .dll
    linux     - only the Linux .so (cross compile via zig)
    all       - both (default)

.PARAMETER Release
  Build in release mode (default true). Pass -Release:$false for debug.

.EXAMPLE
  .\scripts\build-runtime.ps1                # all targets, release
  .\scripts\build-runtime.ps1 -Targets linux # just the .so for the docker image
#>
[CmdletBinding()]
param(
    [ValidateSet('windows', 'linux', 'all')]
    [string]$Targets = 'all',
    [bool]$Release = $true
)

$ErrorActionPreference = 'Stop'

# Repo root = parent of the scripts folder this file lives in.
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$crate = 'agent-runtime-ffi'
$profileFlag = if ($Release) { '--release' } else { '' }
$profileDir = if ($Release) { 'release' } else { 'debug' }
$dist = Join-Path $repo 'dist\runtime'
New-Item -ItemType Directory -Force -Path $dist | Out-Null

function Resolve-ZigOnPath {
    $cmd = Get-Command zig -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    # WinGet drops zig under %LOCALAPPDATA%\Microsoft\WinGet\Packages\zig.zig_*.
    $candidates = Get-ChildItem `
        -Path "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" `
        -Filter 'zig.exe' -Recurse -ErrorAction SilentlyContinue -Depth 6 |
        Sort-Object LastWriteTime -Descending
    if ($candidates) { return $candidates[0].FullName }
    return $null
}

function Build-Windows {
    Write-Host ""
    Write-Host "=== building Windows .dll ===" -ForegroundColor Cyan
    & cargo build $profileFlag -p $crate
    if ($LASTEXITCODE -ne 0) { throw "windows cargo build failed ($LASTEXITCODE)" }

    $dll = Join-Path $repo "target\$profileDir\agent_runtime.dll"
    if (-not (Test-Path $dll)) { throw "expected artifact missing: $dll" }
    Copy-Item -Force $dll (Join-Path $dist 'agent_runtime.dll')
    $importLib = Join-Path $repo "target\$profileDir\agent_runtime.dll.lib"
    if (Test-Path $importLib) {
        Copy-Item -Force $importLib (Join-Path $dist 'agent_runtime.dll.lib')
    }
    Write-Host "  -> $dll" -ForegroundColor Green
}

function Build-Linux {
    Write-Host ""
    Write-Host "=== building Linux .so (x86_64-unknown-linux-gnu via zig) ===" -ForegroundColor Cyan

    $zig = Resolve-ZigOnPath
    if (-not $zig) {
        throw "zig not found on PATH. Install via 'winget install -e --id zig.zig' and retry."
    }
    $zigDir = Split-Path -Parent $zig
    if ($env:PATH -notlike "*$zigDir*") { $env:PATH = "$zigDir;$env:PATH" }
    Write-Host "  zig: $zig"

    # Verify the rustc target is installed; install on demand.
    $installed = (rustup target list --installed) -split "`n"
    if ($installed -notcontains 'x86_64-unknown-linux-gnu') {
        Write-Host "  installing rustc target x86_64-unknown-linux-gnu..."
        rustup target add x86_64-unknown-linux-gnu | Out-Null
    }

    # zig cc -target <triple> emits a Linux ELF and bundles its own libc; we
    # use zig as both the rustc linker AND the cc/cxx/ar that cc-rs (used by
    # build scripts of -sys crates like ring/zstd) shells out to. cc-rs always
    # appends --target=<rustc-triple> for Clang-family compilers and zig 0.16
    # parses that as a (different-syntax) zig target query, so we filter it
    # via a PowerShell shim and pin our own -target.
    $shimDir = Join-Path $repo 'scripts'
    function Write-Shim($name, $body) {
        $cmdPath = Join-Path $shimDir "$name.cmd"
        $ps1Path = Join-Path $shimDir "$name.ps1"
        $cmdBody = "@echo off`r`npowershell -NoProfile -ExecutionPolicy Bypass -File `"%~dp0$name.ps1`" %*`r`nexit /b %ERRORLEVEL%"
        Set-Content -Path $cmdPath -Value $cmdBody -Encoding ASCII -NoNewline
        Set-Content -Path $ps1Path -Value $body -Encoding ASCII -NoNewline
    }
    $ccBody  = @'
$filtered = @($args | Where-Object { $_ -notlike '--target=*' })
& zig cc -target x86_64-linux-gnu @filtered
exit $LASTEXITCODE
'@
    $cxxBody = @'
$filtered = @($args | Where-Object { $_ -notlike '--target=*' })
& zig c++ -target x86_64-linux-gnu @filtered
exit $LASTEXITCODE
'@
    $arBody  = @'
& zig ar @args
exit $LASTEXITCODE
'@
    Write-Shim 'zig-cc-linux-gnu'  $ccBody
    Write-Shim 'zig-cxx-linux-gnu' $cxxBody
    Write-Shim 'zig-ar'            $arBody
    $env:CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = Join-Path $repo 'scripts\zig-cc-linux-gnu.cmd'
    $env:CC_x86_64_unknown_linux_gnu  = Join-Path $repo 'scripts\zig-cc-linux-gnu.cmd'
    $env:CXX_x86_64_unknown_linux_gnu = Join-Path $repo 'scripts\zig-cxx-linux-gnu.cmd'
    $env:AR_x86_64_unknown_linux_gnu  = Join-Path $repo 'scripts\zig-ar.cmd'

    & cargo build $profileFlag --target x86_64-unknown-linux-gnu -p $crate
    if ($LASTEXITCODE -ne 0) { throw "linux cargo build failed ($LASTEXITCODE)" }

    $so = Join-Path $repo "target\x86_64-unknown-linux-gnu\$profileDir\libagent_runtime.so"
    if (-not (Test-Path $so)) { throw "expected artifact missing: $so" }
    Copy-Item -Force $so (Join-Path $dist 'libagent_runtime.so')

    # Also stage to target\release\libagent_runtime.so so the existing
    # Dockerfile.runtime COPY directive (which expects that exact path)
    # works without modification.
    $native = Join-Path $repo "target\$profileDir\libagent_runtime.so"
    New-Item -ItemType Directory -Force -Path (Split-Path $native) | Out-Null
    Copy-Item -Force $so $native
    Write-Host "  -> $so" -ForegroundColor Green
    Write-Host "  -> $native (mirrored for Dockerfile.runtime)" -ForegroundColor Green
}

switch ($Targets) {
    'windows' { Build-Windows }
    'linux'   { Build-Linux }
    'all'     { Build-Windows; Build-Linux }
}

Write-Host ""
Write-Host "Artifacts staged under $dist" -ForegroundColor Green
Get-ChildItem $dist | Format-Table Name, Length, LastWriteTime -AutoSize
