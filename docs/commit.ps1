# SpeakLab: commit script (3 semantic commits)
#
# Usage (from the H:\toos\speaklab directory):
#   .\docs\commit.ps1
#
# NOTE: This file is intentionally ASCII-only. Windows PowerShell 5.1 reads
# BOM-less files as ANSI, which corrupts non-ASCII text and breaks parsing.
# Keeping it ASCII avoids that class of failure entirely.
#
# Prerequisites -- configure your git identity first:
#   git config user.name  "tujue"
#   git config user.email "11224600cc@gmail.com"
#
# This repository trips git's "dubious ownership" check. The fix used here is
# a repo-local safe.directory entry, NOT a GIT_CONFIG_GLOBAL override: that
# override replaces the global config entirely and hides your real identity.

$ErrorActionPreference = 'Continue'

# ---------- safety check: only run against the intended repository ----------
$repoRoot = git rev-parse --show-toplevel 2>$null
if ($LASTEXITCODE -ne 0 -or -not $repoRoot) {
    Write-Host 'Not inside a git repository.' -ForegroundColor Red
    exit 1
}
$repoRoot = $repoRoot -replace '\\', '/'
Write-Host "repository: $repoRoot" -ForegroundColor DarkGray

# Clear the dubious-ownership condition repo-locally (no global override).
git config --local --get safe.directory 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) {
    git config --local safe.directory $repoRoot 2>&1 | Out-Null
}

Write-Host '=== Checking identity ===' -ForegroundColor Cyan
$name = git config user.name
$email = git config user.email
if (-not $name -or -not $email) {
    Write-Host 'git identity is not configured. Run:' -ForegroundColor Red
    Write-Host '  git config user.name  "tujue"'
    Write-Host '  git config user.email "11224600cc@gmail.com"'
    exit 1
}
Write-Host "identity: $name <$email>" -ForegroundColor Green

$msg1 = Join-Path $PSScriptRoot '.commit-msg-1.txt'
$msg2 = Join-Path $PSScriptRoot '.commit-msg-2.txt'
$msg3 = Join-Path $PSScriptRoot '.commit-msg-3.txt'
foreach ($m in @($msg1, $msg2, $msg3)) {
    if (-not (Test-Path $m)) {
        Write-Host "missing commit message file: $m" -ForegroundColor Red
        exit 1
    }
}

# ---------- commit 1: remove Whisper engine + fix mic leaks ----------
Write-Host "`n=== commit 1/3: remove Whisper engine + fix mic leaks ===" -ForegroundColor Cyan
$global:LASTEXITCODE = 0
git add -A server index.html .gitignore 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host 'git add failed' -ForegroundColor Red; exit 1 }
git commit -F $msg1
if ($LASTEXITCODE -ne 0) { Write-Host 'git commit failed' -ForegroundColor Red; exit 1 }

# ---------- commit 2: honest README ----------
Write-Host "`n=== commit 2/3: honest README ===" -ForegroundColor Cyan
$global:LASTEXITCODE = 0
git add README.md README.zh-CN.md 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host 'git add failed' -ForegroundColor Red; exit 1 }
git commit -F $msg2
if ($LASTEXITCODE -ne 0) { Write-Host 'git commit failed' -ForegroundColor Red; exit 1 }

# ---------- commit 3: technical docs ----------
Write-Host "`n=== commit 3/3: technical docs ===" -ForegroundColor Cyan
$global:LASTEXITCODE = 0
git add docs 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host 'git add failed' -ForegroundColor Red; exit 1 }
# The message files are build inputs, not deliverables -- unstage them.
$staged = @(git diff --cached --name-only)
foreach ($f in @('docs/.commit-msg-1.txt','docs/.commit-msg-2.txt','docs/.commit-msg-3.txt')) {
    if ($staged -contains $f) { git rm --cached --quiet $f 2>&1 | Out-Null }
}
git commit -F $msg3
if ($LASTEXITCODE -ne 0) { Write-Host 'git commit failed' -ForegroundColor Red; exit 1 }

Write-Host "`n=== Done ===" -ForegroundColor Green
git log --oneline -4
Write-Host "`nPush with:" -ForegroundColor Yellow
Write-Host "  git push -u origin fix/remove-nonfunctional-whisper-and-honest-readme"
