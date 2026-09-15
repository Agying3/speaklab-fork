# Build the server and assemble a copyable bundle.
#
# Why this script exists: the whisper-rs-sys build script emits
# `cargo:rustc-link-lib=dylib=stdc++`, which makes libstdc++ a DYNAMIC
# dependency. That request comes from the dependency graph, so rustflags
# cannot override it (rustflags apply earlier and get superseded).
# The result is an exe that needs a few MinGW DLLs next to it, otherwise
# it dies at startup with 0xC0000135 STATUS_DLL_NOT_FOUND.
#
# So compiling and copying the DLLs are one step here.
#
# NOTE: keep this file ASCII-only. Windows PowerShell 5.1 reads BOM-less
# files as ANSI, and non-ASCII bytes break parsing.

param(
    [switch]$Whisper,      # include local speech recognition
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# --- toolchain ----------------------------------------------------
# rustup here uses the GNU toolchain. Building whisper's C++ needs:
#   cmake    - generates whisper.cpp's build files
#   ninja    - runs the build
#   g++      - compiles the C++
#   libclang - bindgen generates the FFI bindings
$mingw = 'C:\msys64\mingw64\bin'
$cmake = 'C:\Program Files\CMake\bin'

foreach ($p in @($mingw, $cmake)) {
    if (Test-Path $p) { $env:PATH = "$p;$env:PATH" }
}

if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    Write-Warning 'cmake not found; -Whisper builds will fail. Install: winget install Kitware.CMake'
}
if (-not (Test-Path "$mingw\libclang.dll")) {
    Write-Warning 'libclang.dll not found; -Whisper builds will fail. Install: pacman -S mingw-w64-x86_64-clang'
} else {
    $env:LIBCLANG_PATH = $mingw
}
if (Get-Command ninja -ErrorAction SilentlyContinue) {
    $env:CMAKE_GENERATOR = 'Ninja'
}

# --- compile ------------------------------------------------------
if (-not $SkipBuild) {
    $buildArgs = @('build', '--release')
    if ($Whisper) { $buildArgs += @('--features', 'whisper') }
    Write-Host "cargo $($buildArgs -join ' ')" -ForegroundColor Cyan
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "build failed (exit $LASTEXITCODE)" }
}

# --- runtime DLLs -------------------------------------------------
# These are not in the system directory. Copying them next to the exe is
# what makes the bundle runnable on a machine without MinGW installed.
$outDir = Join-Path $root 'target\release'
$dlls = @(
    'libstdc++-6.dll',      # whisper's C++ runtime
    'libgcc_s_seh-1.dll',   # dependency of the above
    'libwinpthread-1.dll'   # dependency of the above
)

Write-Host ''
Write-Host "Copying runtime DLLs to $outDir" -ForegroundColor Cyan
$missing = @()
foreach ($d in $dlls) {
    $src = Join-Path $mingw $d
    if (Test-Path $src) {
        Copy-Item $src (Join-Path $outDir $d) -Force
        Write-Host "  $d"
    } else {
        $missing += $d
    }
}

if ($missing.Count -gt 0) {
    Write-Warning "Not found: $($missing -join ', '). The bundle may not start elsewhere."
}

# --- verify -------------------------------------------------------
$exe = Join-Path $outDir 'speaklab-server.exe'
if (Test-Path $exe) {
    $size = [math]::Round((Get-Item $exe).Length / 1MB, 2)
    Write-Host ''
    Write-Host "Done: $exe  ($size MB)" -ForegroundColor Green
    Write-Host 'Copy the whole target\release directory to deploy.' -ForegroundColor Green
}
