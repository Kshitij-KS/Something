$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$workspace = Split-Path -Parent $PSScriptRoot
Push-Location $workspace
try {
  & node scripts/verify-release-metadata.mjs
  if ($LASTEXITCODE -ne 0) { throw 'Release metadata verification failed' }
  & node scripts/validate-extension.mjs extension/dist/manifest.json
  if ($LASTEXITCODE -ne 0) { throw 'Built extension validation failed' }

  $targetRoot = if ($env:CARGO_TARGET_DIR) {
    if ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
      $env:CARGO_TARGET_DIR
    } else {
      Join-Path $workspace $env:CARGO_TARGET_DIR
    }
  } else {
    Join-Path $workspace 'target'
  }
  $releaseRoot = Join-Path $targetRoot 'release'
  foreach ($required in @(
    (Join-Path $releaseRoot 'callback-app.exe'),
    (Join-Path $releaseRoot 'callback-native-host.exe')
  )) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
      throw "Missing release executable: $([IO.Path]::GetFileName($required))"
    }
  }

  $stagedHosts = @(
    Get-ChildItem -Path 'src-tauri/binaries/callback-native-host-*.exe' -File -ErrorAction SilentlyContinue
  )
  if ($stagedHosts.Count -ne 1) {
    throw "Expected one staged Tauri sidecar, found $($stagedHosts.Count)"
  }

  $bundleRoots = @(
    (Join-Path $releaseRoot 'bundle'),
    (Join-Path $workspace 'src-tauri/target/release/bundle')
  ) | Select-Object -Unique | Where-Object { Test-Path -LiteralPath $_ -PathType Container }
  $nsis = @(
    foreach ($root in $bundleRoots) {
      $directory = Join-Path $root 'nsis'
      if (Test-Path -LiteralPath $directory -PathType Container) {
        Get-ChildItem -LiteralPath $directory -Filter '*.exe' -File
      }
    }
  )
  $msi = @(
    foreach ($root in $bundleRoots) {
      $directory = Join-Path $root 'msi'
      if (Test-Path -LiteralPath $directory -PathType Container) {
        Get-ChildItem -LiteralPath $directory -Filter '*.msi' -File
      }
    }
  )
  if ($nsis.Count -ne 1) { throw "Expected exactly one NSIS installer, found $($nsis.Count)" }
  if ($msi.Count -ne 1) { throw "Expected exactly one MSI installer, found $($msi.Count)" }

  $artifactDirectory = Join-Path $workspace 'artifacts'
  if (Test-Path -LiteralPath $artifactDirectory) {
    Remove-Item -LiteralPath $artifactDirectory -Recurse -Force
  }
  New-Item -ItemType Directory -Path $artifactDirectory | Out-Null
  Copy-Item -LiteralPath $nsis[0].FullName -Destination $artifactDirectory
  Copy-Item -LiteralPath $msi[0].FullName -Destination $artifactDirectory

  $package = Get-Content -LiteralPath 'package.json' -Raw | ConvertFrom-Json
  $extensionDirectory = Join-Path $workspace 'extension/dist'
  foreach ($required in @('manifest.json', 'background.js', 'content.js', 'selectors.json')) {
    if (-not (Test-Path -LiteralPath (Join-Path $extensionDirectory $required) -PathType Leaf)) {
      throw "Built extension is missing $required"
    }
  }
  $extensionZip = Join-Path $artifactDirectory "callback-extension-$($package.version).zip"
  Compress-Archive -Path (Join-Path $extensionDirectory '*') -DestinationPath $extensionZip -CompressionLevel Optimal

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $archive = [IO.Compression.ZipFile]::OpenRead($extensionZip)
  try {
    $entries = @($archive.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
    foreach ($required in @('manifest.json', 'background.js', 'content.js', 'selectors.json')) {
      if ($entries -notcontains $required) { throw "Extension ZIP is missing root entry $required" }
    }
    if (@($entries | Group-Object | Where-Object Count -gt 1).Count -ne 0) {
      throw 'Extension ZIP contains duplicate entries'
    }
    if (@($entries | Where-Object { $_.StartsWith('/') -or $_ -match '(^|/)\.\.(/|$)' }).Count -ne 0) {
      throw 'Extension ZIP contains an unsafe entry path'
    }
  } finally {
    $archive.Dispose()
  }

  $assets = @(
    Get-ChildItem -LiteralPath $artifactDirectory -File |
      Where-Object Name -ne 'SHA256SUMS.txt' |
      Sort-Object Name
  )
  if ($assets.Count -ne 3) {
    throw "Expected NSIS, MSI, and extension ZIP assets, found $($assets.Count)"
  }
  $sumFile = Join-Path $artifactDirectory 'SHA256SUMS.txt'
  $checksumLines = @(
    foreach ($asset in $assets) {
      $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $asset.FullName).Hash.ToLowerInvariant()
      "$hash *$($asset.Name)"
    }
  )
  Set-Content -LiteralPath $sumFile -Value $checksumLines -Encoding Ascii

  foreach ($line in Get-Content -LiteralPath $sumFile) {
    if ($line -notmatch '^([0-9a-f]{64}) \*(.+)$') { throw 'Malformed SHA256SUMS.txt entry' }
    $name = $Matches[2]
    if ([IO.Path]::GetFileName($name) -ne $name) { throw 'Checksum entry must contain a base filename' }
    $assetPath = Join-Path $artifactDirectory $name
    if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) { throw "Checksum asset is missing: $name" }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $assetPath).Hash.ToLowerInvariant()
    if ($actual -ne $Matches[1]) { throw "Checksum verification failed: $name" }
  }

  Write-Host "Packaged unsigned release-candidate artifacts:"
  Get-ChildItem -LiteralPath $artifactDirectory -File | Sort-Object Name | ForEach-Object {
    Write-Host "  $($_.Name)"
  }
  Write-Host 'SHA-256 checksums verify integrity only; they are not Authenticode signatures.'
} finally {
  Pop-Location
}
