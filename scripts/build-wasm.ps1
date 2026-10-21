# Compile the Rust codebase into WebAssembly files using wasm-pack
[CmdletBinding()]
param(
    [switch]$Dev,
    [string]$OutDir = "pkg"
)

$ErrorActionPreference = "Stop"

$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Users\EmranVaiya\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained;C:\Users\EmranVaiya\tools\bin;C:\Users\EmranVaiya\tools\git\cmd;$env:PATH"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "rust-lld"

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Compiling photocraft-web into WebAssembly with wasm-pack..." -ForegroundColor Cyan

$buildArgs = @("build", "apps/photocraft-web", "--target", "web", "--out-dir", $OutDir)
if ($Dev) {
    $buildArgs += "--dev"
} else {
    $buildArgs += "--release"
}

wasm-pack @buildArgs

if (Test-Path "apps\photocraft-web\index-wasm.html") {
    Copy-Item "apps\photocraft-web\index-wasm.html" -Destination "apps\photocraft-web\$OutDir\index.html" -Force
}

Write-Host "Build complete! WebAssembly files generated in apps/photocraft-web/$OutDir" -ForegroundColor Green
