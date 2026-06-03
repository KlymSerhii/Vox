#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Build and publish the WINDOWS half of a Vox release, merging into the
  existing macOS release WITHOUT clobbering it. The Windows analogue of
  scripts/release.sh — but additive, not authoritative.

.DESCRIPTION
  scripts/release.sh runs on the signing Mac and cuts the canonical
  release: it (re)creates the GitHub release on KlymSerhii/vox-releases,
  uploads Vox.dmg / Vox.app.tar.gz / Vox.app.tar.gz.sig, and writes a
  latest.json whose `platforms` set contains only `darwin-aarch64`.

  This script runs on a Windows box and ADDS the Windows pieces onto that
  same release:
    * Vox_x64-setup.exe          the Windows installer - serves as BOTH the
                                 user download AND the auto-updater payload
                                 (Tauri installs it in passive mode)
    * Vox_x64-setup.exe.sig      its minisign signature
    * latest.json                MERGED: gains `windows-x86_64`, keeps
                                 `darwin-aarch64` byte-for-byte.

  It NEVER deletes the release and NEVER touches the macOS assets, so the
  macOS download link (/releases/latest/download/Vox.dmg) and the macOS
  auto-update entry both survive untouched. The Windows download lives at
  a DISTINCT path (/releases/latest/download/Vox_x64-setup.exe), so the two
  platforms' download links can't collide.

  This script is the authoritative Windows publisher. (release.sh carries an
  older optional VOX_WIN_NSIS_* hand-off that assumed a separate .nsis.zip
  updater payload; this Tauri version ships the -setup.exe as the updater
  payload instead, so prefer this script on Windows.)

.PARAMETER Publish
  Upload the staged Windows artifacts onto the existing release and
  re-upload the merged latest.json. Without this flag the script is a full
  dry run: it builds, signs, stages, prints the merged manifest, and stops.

.PARAMETER AllowCreate
  Permit cutting a Windows-ONLY release when no release exists yet for this
  version (the resulting latest.json has no `darwin-aarch64` entry). Off by
  default because the intended flow is macOS-first; see ORDERING below.

.PARAMETER Target
  Rust target triple. Defaults to x86_64-pc-windows-msvc (x64; runs on
  Windows-on-ARM under emulation too). Pass aarch64-pc-windows-msvc for a
  future native ARM64 build — the NSIS arch token and updater platform key
  derive from this automatically.

.NOTES
  ORDERING — publish the macOS release FIRST, this one SECOND:

      scripts/release.sh --publish          # on the signing Mac
      pwsh scripts/release-windows.ps1 -Publish   # then here, on Windows

  Why the order matters: release.sh's --publish path does `gh release
  delete` + `gh release create`, which would wipe any Windows assets that
  were uploaded before it ran. This script only ever `gh release upload
  --clobber`s its own files, so running it AFTER the Mac is safe and
  idempotent.

  Required env var:
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD   password for the Tauri updater key

  Optional env var (defaults to %USERPROFILE%\.tauri\vox-updater.key):
    TAURI_SIGNING_PRIVATE_KEY            the updater private key contents

  Authenticode (SmartScreen) signing of the .exe is a SEPARATE concern and
  is intentionally out of scope — there's no code-signing certificate wired
  here. The minisign updater signature (what makes auto-update work) IS
  produced automatically by `tauri build` from the key above.

.EXAMPLE
  pwsh scripts/release-windows.ps1
      Dry run: build + sign + stage, print the merged latest.json, stop.

.EXAMPLE
  pwsh scripts/release-windows.ps1 -Publish
      Build, then add the Windows assets to the existing vN release.
#>
[CmdletBinding()]
param(
  [switch]$Publish,
  [switch]$AllowCreate,
  [string]$Target = 'x86_64-pc-windows-msvc'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

# ---------------------------------------------------------------------------
# Native-command helpers. PowerShell does NOT turn a non-zero exit code from
# an external program (gh, npm) into a terminating error consistently across
# versions — pre-7.4 it never does, 7.4+ does only when
# $PSNativeCommandUseErrorActionPreference is on. Both helpers run the call
# under ErrorActionPreference=Continue so the exit code is the single source
# of truth, then branch on $LASTEXITCODE ourselves.
# ---------------------------------------------------------------------------
function Invoke-Native {
  param(
    [Parameter(Mandatory)][scriptblock]$Script,
    [Parameter(Mandatory)][string]$What
  )
  $prev = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try { & $Script } finally { $ErrorActionPreference = $prev }
  if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

function Test-NativeSuccess {
  # Run a native probe whose failure is an expected outcome (e.g. "does this
  # release exist yet?"). Swallows all output and returns $true/$false.
  param([Parameter(Mandatory)][scriptblock]$Script)
  $prev = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try { & $Script *> $null } finally { $ErrorActionPreference = $prev }
  return ($LASTEXITCODE -eq 0)
}

# Repo root = parent of this script's directory (scripts/).
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

$RepoReleases = 'KlymSerhii/vox-releases'

# ---------------------------------------------------------------------------
# Map the Rust target triple to the Tauri NSIS arch token (used in the
# bundle filenames) and the Tauri updater platform key (used in latest.json).
# x64 today; aarch64 is wired so a future native ARM64 build is one flag.
# ---------------------------------------------------------------------------
switch ($Target) {
  'x86_64-pc-windows-msvc'  { $ArchToken = 'x64';   $UpdaterPlatform = 'windows-x86_64' }
  'aarch64-pc-windows-msvc' { $ArchToken = 'arm64'; $UpdaterPlatform = 'windows-aarch64' }
  default { throw "Unsupported -Target '$Target' (expected x86_64-pc-windows-msvc or aarch64-pc-windows-msvc)" }
}

# ---------------------------------------------------------------------------
# Resolve version from Cargo.toml — the same source release.sh and the CI use.
# ---------------------------------------------------------------------------
$verLine = Get-Content 'src-tauri/Cargo.toml' | Where-Object { $_ -match '^version\s*=' } | Select-Object -First 1
if (-not $verLine -or ($verLine -notmatch 'version\s*=\s*"([^"]+)"')) {
  throw "Could not parse version from src-tauri/Cargo.toml"
}
$Version = $Matches[1]
Write-Host "==> Vox Windows release v$Version  (target=$Target, publish=$Publish)"

# ---------------------------------------------------------------------------
# Updater signing key. `tauri build` signs the -setup.exe with this and writes
# the .sig the auto-updater verifies against tauri.conf.json's embedded
# pubkey. Mirrors release.sh: take the env var if set, else read the key file.
# ---------------------------------------------------------------------------
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
  $UpdaterKey = Join-Path $env:USERPROFILE '.tauri\vox-updater.key'
  if (-not (Test-Path $UpdaterKey)) {
    throw "$UpdaterKey not found and TAURI_SIGNING_PRIVATE_KEY is unset."
  }
  # .Trim() mirrors release.sh's `$(cat …)`, which strips the trailing
  # newline. A stray newline (or a CR from a Notepad save) would otherwise
  # reach the base64 decoder as an invalid symbol.
  $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -Raw $UpdaterKey).Trim()
}
if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
  Write-Host ""
  Write-Host "Export the updater key password, then re-run. Example:"
  Write-Host '  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "..."'
  throw "missing required env var: TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
}

# Fail early if we're going to publish but gh isn't authenticated.
if ($Publish -and -not (Test-NativeSuccess { & gh auth status })) {
  throw "-Publish requested but 'gh' is not authenticated. Run: gh auth login"
}

$Stage = Join-Path ([System.IO.Path]::GetTempPath()) ("vox-win-release-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $Stage | Out-Null

try {
  # -------------------------------------------------------------------------
  # Build the NSIS installer + the updater artifact. `createUpdaterArtifacts:
  # true` in tauri.conf.json makes the -setup.exe.sig fall out of the NSIS
  # bundle automatically once the signing key env vars (set above) are present.
  # -------------------------------------------------------------------------
  Write-Host "==> Building Windows NSIS bundle + signed updater artifact (this takes a while)..."
  Invoke-Native -What "tauri build" -Script {
    & npm run tauri build -- --target $Target --bundles nsis --verbose
  }

  # -------------------------------------------------------------------------
  # Locate the artifacts. With `createUpdaterArtifacts: true` + the NSIS
  # bundle, this Tauri version's Windows updater payload IS the -setup.exe
  # itself (installed in passive mode), signed alongside as
  # <name>-setup.exe.sig. There is no separate .nsis.zip - one .exe is BOTH
  # the user-facing download AND the auto-updater payload.
  # -------------------------------------------------------------------------
  $BundleDir = "src-tauri/target/$Target/release/bundle/nsis"
  $SetupExe  = Join-Path $BundleDir "Vox_${Version}_${ArchToken}-setup.exe"
  $SetupSig  = "${SetupExe}.sig"

  foreach ($f in @($SetupExe, $SetupSig)) {
    if (-not (Test-Path $f)) {
      Write-Host "Bundle dir contents:"
      Get-ChildItem $BundleDir -ErrorAction SilentlyContinue |
        Format-Table -AutoSize | Out-String | Write-Host
      throw "missing expected artifact: $f"
    }
  }

  # -------------------------------------------------------------------------
  # Stage under version-AGNOSTIC names so the marketing site can link to
  # /releases/latest/download/Vox_x64-setup.exe and have GitHub resolve it to
  # the newest release. These names are DISTINCT from the macOS Vox.dmg /
  # Vox.app.tar.gz, so a Windows upload can never overwrite a macOS asset.
  # -------------------------------------------------------------------------
  $StagedExe = Join-Path $Stage "Vox_${ArchToken}-setup.exe"
  $StagedSig = Join-Path $Stage "Vox_${ArchToken}-setup.exe.sig"
  Copy-Item $SetupExe $StagedExe -Force
  Copy-Item $SetupSig $StagedSig -Force

  $Signature = (Get-Content -Raw $SetupSig).Trim()
  $ExeUrl = "https://github.com/$RepoReleases/releases/download/v$Version/Vox_${ArchToken}-setup.exe"

  # -------------------------------------------------------------------------
  # Fetch the EXISTING latest.json from the release and merge our platform
  # entry into it. This is what guarantees the macOS `darwin-aarch64` entry
  # is preserved — we add `windows-x86_64`, we never rewrite the whole file.
  # -------------------------------------------------------------------------
  $ExistingDir = Join-Path $Stage 'existing'
  New-Item -ItemType Directory -Force -Path $ExistingDir | Out-Null
  $HaveExisting = Test-NativeSuccess {
    & gh release download "v$Version" --repo $RepoReleases --pattern 'latest.json' --dir $ExistingDir
  }
  $ExistingLatest = Join-Path $ExistingDir 'latest.json'

  if ($HaveExisting -and (Test-Path $ExistingLatest)) {
    Write-Host "==> Merging into existing latest.json (preserves the macOS entry)."
    $manifest = Get-Content -Raw $ExistingLatest | ConvertFrom-Json
  }
  else {
    if (-not $AllowCreate) {
      throw @"
No existing latest.json for v$Version on $RepoReleases.

The macOS release must be published FIRST so the Windows build merges into
it - this keeps darwin-aarch64 in the manifest and the Vox.dmg download link
intact:

    scripts/release.sh --publish                 # on the signing Mac
    pwsh scripts/release-windows.ps1 -Publish     # then here

To deliberately cut a Windows-ONLY release (no macOS entry), re-run with
-AllowCreate.
"@
    }
    Write-Warning "No existing release for v$Version - building a Windows-only manifest (no darwin-aarch64 entry)."
    $PubDate = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    $manifest = [pscustomobject]@{
      version   = $Version
      notes     = "Windows release v$Version."
      pub_date  = $PubDate
      platforms = [pscustomobject]@{}
    }
  }

  # Defensive: a hand-edited manifest could be missing `platforms`.
  if (-not $manifest.PSObject.Properties['platforms'] -or ($null -eq $manifest.platforms)) {
    $manifest | Add-Member -NotePropertyName 'platforms' -NotePropertyValue ([pscustomobject]@{}) -Force
  }

  # Add (or replace) only OUR platform key. -Force overwrites a stale Windows
  # entry from a previous run; every other key (notably darwin-aarch64) is
  # left exactly as it was.
  $winEntry = [pscustomobject]@{ signature = $Signature; url = $ExeUrl }
  $manifest.platforms | Add-Member -NotePropertyName $UpdaterPlatform -NotePropertyValue $winEntry -Force

  # Write the merged manifest without a BOM — a UTF-8 BOM trips some strict
  # JSON parsers and there's no reason to emit one.
  $MergedLatest = Join-Path $Stage 'latest.json'
  $json = $manifest | ConvertTo-Json -Depth 12
  [System.IO.File]::WriteAllText($MergedLatest, $json, (New-Object System.Text.UTF8Encoding($false)))

  Write-Host ""
  Write-Host "==> Merged latest.json:"
  Get-Content $MergedLatest | Write-Host
  Write-Host ""
  Write-Host "==> Staged artifacts in ${Stage}:"
  Get-ChildItem $Stage -File | Select-Object Name, Length | Format-Table -AutoSize | Out-String | Write-Host

  # -------------------------------------------------------------------------
  # Dry run: copy artifacts somewhere durable (the temp stage is about to be
  # cleaned), print the publish hint, and stop.
  # -------------------------------------------------------------------------
  if (-not $Publish) {
    $Out = "src-tauri/target/$Target/release/vox-win-release-v$Version"
    if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
    New-Item -ItemType Directory -Force -Path $Out | Out-Null
    Copy-Item $StagedExe, $StagedSig, $MergedLatest $Out -Force
    Write-Host ""
    Write-Host "============================================================="
    Write-Host "Dry run complete - nothing was published."
    Write-Host "Artifacts copied to: $Out"
    Write-Host ""
    Write-Host "Publish onto the EXISTING v$Version release (adds Windows, keeps macOS):"
    Write-Host ""
    Write-Host "    pwsh scripts/release-windows.ps1 -Publish"
    Write-Host "============================================================="
    return
  }

  # -------------------------------------------------------------------------
  # Publish. Upload ONTO the existing release with --clobber so we replace
  # only our own assets (latest.json + the three Windows files). We never
  # `gh release delete`, so Vox.dmg / Vox.app.tar.gz stay put.
  # -------------------------------------------------------------------------
  $releaseExists = Test-NativeSuccess { & gh release view "v$Version" --repo $RepoReleases }
  if (-not $releaseExists) {
    if (-not $AllowCreate) {
      throw "Release v$Version does not exist on $RepoReleases. Publish the macOS release first (scripts/release.sh --publish), or re-run with -AllowCreate for a Windows-only release."
    }
    Write-Warning "Creating a NEW Windows-only release v$Version (no macOS assets)."
    Invoke-Native -What "gh release create" -Script {
      & gh release create "v$Version" --repo $RepoReleases --title "Vox $Version" --notes "Windows release v$Version."
    }
  }

  Write-Host "==> Uploading Windows assets onto v$Version (clobbering only our own; macOS assets untouched)..."
  Invoke-Native -What "gh release upload" -Script {
    & gh release upload "v$Version" `
      $StagedExe `
      $StagedSig `
      $MergedLatest `
      --repo $RepoReleases `
      --clobber
  }

  Write-Host ""
  Write-Host "Published Windows artifacts for v${Version}:"
  Write-Host "  Release:            https://github.com/$RepoReleases/releases/tag/v$Version"
  Write-Host "  Windows download:   https://github.com/$RepoReleases/releases/latest/download/Vox_${ArchToken}-setup.exe"
  Write-Host "  macOS download:     https://github.com/$RepoReleases/releases/latest/download/Vox.dmg  (untouched)"
}
finally {
  if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
}
