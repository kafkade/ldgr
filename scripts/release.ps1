#!/usr/bin/env pwsh
#Requires -Version 7.0
<#
.SYNOPSIS
    Prepare and cut an ldgr release.
.DESCRIPTION
    Automates the release process:
    1. Reads current version from Cargo.toml workspace
    2. Bumps the specified semver component (major, minor, or patch)
    3. Validates there are unreleased changelog entries
    4. Stamps the [Unreleased] section in CHANGELOG.md with version and date
    5. Runs cargo check to update Cargo.lock
    6. Commits, tags, and (optionally) pushes
.PARAMETER Bump
    Which semver component to bump: major, minor, or patch.
.PARAMETER Push
    Push the commit and tag to origin after creating them.
.PARAMETER DryRun
    Show what would happen without making changes.
.EXAMPLE
    ./scripts/release.ps1 patch
    ./scripts/release.ps1 minor -Push
    ./scripts/release.ps1 major -DryRun
#>
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateSet("major", "minor", "patch")]
    [string]$Bump,

    [switch]$Push,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = git rev-parse --show-toplevel 2>$null
if (-not $RepoRoot) { Write-Error "Not in a git repository"; exit 1 }
Set-Location $RepoRoot

# --- Read and bump version ---

# ldgr uses [workspace.package] version in the root Cargo.toml
$cargoToml = Get-Content Cargo.toml -Raw
$currentMatch = [regex]::Match($cargoToml, '(?m)^version = "(\d+)\.(\d+)\.(\d+)"')
if (-not $currentMatch.Success) {
    Write-Error "Could not parse version from Cargo.toml [workspace.package]"
    exit 1
}

$major = [int]$currentMatch.Groups[1].Value
$minor = [int]$currentMatch.Groups[2].Value
$patch = [int]$currentMatch.Groups[3].Value
$currentVersion = "$major.$minor.$patch"

switch ($Bump) {
    "major" { $major++; $minor = 0; $patch = 0 }
    "minor" { $minor++; $patch = 0 }
    "patch" { $patch++ }
}

$Version = "$major.$minor.$patch"
$Tag = "v$Version"
$Today = Get-Date -Format "yyyy-MM-dd"

Write-Host "`n📦 Release: $currentVersion → $Version ($Bump bump)" -ForegroundColor Cyan

# --- Preflight checks ---

Write-Host "`n🔍 Preflight checks" -ForegroundColor Cyan

# Clean working tree
$status = git status --porcelain
if ($status) {
    Write-Error "Working tree is not clean. Commit or stash changes first."
    exit 1
}
Write-Host "  ✓ Working tree clean" -ForegroundColor Green

# On main branch
$branch = git branch --show-current
if ($branch -ne "main") {
    Write-Error "Must be on 'main' branch (currently on '$branch')."
    exit 1
}
Write-Host "  ✓ On main branch" -ForegroundColor Green

# Tag doesn't already exist
$existing = git tag -l $Tag
if ($existing) {
    Write-Error "Tag '$Tag' already exists."
    exit 1
}
Write-Host "  ✓ Tag $Tag is available" -ForegroundColor Green

# Changelog has unreleased entries
$changelog = Get-Content CHANGELOG.md -Raw
if ($changelog -notmatch '## \[Unreleased\]\s*\n+### ') {
    Write-Error "No entries found under [Unreleased] in CHANGELOG.md."
    exit 1
}
Write-Host "  ✓ Changelog has unreleased entries" -ForegroundColor Green

# Formatting
Write-Host "`n🎨 Checking formatting..." -ForegroundColor Cyan
$fmtOutput = cargo fmt -- --check 2>&1
if ($LASTEXITCODE -ne 0) {
    $fmtOutput | Write-Host
    Write-Error "Formatting issues found. Run 'cargo fmt' first."
    exit 1
}
Write-Host "  ✓ Formatting clean" -ForegroundColor Green

# Tests pass
Write-Host "`n🧪 Running tests..." -ForegroundColor Cyan
$testOutput = cargo test --workspace 2>&1
$testExitCode = $LASTEXITCODE
if ($testExitCode -ne 0) {
    $testOutput | Write-Host
    Write-Error "Tests failed. Fix before releasing."
    exit 1
}
Write-Host "  ✓ All tests pass" -ForegroundColor Green

# Clippy clean
Write-Host "`n🔎 Running clippy..." -ForegroundColor Cyan
$clippyOutput = cargo clippy --workspace --all-features --all-targets -- -D warnings 2>&1
$clippyExitCode = $LASTEXITCODE
if ($clippyExitCode -ne 0) {
    $clippyOutput | Write-Host
    Write-Error "Clippy warnings found. Fix before releasing."
    exit 1
}
Write-Host "  ✓ Clippy clean" -ForegroundColor Green

if ($DryRun) {
    Write-Host "`n📋 Dry run — would perform:" -ForegroundColor Yellow
    Write-Host "  1. Bump Cargo.toml workspace version to $Version"
    Write-Host "  2. Stamp CHANGELOG.md [Unreleased] → [$Version] - $Today"
    Write-Host "  3. Commit: 'chore: release v$Version'"
    Write-Host "  4. Tag: $Tag"
    if ($Push) { Write-Host "  5. Push to origin with tag" }
    Write-Host "  6. Release workflow builds multi-platform binaries"
    exit 0
}

# --- Apply changes ---

Write-Host "`n📦 Preparing release $Tag" -ForegroundColor Cyan

# 1. Bump Cargo.toml workspace version
$cargoToml = Get-Content Cargo.toml -Raw
$cargoToml = $cargoToml -replace '(?m)^version = "[^"]*"', "version = `"$Version`""
Set-Content Cargo.toml -Value $cargoToml -NoNewline
Write-Host "  ✓ Cargo.toml → $Version" -ForegroundColor Green

# 2. Update Cargo.lock
cargo check --quiet 2>$null
Write-Host "  ✓ Cargo.lock updated" -ForegroundColor Green

# 3. Stamp CHANGELOG.md
$changelog = Get-Content CHANGELOG.md -Raw
$changelog = $changelog -replace '## \[Unreleased\]', "## [Unreleased]`n`n## [$Version] - $Today"
Set-Content CHANGELOG.md -Value $changelog -NoNewline
Write-Host "  ✓ CHANGELOG.md stamped" -ForegroundColor Green

# 4. Commit and tag
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore: release v$Version"
git tag -a $Tag -m "Release $Version"
Write-Host "  ✓ Committed and tagged $Tag" -ForegroundColor Green

# 5. Push (optional)
if ($Push) {
    Write-Host "`n🚀 Pushing to origin..." -ForegroundColor Cyan
    git push origin main --follow-tags
    Write-Host "  ✓ Pushed — release workflow will build binaries" -ForegroundColor Green
} else {
    Write-Host "`n📌 Ready to push. Run:" -ForegroundColor Yellow
    Write-Host "  git push origin main --follow-tags" -ForegroundColor White
}

Write-Host "`n✅ Release $Tag prepared successfully!`n" -ForegroundColor Green
