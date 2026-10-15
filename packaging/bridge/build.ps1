# Builds the bridge jar: BlockForge-<version>.jar
#   .\build.ps1 [-Version 4.0.0] [-Out C:\some\dir]
# Needs a JDK (javac and jar), 8 or newer. The jar runs on Java 8 and up.
param(
    [string]$Version = "",
    [string]$Out = ""
)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not $Version) {
    $line = Select-String -Path (Join-Path $here "..\..\Cargo.toml") -Pattern '^version\s*=\s*"(.*)"' | Select-Object -First 1
    if ($line) { $Version = $line.Matches[0].Groups[1].Value }
}
if (-not $Version) { throw "Could not work out the version; pass -Version." }
if (-not $Out) { $Out = Join-Path $here "dist" }

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("blockforge-bridge-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path (Join-Path $work "classes") -Force | Out-Null
New-Item -ItemType Directory -Path $Out -Force | Out-Null
try {
    $src = Join-Path $here "Bridge.java"
    & javac --release 8 "-Xlint:-options" -encoding UTF-8 -d (Join-Path $work "classes") $src
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "javac has no --release 8, using --release 11"
        & javac --release 11 -encoding UTF-8 -d (Join-Path $work "classes") $src
        if ($LASTEXITCODE -ne 0) { throw "javac failed" }
    }

    $manifest = Join-Path $work "manifest.txt"
    $text = "Main-Class: com.blockforge.launcher.Bridge`r`nImplementation-Title: BlockForge`r`nImplementation-Version: $Version`r`n"
    [System.IO.File]::WriteAllText($manifest, $text, (New-Object System.Text.UTF8Encoding($false)))

    $jar = Join-Path $Out "BlockForge-$Version.jar"
    & jar cfm $jar $manifest -C (Join-Path $work "classes") .
    if ($LASTEXITCODE -ne 0) { throw "jar failed" }
    Write-Host "Built $jar"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
