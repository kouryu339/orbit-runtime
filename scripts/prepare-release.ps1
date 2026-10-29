<#
.SYNOPSIS
  Prepare Runtime, standalone Svelte JS, and matching SDK release packages.

.DESCRIPTION
  Builds, validates, and packages the Runtime FFI artifacts that SDKs download
  or embed at install time. macOS packages are intentionally not produced by
  this script because the local release machine does not provide an Apple build
  environment.

  The script produces zip files under dist\releases:
    orbit-runtime-runtime-vX.Y.Z-windows-x86_64.zip
    orbit-runtime-runtime-vX.Y.Z-linux-x86_64.zip
    orbit-runtime-frontend-svelte-vX.Y.Z.zip
    orbit-runtime-sdk-vX.Y.Z.zip
    orbit-runtime-browser-vX.Y.Z.zip
    orbit-runtime-browser-vX.Y.Z.tgz

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

.PARAMETER FrontendOnly
  Build and package only the standalone Svelte JS and its distribution archive.

.PARAMETER SkipFrontendBuild
  Reuse the already built and verified ui/svelte/dist-library output.

.PARAMETER SkipBrowserBuild
  Reuse the already built and verified sdk/runtime/js/dist output.

.PARAMETER Clean
  Remove existing output and staging directories before packaging.

.PARAMETER LocalPreview
  Mark local preview packages and preserve stable SDK online-download pins.

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

    [switch]$FrontendOnly,

    [switch]$SkipFrontendBuild,

    [switch]$SkipBrowserBuild,

    [switch]$Clean,

    [switch]$LocalPreview
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$runtimeDist = Join-Path $repo 'dist\runtime'
$stagingRoot = Join-Path $repo 'dist\release-staging'
if (-not $OutputDir) {
    $OutputDir = Join-Path $repo 'dist\releases'
}
$OutputDir = [IO.Path]::GetFullPath($OutputDir)

function Remove-GeneratedDirectory($path, $allowedRoot) {
    $fullPath = [IO.Path]::GetFullPath($path)
    $prefix = [IO.Path]::GetFullPath($allowedRoot).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if (-not $fullPath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing to remove directory outside generated root: $fullPath"
    }
    if (Test-Path -LiteralPath $fullPath) {
        Remove-Item -Recurse -Force -LiteralPath $fullPath
    }
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
Distribution: $(if ($LocalPreview) { 'local-preview' } else { 'release' })

This package contains the native Runtime FFI library and the matching C header.
Host SDKs should load the library from this package and call
agent_runtime_abi_version_v1 before using the ABI.

Workflow JSON schemas, examples, and API documentation
are included. Read docs/workflow-json-update.zh-CN.md for breaking changes.
Public schemas: schemas/resource.schema.json and schemas/logic.schema.json.
Source syntax: docs/zh/workflow-json-source-syntax.md (also available in en).
Distribute these files with the native library. Schemas describe normalized
structures; validate authored source through workflow.validate.
Old script APIs and both standalone Studio pages are disabled.
Dynamic clusters: runtime.add_agent_cluster / runtime.update_agent_cluster.
Updates require all cluster conversations to be closed. Read
docs/zh/13-dynamic-agent-clusters.md (also available in en) and
docs/dynamic-clusters-update.zh-CN.md for the contract and local build notes.
Skill candidate transactions: runtime.read_skill / runtime.validate_skill_transaction /
runtime.commit_skill_transaction. See docs/zh/14-skill-candidate-transactions.md
(also available in en) and schemas/skill-transaction.schema.json.
Existing Workflow edits become drafts; active runs block edits to their complete
reference chain. Detect these features through capabilities rather than base version.
Dynamic RPC additions: runtime.add_rpc_endpoint accepts an external gRPC endpoint
after start. Existing endpoint IDs and tool names cannot be replaced. See
docs/zh/17-dynamic-rpc-additions.md (also available in en) and
schemas/rpc-endpoint-add.schema.json. Hosts persist accepted endpoint configuration.
Matching SDK sources are distributed separately as $PackageName-sdk-$versionLabel.zip.
Extract that archive's sdk directory here to use the included verification scripts.
The Svelte frontend is a separate platform-independent release asset.
JS/TS browser hosts use the separate browser ZIP or installable TGZ with WASM.
See docs/browser-wasm-usage.zh-CN.md for capabilities and deployment constraints.
See build-info.json for the source branch, commit, and dirty-worktree status.
$(if ($LocalPreview) { 'Local preview: load this native library by explicit path. SDK online-download defaults still point to the published stable release.' })

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
    Copy-RequiredFile (Join-Path $repo 'agent_runtime_ffi/docs/skill-transaction.schema.json') (Join-Path $packageRoot 'schemas/skill-transaction.schema.json')
    Copy-RequiredFile (Join-Path $repo 'agent_runtime_ffi/docs/rpc-endpoint-add.schema.json') (Join-Path $packageRoot 'schemas/rpc-endpoint-add.schema.json')
    Copy-RequiredFile (Join-Path $repo 'agent_runtime_ffi\docs\workflow-json-update.zh-CN.md') (Join-Path $packageRoot 'docs\workflow-json-update.zh-CN.md')
    Copy-RequiredFile (Join-Path $repo 'agent_runtime_ffi\docs\dynamic-clusters-update.zh-CN.md') (Join-Path $packageRoot 'docs\dynamic-clusters-update.zh-CN.md')
    $releaseNotes = Join-Path $repo "agent_runtime_ffi/docs/releases/v$($Version.TrimStart('v'))/release-notes.md"
    if (Test-Path -LiteralPath $releaseNotes) { Copy-RequiredFile $releaseNotes (Join-Path $packageRoot 'docs/release-notes.md') }
    Copy-RequiredFile (Join-Path $repo 'sdk/runtime/js/USAGE.zh-CN.md') (Join-Path $packageRoot 'docs/browser-wasm-usage.zh-CN.md')
    foreach ($language in @('zh', 'en')) {
        Copy-RequiredFile (Join-Path $repo "agent_runtime_ffi\docs\$language\12-workflow-resource-api.md") (Join-Path $packageRoot "docs\$language\12-workflow-resource-api.md")
        Copy-RequiredFile (Join-Path $repo "agent_runtime_ffi\docs\$language\13-dynamic-agent-clusters.md") (Join-Path $packageRoot "docs\$language\13-dynamic-agent-clusters.md")
        Copy-RequiredFile (Join-Path $repo "agent_runtime_ffi/docs/$language/14-skill-candidate-transactions.md") (Join-Path $packageRoot "docs/$language/14-skill-candidate-transactions.md")
        Copy-RequiredFile (Join-Path $repo "agent_runtime_ffi/docs/$language/17-dynamic-rpc-additions.md") (Join-Path $packageRoot "docs/$language/17-dynamic-rpc-additions.md")
        Copy-RequiredFile (Join-Path $repo "ai-assistant\prompts\$language\workflow_tool_rules.md") (Join-Path $packageRoot "docs\$language\workflow-json-source-syntax.md")
    }
    New-Directory (Join-Path $packageRoot 'examples\workflows')
    Get-ChildItem -LiteralPath (Join-Path $repo 'examples\workflows') -File | ForEach-Object {
        Copy-RequiredFile $_.FullName (Join-Path $packageRoot "examples\workflows\$($_.Name)")
    }
    $nativePath = if ($platform -eq 'windows') { Join-Path $packageRoot 'bin\agent_runtime.dll' } else { Join-Path $packageRoot 'lib\libagent_runtime.so' }
    $buildInfo = [ordered]@{
        schema = 'orbit-runtime-build/v1'
        runtime_version = $Version.TrimStart('v')
        distribution = $(if ($LocalPreview) { 'local-preview' } else { 'release' })
        platform = $platformId
        branch = (git -C $repo branch --show-current)
        commit = (git -C $repo rev-parse HEAD)
        worktree_dirty = [bool](@(git -C $repo status --porcelain --untracked-files=normal).Count)
        built_utc = [DateTime]::UtcNow.ToString('o')
        native_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $nativePath).Hash.ToLowerInvariant()
        cargo_lock_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $repo 'Cargo.lock')).Hash.ToLowerInvariant()
        workflow_schema = 'orbit.workflow.resource/v1'
        compiler_version = ([regex]::Match((Get-Content -Raw (Join-Path $repo 'corework/src/workflow/logic.rs')), 'COMPILER_VERSION: &str = "([^"]+)"')).Groups[1].Value
    }
    $buildInfo | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot 'build-info.json') -Encoding UTF8
    $packagedArtifacts += @('schemas/', 'examples/workflows/', 'docs/', 'build-info.json')
    Copy-RequiredFile (Join-Path $repo 'scripts/verify-workflow-json.py') (Join-Path $packageRoot 'scripts/verify-workflow-json.py')
    $packagedArtifacts += 'scripts/verify-workflow-json.py'
    Copy-RequiredFile (Join-Path $repo 'scripts/verify-dynamic-clusters.py') (Join-Path $packageRoot 'scripts\verify-dynamic-clusters.py')
    $packagedArtifacts += 'scripts/verify-dynamic-clusters.py'
    Copy-RequiredFile (Join-Path $repo 'scripts/verify-runtime-transactions.py') (Join-Path $packageRoot 'scripts/verify-runtime-transactions.py')
    $packagedArtifacts += 'scripts/verify-runtime-transactions.py'

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

function Complete-Archive($root, $name, $platform) {
    $zip = Join-Path $OutputDir "$name.zip"
    if (Test-Path -LiteralPath $zip) { Remove-Item -Force -LiteralPath $zip }
    Compress-Archive -LiteralPath $root -DestinationPath $zip -CompressionLevel Optimal
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$zip.sha256" -Encoding ASCII -Value "$hash  $name.zip"
    [pscustomobject]@{ Platform = $platform; Package = $zip; Sha256 = "$zip.sha256" }
}

function New-FrontendPackage {
    if (-not $SkipFrontendBuild) {
        Push-Location (Join-Path $repo 'ui/lit')
        try { & npm.cmd run build | Out-Host; if ($LASTEXITCODE -ne 0) { throw 'Lit contracts build failed' } }
        finally { Pop-Location }
        Push-Location (Join-Path $repo 'ui/svelte')
        try {
            & npm.cmd run check:styles | Out-Host
            if ($LASTEXITCODE -ne 0) { throw 'Svelte styles are stale' }
            & npm.cmd run build:library | Out-Host
            if ($LASTEXITCODE -ne 0) { throw 'Svelte library build failed' }
            & npm.cmd run verify:library | Out-Host
            if ($LASTEXITCODE -ne 0) { throw 'Svelte library verification failed' }
        } finally { Pop-Location }
    }
    $name = "$PackageName-frontend-svelte-v$($Version.TrimStart('v'))"
    $root = Join-Path $stagingRoot $name
    Remove-GeneratedDirectory $root $stagingRoot
    New-Directory $root
    foreach ($file in @('orbit-conversation-svelte.js', 'THIRD_PARTY_NOTICES.txt')) {
        Copy-RequiredFile (Join-Path $repo "ui/svelte/dist-library/$file") (Join-Path $root $file)
    }
    Copy-RequiredFile (Join-Path $repo 'ui/svelte/RELEASE-README.md') (Join-Path $root 'README.md')
    foreach ($file in @('LICENSE', 'NOTICE')) { Copy-RequiredFile (Join-Path $repo $file) (Join-Path $root $file) }
    $files = [ordered]@{}
    Get-ChildItem -LiteralPath $root -File | ForEach-Object { $files[$_.Name] = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant() }
    [ordered]@{
        schema = 'orbit-runtime-frontend/v1'
        runtime_version = $Version.TrimStart('v')
        entry = 'orbit-conversation-svelte.js'
        format = 'esm'
        styles = 'embedded'
        commit = (git -C $repo rev-parse HEAD)
        files = $files
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'manifest.json') -Encoding UTF8
    # Also expose the exact JS directly, without requiring archive extraction.
    $js = Join-Path $OutputDir "$name.js"
    Copy-RequiredFile (Join-Path $root 'orbit-conversation-svelte.js') $js
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $js).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$js.sha256" -Encoding ASCII -Value "$hash  $name.js"
    Complete-Archive $root $name 'svelte-esm'
}

function New-SdkPackage {
    $name = "$PackageName-sdk-v$($Version.TrimStart('v'))"
    $root = Join-Path $stagingRoot $name
    Remove-GeneratedDirectory $root $stagingRoot
    New-Directory $root
    $sdkFiles = @(git -C $repo ls-files -- sdk/runtime)
    if ($LASTEXITCODE -ne 0 -or $sdkFiles.Count -eq 0) { throw 'could not enumerate Runtime SDK sources' }
    foreach ($file in $sdkFiles) {
        Copy-RequiredFile (Join-Path $repo $file) (Join-Path $root ("sdk/" + $file.Substring('sdk/runtime/'.Length)))
    }
    foreach ($file in @('LICENSE', 'NOTICE')) { Copy-RequiredFile (Join-Path $repo $file) (Join-Path $root $file) }
    Complete-Archive $root $name 'sdk-sources'
}

function New-BrowserPackage {
    $browser = Join-Path $repo 'sdk/runtime/js'
    $browserVersion = (Get-Content -Raw (Join-Path $browser 'package.json') | ConvertFrom-Json).version
    if ($browserVersion -ne $Version.TrimStart('v')) { throw 'browser SDK version does not match release' }
    Push-Location $browser
    try {
        if (-not $SkipBrowserBuild) {
            & npm.cmd run build:wasm | Out-Host
            if ($LASTEXITCODE -ne 0) { throw 'browser WASM build failed' }
        }
        $packed = & npm.cmd pack --json --pack-destination $OutputDir
        if ($LASTEXITCODE -ne 0) { throw 'browser SDK pack failed' }
        $packed = ($packed -join "`n") | ConvertFrom-Json
    } finally { Pop-Location }
    $name = "$PackageName-browser-v$($Version.TrimStart('v'))"
    $tgz = Join-Path $OutputDir "$name.tgz"
    Move-Item -Force -LiteralPath (Join-Path $OutputDir $packed[0].filename) -Destination $tgz
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $tgz).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$tgz.sha256" -Encoding ASCII -Value "$hash  $name.tgz"
    $root = Join-Path $stagingRoot $name
    Remove-GeneratedDirectory $root $stagingRoot
    New-Directory $root
    & tar -xzf $tgz -C $root --strip-components=1
    if ($LASTEXITCODE -ne 0) { throw 'browser SDK archive extraction failed' }
    [pscustomobject]@{ Platform = 'browser-npm'; Package = $tgz; Sha256 = "$tgz.sha256" }
    Complete-Archive $root $name 'browser-wasm'
}

function Update-SdkReleaseMetadata {
    # SDK sources are outside native ZIPs so their pinned archive hashes cannot
    # create a circular hash dependency.
    $manifestPath = Join-Path $repo 'sdk/runtime/release_manifest.json'
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    $oldVersion = $manifest.runtime_version
    $newVersion = $Version.TrimStart('v')
    $replacements = @{}
    foreach ($platform in @('windows-x86_64', 'linux-x86_64')) {
        $asset = $manifest.assets.$platform
        $archive = "$PackageName-runtime-v$newVersion-$platform.zip"
        $newHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $OutputDir $archive)).Hash.ToLowerInvariant()
        $replacements[$asset.sha256] = $newHash
        $asset.archive = $archive
        $asset.sha256 = $newHash
    }
    foreach ($file in @(git -C $repo ls-files -- sdk/runtime)) {
        if ($file -eq 'sdk/runtime/release_manifest.json') { continue }
        $path = Join-Path $repo $file
        $content = [IO.File]::ReadAllText($path)
        $updated = $content.Replace($oldVersion, $newVersion)
        foreach ($oldHash in $replacements.Keys) { $updated = $updated.Replace($oldHash, $replacements[$oldHash]) }
        if ($updated -cne $content) { [IO.File]::WriteAllText($path, $updated, [Text.UTF8Encoding]::new($false)) }
    }
    $manifest.runtime_version = $newVersion
    $manifest.release_tag = "v$newVersion"
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
}

Set-Location $repo

if (-not $Version) {
    $Version = Get-RuntimeVersion
}
if ($Version -notmatch '^v?\d+\.\d+\.\d+([.-][A-Za-z0-9.-]+)?$' -or $PackageName -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') {
    throw 'invalid version or package filename prefix'
}

if ($Clean) {
    if (Test-Path $OutputDir) {
        Remove-GeneratedDirectory $OutputDir (Join-Path $repo 'dist')
    }
    if (Test-Path $stagingRoot) {
        Remove-GeneratedDirectory $stagingRoot (Join-Path $repo 'dist')
    }
}

New-Directory $runtimeDist
New-Directory $OutputDir
New-Directory $stagingRoot

if (-not $SkipBuild -and -not $FrontendOnly) {
    & (Join-Path $PSScriptRoot 'build-runtime.ps1') -Targets $Targets -Release:$true
    if ($LASTEXITCODE -ne 0) {
        throw "runtime build failed ($LASTEXITCODE)"
    }
}

$results = @()
if (-not $FrontendOnly) {
    $results += foreach ($target in Get-SelectedTargets) { New-ReleasePackage $target }
}
$results += New-FrontendPackage
if (-not $FrontendOnly) {
    $results += New-BrowserPackage
    if ($LocalPreview) { Write-Warning 'Local preview: stable SDK online-download pins are unchanged; load the packaged library explicitly.' }
    elseif ($Targets -eq 'all') { Update-SdkReleaseMetadata }
    else { Write-Warning 'Single-platform preview: SDK download pins are unchanged; load the packaged library explicitly.' }
    $results += New-SdkPackage
}

Write-Host ""
Write-Host "Release packages prepared under $OutputDir" -ForegroundColor Green
$results | Format-Table Platform, Package, Sha256 -AutoSize
