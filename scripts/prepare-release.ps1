<#
.SYNOPSIS
  Prepare native Runtime release packages for Windows and Linux.

.DESCRIPTION
  Builds, validates, and packages the Runtime FFI artifacts that SDKs download
  or embed at install time. macOS packages are intentionally not produced by
  this script because the local release machine does not provide an Apple build
  environment.

  The script produces zip files under dist\releases:
    orbit-runtime-runtime-vX.Y.Z-windows-x86_64.zip
    orbit-runtime-runtime-vX.Y.Z-linux-x86_64.zip

  Each package contains:
    include\agent_runtime.h
    bin\agent_runtime.dll                    (Windows)
    lib\agent_runtime.dll.lib                (Windows, when emitted by MSVC)
    lib\libagent_runtime.so                  (Linux)
    LICENSE
    NOTICE
    README.md

.PARAMETER Targets
  Which platform packages to prepare. One of: windows, linux, all.

.PARAMETER Version
  Release version. Defaults to agent_runtime_ffi/Cargo.toml package version.

.PARAMETER PackageName
  Prefix used in package filenames and internal package folders.

.PARAMETER OutputDir
  Directory that receives final zip files. Defaults to dist\releases.

.PARAMETER SkipBuild
  Package artifacts already staged under dist\runtime without rebuilding.

.PARAMETER Clean
  Remove existing output and staging directories before packaging.

.EXAMPLE
  .\scripts\prepare-release.ps1

.EXAMPLE
  .\scripts\prepare-release.ps1 -Targets windows -Version 0.4.3

.EXAMPLE
  .\scripts\prepare-release.ps1 -Targets linux -SkipBuild
#>
[CmdletBinding()]
param(
    [ValidateSet('windows', 'linux', 'all')]
    [string]$Targets = 'all',

    [string]$Version,

    [ValidateNotNullOrEmpty()]
    [string]$PackageName = 'orbit-runtime',

    [string]$OutputDir,

    [switch]$SkipBuild,

    [switch]$Clean
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$runtimeDist = Join-Path $repo 'dist\runtime'
$stagingRoot = Join-Path $repo 'dist\release-staging'
if (-not $OutputDir) {
    $OutputDir = Join-Path $repo 'dist\releases'
}

function Get-RuntimeVersion {
    $manifest = Join-Path $repo 'agent_runtime_ffi\Cargo.toml'
    if (-not (Test-Path $manifest)) {
        throw "missing manifest: $manifest"
    }

    $content = Get-Content -Raw -Path $manifest
    $match = [regex]::Match($content, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $match.Success) {
        throw "could not read package version from $manifest"
    }
    return $match.Groups[1].Value
}

function Get-SelectedTargets {
    switch ($Targets) {
        'windows' { return @('windows') }
        'linux' { return @('linux') }
        default { return @('windows', 'linux') }
    }
}

function New-Directory($path) {
    New-Item -ItemType Directory -Force -Path $path | Out-Null
}

function Copy-RequiredFile($source, $destination) {
    if (-not (Test-Path $source)) {
        throw "required release artifact missing: $source"
    }
    New-Directory (Split-Path -Parent $destination)
    Copy-Item -Force -Path $source -Destination $destination
}

function Write-PackageReadme($path, $platformId, $versionLabel, $artifacts) {
    $artifactLines = ($artifacts | ForEach-Object { "- $_" }) -join "`r`n"
    $body = @"
# Orbit Runtime Native Package

Package: $PackageName
Version: $versionLabel
Platform: $platformId

This package contains the native Runtime FFI library and the matching C header.
Host SDKs should load the library from this package and call
agent_runtime_abi_version_v1 before using the ABI.

Workflow JSON schemas, examples, API documentation, and matching SDK sources
are included. Read docs/workflow-json-update.zh-CN.md for breaking changes.
Public schemas: schemas/resource.schema.json and schemas/logic.schema.json.
Source syntax: docs/zh/workflow-json-source-syntax.md (also available in en).
Distribute these files with the native library. Schemas describe normalized
structures; validate authored source through workflow.validate.
Old script APIs and both standalone Studio pages are disabled.
Load this package's native library explicitly: SDK automatic downloads still
refer to the published release and do not retrieve a local branch build.
See build-info.json for the source branch, commit, and dirty-worktree status.

Artifacts:
$artifactLines

macOS artifacts are not included in this release package.
"@
    Set-Content -Path $path -Value $body -Encoding UTF8
}

function New-ReleasePackage($platform) {
    $versionLabel = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
    $platformId = switch ($platform) {
        'windows' { 'windows-x86_64' }
        'linux' { 'linux-x86_64' }
    }

    $packageRootName = "$PackageName-runtime-$versionLabel-$platformId"
    $packageRoot = Join-Path $stagingRoot $packageRootName
    $zipPath = Join-Path $OutputDir "$packageRootName.zip"

    if (Test-Path $packageRoot) {
        $resolvedStage = [IO.Path]::GetFullPath($packageRoot)
        $stagePrefix = [IO.Path]::GetFullPath($stagingRoot).TrimEnd('\') + '\'
        if (-not $resolvedStage.StartsWith($stagePrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "staging path escaped its root: $resolvedStage"
        }
        Remove-Item -Recurse -Force -LiteralPath $resolvedStage
    }
    New-Directory $packageRoot

    Copy-RequiredFile `
        (Join-Path $repo 'agent_runtime_ffi\include\agent_runtime.h') `
        (Join-Path $packageRoot 'include\agent_runtime.h')
    Copy-RequiredFile `
        (Join-Path $repo 'LICENSE') `
        (Join-Path $packageRoot 'LICENSE')
    Copy-RequiredFile `
        (Join-Path $repo 'NOTICE') `
        (Join-Path $packageRoot 'NOTICE')

    $packagedArtifacts = @('include/agent_runtime.h', 'LICENSE', 'NOTICE')

    if ($platform -eq 'windows') {
        Copy-RequiredFile `
            (Join-Path $runtimeDist 'agent_runtime.dll') `
            (Join-Path $packageRoot 'bin\agent_runtime.dll')
        $packagedArtifacts += 'bin/agent_runtime.dll'

        $importLib = Join-Path $runtimeDist 'agent_runtime.dll.lib'
        if (Test-Path $importLib) {
            Copy-RequiredFile `
                $importLib `
                (Join-Path $packageRoot 'lib\agent_runtime.dll.lib')
            $packagedArtifacts += 'lib/agent_runtime.dll.lib'
        }
    }

    if ($platform -eq 'linux') {
        Copy-RequiredFile `
            (Join-Path $runtimeDist 'libagent_runtime.so') `
            (Join-Path $packageRoot 'lib\libagent_runtime.so')
        $packagedArtifacts += 'lib/libagent_runtime.so'
    }

    foreach ($schema in @('logic.schema.json', 'resource.schema.json')) {
        Copy-RequiredFile (Join-Path $repo "corework\src\workflow\$schema") (Join-Path $packageRoot "schemas\$schema")
    }
    Copy-RequiredFile (Join-Path $repo 'agent_runtime_ffi\docs\workflow-json-update.zh-CN.md') (Join-Path $packageRoot 'docs\workflow-json-update.zh-CN.md')
    foreach ($language in @('zh', 'en')) {
        Copy-RequiredFile (Join-Path $repo "agent_runtime_ffi\docs\$language\12-workflow-resource-api.md") (Join-Path $packageRoot "docs\$language\12-workflow-resource-api.md")
        Copy-RequiredFile (Join-Path $repo "ai-assistant\prompts\$language\workflow_tool_rules.md") (Join-Path $packageRoot "docs\$language\workflow-json-source-syntax.md")
    }
    New-Directory (Join-Path $packageRoot 'examples\workflows')
    Get-ChildItem -LiteralPath (Join-Path $repo 'examples\workflows') -File | ForEach-Object {
        Copy-RequiredFile $_.FullName (Join-Path $packageRoot "examples\workflows\$($_.Name)")
    }
    $sdkFiles = @(git -C $repo ls-files -- sdk/runtime)
    if ($LASTEXITCODE -ne 0 -or $sdkFiles.Count -eq 0) { throw 'could not enumerate Runtime SDK sources' }
    foreach ($sdkFile in $sdkFiles) {
        Copy-RequiredFile (Join-Path $repo $sdkFile) (Join-Path $packageRoot ("sdk/" + $sdkFile.Substring('sdk/runtime/'.Length)))
    }
    $nativePath = if ($platform -eq 'windows') { Join-Path $packageRoot 'bin\agent_runtime.dll' } else { Join-Path $packageRoot 'lib\libagent_runtime.so' }
    $buildInfo = [ordered]@{
        schema = 'orbit-runtime-build/v1'
        runtime_version = $Version.TrimStart('v')
        platform = $platformId
        branch = (git -C $repo branch --show-current)
        commit = (git -C $repo rev-parse HEAD)
        worktree_dirty = [bool](@(git -C $repo status --porcelain --untracked-files=normal).Count)
        built_utc = [DateTime]::UtcNow.ToString('o')
        native_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $nativePath).Hash.ToLowerInvariant()
        workflow_schema = 'orbit.workflow.resource/v1'
        compiler_version = ([regex]::Match((Get-Content -Raw (Join-Path $repo 'corework/src/workflow/logic.rs')), 'COMPILER_VERSION: &str = "([^"]+)"')).Groups[1].Value
    }
    $buildInfo | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot 'build-info.json') -Encoding UTF8
    $packagedArtifacts += @('schemas/', 'examples/workflows/', 'docs/', 'sdk/', 'build-info.json')
    Copy-RequiredFile (Join-Path $repo 'scripts/verify-workflow-json.py') (Join-Path $packageRoot 'scripts/verify-workflow-json.py')
    $packagedArtifacts += 'scripts/verify-workflow-json.py'

    Write-PackageReadme `
        (Join-Path $packageRoot 'README.md') `
        $platformId `
        $versionLabel `
        $packagedArtifacts

    if (Test-Path $zipPath) {
        Remove-Item -Force -Path $zipPath
    }
    Compress-Archive -Path $packageRoot -DestinationPath $zipPath -CompressionLevel Optimal

    $hash = Get-FileHash -Algorithm SHA256 -Path $zipPath
    Set-Content -Path "$zipPath.sha256" -Encoding ASCII -Value "$($hash.Hash.ToLowerInvariant())  $(Split-Path -Leaf $zipPath)"

    [pscustomobject]@{
        Platform = $platformId
        Package = $zipPath
        Sha256 = "$zipPath.sha256"
    }
}

Set-Location $repo

if (-not $Version) {
    $Version = Get-RuntimeVersion
}

if ($Clean) {
    if (Test-Path $OutputDir) {
        Remove-Item -Recurse -Force -Path $OutputDir
    }
    if (Test-Path $stagingRoot) {
        Remove-Item -Recurse -Force -Path $stagingRoot
    }
}

New-Directory $runtimeDist
New-Directory $OutputDir
New-Directory $stagingRoot

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build-runtime.ps1') -Targets $Targets -Release:$true
    if ($LASTEXITCODE -ne 0) {
        throw "runtime build failed ($LASTEXITCODE)"
    }
}

$results = foreach ($target in Get-SelectedTargets) {
    New-ReleasePackage $target
}

Write-Host ""
Write-Host "Release packages prepared under $OutputDir" -ForegroundColor Green
$results | Format-Table Platform, Package, Sha256 -AutoSize
