$ErrorActionPreference = "Stop"

function Get-WorkspaceVersion {
  $cargo = Get-Content -Raw -Path "Cargo.toml"
  $m = [regex]::Match($cargo, "(?ms)^\[workspace\.package\].*?^version\s*=\s*""([^""]+)""")
  if (-not $m.Success) { throw "Failed to locate [workspace.package] version in Cargo.toml" }
  return $m.Groups[1].Value
}

function Get-GitShortHash {
  try {
    $h = (& git rev-parse --short HEAD).Trim()
    if ($h) { return $h }
  } catch {}
  return "unknown"
}

function Get-Arch {
  $arch = $env:PROCESSOR_ARCHITECTURE
  if ($arch -eq "AMD64") { return "x86_64" }
  return $arch.ToLowerInvariant()
}

# Ensure dist contains only artifacts from this build.
if (Test-Path "dist") {
  Get-ChildItem -Path "dist" -Force | Remove-Item -Recurse -Force
} else {
  New-Item -ItemType Directory -Force -Path "dist" | Out-Null
}

$version = Get-WorkspaceVersion
$gitHash = Get-GitShortHash
$buildVersion = "$version+$gitHash"
$arch = Get-Arch

Write-Host "Building llmhotspot + llmhotspot-shell (release) for windows/$arch..."
cargo build -p llmhotspot -p llmhotspot-shell --release --locked | Out-Host

$outDir = Join-Path "dist" ("llmhotspot-v{0}-windows-{1}" -f $buildVersion, $arch)
if (Test-Path $outDir) { Remove-Item -Recurse -Force $outDir }
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

Copy-Item -Force "target\\release\\llmhotspot.exe" (Join-Path $outDir "llmhotspot.exe")
Copy-Item -Force "target\\release\\llmhotspot-shell.exe" (Join-Path $outDir "llmhotspot-shell.exe")

# Bundle cloudflared (Windows). Asset names are stable today, but verify if Cloudflare changes releases.
$cloudflaredUrl = "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe"
$cloudflaredPath = Join-Path $outDir "cloudflared.exe"
Write-Host "Downloading cloudflared -> $cloudflaredPath"
Invoke-WebRequest -Uri $cloudflaredUrl -OutFile $cloudflaredPath

@"
LLM Hotspot (v2) - Windows

Run:
  llmhotspot-shell.exe

Then open:
  Tray icon -> Open Console

Notes:
- cloudflared is bundled as cloudflared.exe and is managed by the app when remote mode is enabled.
- llmhotspot-shell.exe is the Windows tray host that supervises llmhotspot.exe.
- NSIS installer is built by this script when makensis is available.
"@ | Set-Content -Path (Join-Path $outDir "README.txt") -Encoding ASCII

$zipPath = Join-Path "dist" ("llmhotspot-v{0}-windows-{1}.zip" -f $buildVersion, $arch)
if (Test-Path $zipPath) { Remove-Item -Force $zipPath }

Write-Host "Packaging $zipPath"
Compress-Archive -Path (Join-Path $outDir "*") -DestinationPath $zipPath

$nsisScript = "installer\windows\llmhotspot.nsi"
$installerPath = Join-Path "dist" ("LLMHotspot-Setup-v{0}-windows-{1}.exe" -f $buildVersion, $arch)
$makensis = Get-Command "makensis" -ErrorAction SilentlyContinue

# Convert branding image to NSIS-friendly BMPs while preserving aspect ratio.
$brandingPng = "light-logo.png"
function New-BrandedBitmap {
  param(
    [Parameter(Mandatory=$true)][string]$SourcePath,
    [Parameter(Mandatory=$true)][string]$DestPath,
    [Parameter(Mandatory=$true)][int]$CanvasWidth,
    [Parameter(Mandatory=$true)][int]$CanvasHeight,
    [int]$Padding = 8
  )
  $destDir = Split-Path -Parent $DestPath
  New-Item -ItemType Directory -Force -Path $destDir | Out-Null
  $src = New-Object System.Drawing.Bitmap((Resolve-Path $SourcePath).Path)
  try {
    $canvas = New-Object System.Drawing.Bitmap($CanvasWidth, $CanvasHeight)
    try {
      $g = [System.Drawing.Graphics]::FromImage($canvas)
      try {
        $g.Clear([System.Drawing.Color]::White)
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
        $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $maxW = [Math]::Max(1, $CanvasWidth - ($Padding * 2))
        $maxH = [Math]::Max(1, $CanvasHeight - ($Padding * 2))
        $scale = [Math]::Min($maxW / [double]$src.Width, $maxH / [double]$src.Height)
        $drawW = [int][Math]::Max(1, [Math]::Round($src.Width * $scale))
        $drawH = [int][Math]::Max(1, [Math]::Round($src.Height * $scale))
        $x = [int][Math]::Round(($CanvasWidth - $drawW) / 2.0)
        $y = [int][Math]::Round(($CanvasHeight - $drawH) / 2.0)
        $g.DrawImage($src, $x, $y, $drawW, $drawH)
      } finally {
        $g.Dispose()
      }
      $canvas.Save($DestPath, [System.Drawing.Imaging.ImageFormat]::Bmp)
    } finally {
      $canvas.Dispose()
    }
  } finally {
    $src.Dispose()
  }
}

$brandingHeaderBmp = Join-Path "dist" "nsis-branding\light-logo-header.bmp"
$brandingWelcomeBmp = Join-Path "dist" "nsis-branding\light-logo-welcome.bmp"
$brandingHeaderDefine = $null
$brandingWelcomeDefine = $null
if (Test-Path $brandingPng) {
  try {
    Add-Type -AssemblyName System.Drawing
    # NSIS MUI expected dimensions:
    # - Header image: 150x57
    # - Welcome/Finish image: 164x314
    New-BrandedBitmap -SourcePath $brandingPng -DestPath $brandingHeaderBmp -CanvasWidth 150 -CanvasHeight 57 -Padding 4
    New-BrandedBitmap -SourcePath $brandingPng -DestPath $brandingWelcomeBmp -CanvasWidth 164 -CanvasHeight 314 -Padding 12
    $brandingHeaderDefine = "/DBRANDING_HEADER_BITMAP=$((Resolve-Path $brandingHeaderBmp).Path)"
    $brandingWelcomeDefine = "/DBRANDING_WELCOME_BITMAP=$((Resolve-Path $brandingWelcomeBmp).Path)"
    Write-Host "Generated NSIS branding bitmaps -> $brandingHeaderBmp, $brandingWelcomeBmp"
  } catch {
    Write-Warning "Failed to generate NSIS branding bitmap from $brandingPng; continuing without custom branding."
  }
}

# NSIS often installs to Program Files without adding makensis to PATH.
if (-not $makensis) {
  $candidates = @(
    "C:\Program Files (x86)\NSIS\makensis.exe",
    "C:\Program Files\NSIS\makensis.exe"
  )
  foreach ($p in $candidates) {
    if (Test-Path $p) { $makensis = @{ Path = $p }; break }
  }
}

if ($makensis -and (Test-Path $nsisScript)) {
  Write-Host "Building NSIS installer $installerPath"
  $appDirAbs = (Resolve-Path $outDir).Path
  $outFileAbs = (Resolve-Path "dist").Path + "\" + [System.IO.Path]::GetFileName($installerPath)
  $nsisArgs = @(
    "/DAPP_DIR=$appDirAbs"
    "/DVERSION=$version"
    "/DOUT_FILE=$outFileAbs"
  )
  if ($brandingHeaderDefine) { $nsisArgs += $brandingHeaderDefine }
  if ($brandingWelcomeDefine) { $nsisArgs += $brandingWelcomeDefine }
  $nsisArgs += $nsisScript
  & $makensis.Path @nsisArgs
  if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE" }
} else {
  Write-Warning "makensis not found; skipping installer build. Install NSIS to produce Setup.exe."
}

Write-Host "Done:"
Write-Host ("  {0}" -f $zipPath)
if (Test-Path $installerPath) {
  Write-Host ("  {0}" -f $installerPath)
}
