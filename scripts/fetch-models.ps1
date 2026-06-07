<#
.SYNOPSIS
  Download the offline sherpa-onnx models (wake word, ASR, VAD) and arrange them
  into the canonical layout the app expects:

    src-tauri/models/
      kws/  encoder.onnx decoder.onnx joiner.onnx tokens.txt keywords.txt
      asr/  model.onnx tokens.txt
      vad/  silero_vad.onnx

  Re-running is cheap: existing target files are skipped.
#>
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # much faster Invoke-WebRequest

$root   = Split-Path -Parent $PSScriptRoot
$models = Join-Path $root 'src-tauri\models'
$tmp    = Join-Path $env:TEMP 'siri-models'
New-Item -ItemType Directory -Force -Path $models, $tmp | Out-Null

function Get-Archive($url, $outFile) {
    if (Test-Path $outFile) { Write-Host "  cached: $(Split-Path $outFile -Leaf)"; return }
    Write-Host "  downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $outFile
}

function Expand-Tar($archive, $dest) {
    Write-Host "  extracting $(Split-Path $archive -Leaf)"
    # Windows 10/11 ship bsdtar (tar.exe), which auto-detects bzip2.
    tar -xf $archive -C $dest
    if ($LASTEXITCODE -ne 0) { throw "tar failed for $archive" }
}

function Copy-First($srcDir, $pattern, $dest, [switch]$ExcludeInt8) {
    $items = Get-ChildItem -Path $srcDir -Filter $pattern
    if ($ExcludeInt8) { $items = $items | Where-Object { $_.Name -notmatch 'int8' } }
    $file = $items | Select-Object -First 1
    if (-not $file) { throw "no file matching '$pattern' in $srcDir" }
    Copy-Item $file.FullName $dest -Force
    Write-Host "  -> $(Split-Path $dest -Leaf)  (from $($file.Name))"
}

# ---------------------------------------------------------------- wake word ---
Write-Host "[1/4] Wake word (KWS, Chinese zipformer)"
$kwsDst = Join-Path $models 'kws'
New-Item -ItemType Directory -Force -Path $kwsDst | Out-Null
if (Test-Path (Join-Path $kwsDst 'encoder.onnx')) {
    Write-Host "  already installed, skipping"
} else {
    $url = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01.tar.bz2'
    $tar = Join-Path $tmp 'kws.tar.bz2'
    Get-Archive $url $tar
    Expand-Tar $tar $tmp
    $src = Join-Path $tmp 'sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01'
    Copy-First $src '*encoder*.onnx' (Join-Path $kwsDst 'encoder.onnx') -ExcludeInt8
    Copy-First $src '*decoder*.onnx' (Join-Path $kwsDst 'decoder.onnx') -ExcludeInt8
    Copy-First $src '*joiner*.onnx'  (Join-Path $kwsDst 'joiner.onnx')  -ExcludeInt8
    Copy-Item (Join-Path $src 'tokens.txt')   (Join-Path $kwsDst 'tokens.txt')   -Force
    Copy-Item (Join-Path $src 'keywords.txt') (Join-Path $kwsDst 'keywords.txt') -Force
}

# ---------------------------------------------------------------------- ASR ---
Write-Host "[2/4] ASR (SenseVoice, multilingual incl. Chinese)"
$asrDst = Join-Path $models 'asr'
New-Item -ItemType Directory -Force -Path $asrDst | Out-Null
if (Test-Path (Join-Path $asrDst 'model.onnx')) {
    Write-Host "  already installed, skipping"
} else {
    $url = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2'
    $tar = Join-Path $tmp 'asr.tar.bz2'
    Get-Archive $url $tar
    Expand-Tar $tar $tmp
    $src = Join-Path $tmp 'sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17'
    # int8 model: ~5x smaller, accuracy is still excellent for commands
    Copy-Item (Join-Path $src 'model.int8.onnx') (Join-Path $asrDst 'model.onnx') -Force
    Copy-Item (Join-Path $src 'tokens.txt')      (Join-Path $asrDst 'tokens.txt') -Force
    Write-Host "  -> model.onnx  (from model.int8.onnx)"
}

# ---------------------------------------------------------------------- VAD ---
Write-Host "[3/4] VAD (silero)"
$vadDst = Join-Path $models 'vad'
New-Item -ItemType Directory -Force -Path $vadDst | Out-Null
$vadFile = Join-Path $vadDst 'silero_vad.onnx'
if (Test-Path $vadFile) {
    Write-Host "  already installed, skipping"
} else {
    $url = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx'
    Get-Archive $url $vadFile
}

# ---------------------------------------------------------------------- TTS ---
Write-Host "[4/4] TTS (MeloTTS zh-en, for voice replies)"
$ttsDst = Join-Path $models 'tts'
New-Item -ItemType Directory -Force -Path $ttsDst | Out-Null
if (Test-Path (Join-Path $ttsDst 'model.onnx')) {
    Write-Host "  already installed, skipping"
} else {
    $url = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-melo-tts-zh_en.tar.bz2'
    $tar = Join-Path $tmp 'tts.tar.bz2'
    Get-Archive $url $tar
    Expand-Tar $tar $tmp
    $src = Join-Path $tmp 'vits-melo-tts-zh_en'
    Copy-Item (Join-Path $src 'model.onnx')  (Join-Path $ttsDst 'model.onnx')  -Force
    Copy-Item (Join-Path $src 'lexicon.txt') (Join-Path $ttsDst 'lexicon.txt') -Force
    Copy-Item (Join-Path $src 'tokens.txt')  (Join-Path $ttsDst 'tokens.txt')  -Force
    if (Test-Path (Join-Path $src 'dict')) {
        Copy-Item (Join-Path $src 'dict') (Join-Path $ttsDst 'dict') -Recurse -Force
    }
    Get-ChildItem -Path $src -Filter '*.fst' -ErrorAction SilentlyContinue | ForEach-Object {
        Copy-Item $_.FullName (Join-Path $ttsDst $_.Name) -Force
    }
    Write-Host "  -> model.onnx, lexicon.txt, tokens.txt, dict/, *.fst"
}

Write-Host ""
Write-Host "Done. Models installed to $models"
Write-Host ""
Write-Host "Active wake words (models/kws/keywords.txt):" -ForegroundColor Cyan
Get-Content (Join-Path $kwsDst 'keywords.txt') | ForEach-Object {
    if ($_ -match '@(.+)$') { Write-Host ("  - " + $Matches[1]) }
}
Write-Host ""
Write-Host "To add your own wake word, install the python CLI and run:" -ForegroundColor Cyan
Write-Host "  pip install sherpa-onnx"
Write-Host "  echo 你好小智 > raw.txt"
Write-Host "  sherpa-onnx-cli text2token --tokens src-tauri/models/kws/tokens.txt --tokens-type ppinyin raw.txt line.txt"
Write-Host "  # then append the contents of line.txt to src-tauri/models/kws/keywords.txt"
