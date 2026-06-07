<#
.SYNOPSIS
  Build a self-contained portable folder in dist-portable/ containing the release
  exe, the sherpa-onnx runtime DLLs, the models and the config. Run the result
  with dist-portable/siri-desktop.exe (models resolve from ./models).

  Requires models to be present first: scripts/fetch-models.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$models = Join-Path $root 'src-tauri\models'
if (-not (Test-Path (Join-Path $models 'asr\model.onnx'))) {
    throw "Models not found. Run scripts/fetch-models.ps1 first."
}

Write-Host "Building release app (Tauri production mode embeds the frontend)..." -ForegroundColor Cyan
# Must go through the Tauri CLI so the webview loads the embedded assets rather
# than the dev URL. --no-bundle skips the NSIS/MSI installer (we ship portable).
npm run tauri build -- --no-bundle
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$rel = Join-Path $root 'src-tauri\target\release'
$out = Join-Path $root 'dist-portable'
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force -Path $out | Out-Null

Write-Host "Assembling portable folder..." -ForegroundColor Cyan
Copy-Item (Join-Path $rel 'siri-desktop.exe') $out -Force
# runtime DLLs (sherpa-onnx-c-api.dll, onnxruntime*.dll) the build script placed here
Copy-Item (Join-Path $rel '*.dll') $out -Force
Copy-Item $models (Join-Path $out 'models') -Recurse -Force
Copy-Item (Join-Path $root 'config.toml') $out -Force
if (Test-Path (Join-Path $root 'config.local.toml')) {
    Copy-Item (Join-Path $root 'config.local.toml') $out -Force
}

Write-Host ""
Write-Host "Done -> $out" -ForegroundColor Green
Write-Host "Run: $out\siri-desktop.exe"
