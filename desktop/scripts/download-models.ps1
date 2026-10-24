<#
.SYNOPSIS
    Fetches the sherpa-onnx runtime and STT model weights Perihel needs.

.DESCRIPTION
    Perihel Transcribe shells out to the prebuilt `sherpa-onnx-offline.exe`
    sidecar rather than linking sherpa-onnx into Rust. Nothing model-sized
    stays resident between dictations — the sidecar process loads weights,
    transcribes one wav, and exits.

    Two things get downloaded:

      1. models/bin/   — the sherpa-onnx v1.13.8 Windows release (runtime + offline exe)
      2. models/stt/   — the model weights for the engine you pick

    Weights are large (SenseVoice int8 ≈ 226 MB). They are deliberately
    gitignored; run this once after cloning.

.EXAMPLE
    .\scripts\download-models.ps1                      # SenseVoice (recommended, multilingual)
    .\scripts\download-models.ps1 -Engine whisper-tiny # lighter, English-only
    .\scripts\download-models.ps1 -Engine all          # both
#>
[CmdletBinding()]
param(
    [ValidateSet('sensevoice', 'whisper-tiny', 'parakeet', 'all')]
    [string]$Engine = 'sensevoice',

    # Override when models live outside the repo (see perihel_core::paths::models_dir).
    [string]$ModelsDir
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $ModelsDir) { $ModelsDir = Join-Path $RepoRoot 'models' }

$SherpaVersion = 'v1.13.8'
$SherpaFlavour = 'sherpa-onnx-v1.13.8-win-x64-shared-MD-MinSizeRel'

# name -> @{ url; files = @(...); dest = folder under models/stt }
$Catalog = @{
    'sensevoice'  = @{
        url   = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2'
        files = @('model.int8.onnx', 'tokens.txt')
        dest  = 'sensevoice-int8'
    }
    'whisper-tiny' = @{
        url   = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-whisper-tiny.en-2023-06-26.tar.bz2'
        files = @('tiny.en-encoder.int8.onnx', 'tiny.en-decoder.int8.onnx', 'tiny.en-tokens.txt')
        dest  = 'whisper-tiny-en'
    }
    'parakeet'    = @{
        # NVIDIA NeMo Parakeet TDT, int8. Large (~600 MB) and English-only.
        url   = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2.tar.bz2'
        files = @('encoder.int8.onnx', 'decoder.int8.onnx', 'joiner.int8.onnx', 'tokens.txt')
        dest  = 'parakeet-int8'
    }
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("perihel-models-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null

function Get-Archive {
    param([string]$Url, [string]$OutFile)
    Write-Host "  downloading $(Split-Path $Url -Leaf)" -ForegroundColor Cyan
    # -UseBasicParsing keeps this working on Windows PowerShell 5.1.
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
}

function Expand-ArchiveFile {
    param([string]$Archive, [string]$Into)
    New-Item -ItemType Directory -Path $Into -Force | Out-Null
    # tar ships with Windows 10 1803+ and handles .tar.bz2 on current builds.
    & tar -xf $Archive -C $Into
    if ($LASTEXITCODE -ne 0) { throw "tar failed to extract $Archive" }
}

try {
    # ---------------------------------------------------------------- runtime
    $binDir = Join-Path $ModelsDir 'bin'
    $offlineExe = Join-Path $binDir "$SherpaFlavour/bin/sherpa-onnx-offline.exe"
    if (Test-Path $offlineExe) {
        Write-Host "sherpa-onnx runtime already present - skipping" -ForegroundColor DarkGray
    } else {
        Write-Host "Fetching sherpa-onnx $SherpaVersion runtime..." -ForegroundColor Green
        $archive = Join-Path $tmp "$SherpaFlavour.tar.bz2"
        Get-Archive -Url "https://github.com/k2-fsa/sherpa-onnx/releases/download/$SherpaVersion/$SherpaFlavour.tar.bz2" -OutFile $archive
        Expand-ArchiveFile -Archive $archive -Into $binDir
        if (-not (Test-Path $offlineExe)) { throw "extracted archive but $offlineExe is missing" }
        Write-Host "  -> $offlineExe" -ForegroundColor DarkGray
    }

    # ----------------------------------------------------------------- models
    $engines = if ($Engine -eq 'all') { @('sensevoice', 'whisper-tiny') } else { @($Engine) }
    foreach ($name in $engines) {
        $spec = $Catalog[$name]
        $dest = Join-Path (Join-Path $ModelsDir 'stt') $spec.dest
        $missing = @($spec.files | Where-Object { -not (Test-Path (Join-Path $dest $_)) })
        if ($missing.Count -eq 0) {
            Write-Host "$name already present - skipping" -ForegroundColor DarkGray
            continue
        }

        Write-Host "Fetching $name..." -ForegroundColor Green
        $stage = Join-Path $tmp $name
        New-Item -ItemType Directory -Path $stage -Force | Out-Null
        $archive = Join-Path $tmp "$name.tar.bz2"
        Get-Archive -Url $spec.url -OutFile $archive
        Expand-ArchiveFile -Archive $archive -Into $stage

        # Releases wrap everything in a single versioned folder; flatten it.
        $inner = Get-ChildItem $stage -Directory | Select-Object -First 1
        $source = if ($inner) { $inner.FullName } else { $stage }

        New-Item -ItemType Directory -Path $dest -Force | Out-Null
        foreach ($file in $spec.files) {
            $src = Join-Path $source $file
            if (-not (Test-Path $src)) { throw "$name archive is missing '$file'" }
            Copy-Item $src (Join-Path $dest $file) -Force
        }
        Write-Host "  -> $dest" -ForegroundColor DarkGray
    }

    Write-Host ""
    Write-Host "Done. Models are under $ModelsDir" -ForegroundColor Green
    Write-Host "Set the engine in settings.json (stt_engine): sensevoice | whisper-tiny | parakeet" -ForegroundColor DarkGray
}
finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}