# Download the models listed in config/models.json (same manifest as the UI's
# "Models" tab).
#
#   .\scripts\download_models.ps1                # models needed for -Mode full
#   .\scripts\download_models.ps1 -Mode duplex   # full | listen | duplex
#   .\scripts\download_models.ps1 -All
#
# Requires: PowerShell 5.1+, Windows 10+ (tar.exe for .tar.bz2 archives).
param(
    [ValidateSet("full", "listen", "duplex")] [string]$Mode = "full",
    [switch]$All
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest is ~10x faster without the progress bar

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Manifest = (Get-Content (Join-Path $Root "config\models.json") -Raw -Encoding UTF8 | ConvertFrom-Json).models
$Tar = Join-Path $env:SystemRoot "System32\tar.exe"

foreach ($m in $Manifest) {
    if (-not $All -and -not (@($m.needed) -contains $Mode)) { continue }
    $check = Join-Path $Root $m.check
    if (Test-Path $check) { Write-Host "[ok]   $($m.name)" -ForegroundColor Gray; continue }

    Write-Host "[get]  $($m.name) (~$($m.size_mb) MB)" -ForegroundColor Yellow
    if ($m.archive) {
        $tmp = Join-Path $env:TEMP ([System.IO.Path]::GetFileName($m.archive))
        Invoke-WebRequest -Uri $m.archive -OutFile $tmp -UseBasicParsing
        & $Tar -xjf $tmp -C (Join-Path $Root "models")
        if ($LASTEXITCODE -ne 0) { throw "tar failed for $tmp" }
        Remove-Item -LiteralPath $tmp
    } else {
        foreach ($f in $m.files) {
            $dest = Join-Path $Root $f.path
            if (Test-Path $dest) { continue }
            New-Item -ItemType Directory -Force -Path (Split-Path $dest) | Out-Null
            Invoke-WebRequest -Uri $f.url -OutFile "$dest.part" -UseBasicParsing
            Move-Item -LiteralPath "$dest.part" -Destination $dest -Force
        }
    }
    Write-Host "[done] $($m.name)" -ForegroundColor Green
}
