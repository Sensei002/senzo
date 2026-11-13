$ErrorActionPreference = "Stop"

$mpvVersion = "0.42.0"
$assetName = "libmpv-v$mpvVersion-x86_64-w64-mingw32-lgpl.zip"
$assetUrl = "https://github.com/mpv-player/mpv/releases/download/v$mpvVersion/$assetName"
$workRoot = Join-Path $env:RUNNER_TEMP "senzo-mpv-runtime"
$archivePath = Join-Path $workRoot $assetName
$extractPath = Join-Path $workRoot "extracted"
$runtimePath = Join-Path $env:GITHUB_WORKSPACE "runtime"

if (Test-Path $workRoot) {
    Remove-Item $workRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $extractPath -Force | Out-Null

Write-Host "Downloading upstream mpv $mpvVersion LGPL x64 runtime"
Invoke-WebRequest -Uri $assetUrl -OutFile $archivePath
Expand-Archive -LiteralPath $archivePath -DestinationPath $extractPath -Force

$mainDll = Get-ChildItem -LiteralPath $extractPath -Filter "libmpv-2.dll" -File -Recurse |
    Select-Object -First 1
if ($null -eq $mainDll) {
    throw "The upstream archive did not contain libmpv-2.dll"
}

if (Test-Path $runtimePath) {
    Remove-Item $runtimePath -Recurse -Force
}
New-Item -ItemType Directory -Path $runtimePath -Force | Out-Null
$licensesPath = Join-Path $runtimePath "licenses"
New-Item -ItemType Directory -Path $licensesPath -Force | Out-Null

# Windows resolves dependent DLLs beside the loaded module. Flatten the
# upstream package's DLLs into the runtime directory and reject collisions.
$copied = @{}
Get-ChildItem -LiteralPath $extractPath -Filter "*.dll" -File -Recurse | ForEach-Object {
    $destination = Join-Path $runtimePath $_.Name
    $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
    if ($copied.ContainsKey($_.Name)) {
        if ($copied[$_.Name] -ne $hash) {
            throw "Conflicting upstream DLLs share the filename $($_.Name)"
        }
        return
    }
    Copy-Item -LiteralPath $_.FullName -Destination $destination
    $copied[$_.Name] = $hash
}

if (-not (Test-Path (Join-Path $runtimePath "libmpv-2.dll"))) {
    throw "libmpv-2.dll was not staged in the runtime directory"
}

# Retain upstream legal notices with their relative paths encoded in filenames.
Get-ChildItem -LiteralPath $extractPath -File -Recurse |
    Where-Object { $_.Name -match '^(LICENSE|COPYING|COPYRIGHT|NOTICE)' } |
    ForEach-Object {
        $relative = [System.IO.Path]::GetRelativePath($extractPath, $_.FullName)
        $safeName = ($relative -replace '[\\/]', '_')
        Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $licensesPath $safeName)
    }

$archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash
$runtimeHash = (Get-FileHash -LiteralPath (Join-Path $runtimePath "libmpv-2.dll") -Algorithm SHA256).Hash
$manifest = [ordered]@{
    provider = "mpv-player/mpv"
    version = $mpvVersion
    asset = $assetName
    source = $assetUrl
    variant = "LGPL"
    architecture = "x86_64"
    archive_sha256 = $archiveHash
    libmpv_sha256 = $runtimeHash
    packaged_dlls = @($copied.Keys | Sort-Object)
}
$manifest | ConvertTo-Json -Depth 5 |
    Set-Content -LiteralPath (Join-Path $runtimePath "runtime-manifest.json") -Encoding utf8

Write-Host "Packaged $($copied.Count) DLLs from mpv $mpvVersion"
Get-ChildItem -LiteralPath $runtimePath -File | Select-Object Name, Length
