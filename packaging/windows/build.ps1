# Builds the Windows release files from an already built blockforge.exe:
#
#   BlockForge-<version>-windows-x64-setup.exe     the installer (Inno Setup, installs for the current user)
#   BlockForge-<version>-windows-x64-portable.zip  blockforge.exe and a README, for people who don't want to install
#
#   packaging\windows\build.ps1 -Version 4.0.0 [-Exe target\release\blockforge.exe] [-OutDir dist]
#
# Build the program first with `cargo build --release` on Windows (that is what puts the icon and version
# information into the exe). Inno Setup 6 is looked for in its usual folders; GitHub's windows runners have it, and
# on any other computer it is installed with Chocolatey when it is missing.
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$Exe,
    [string]$OutDir
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Exe) { $Exe = Join-Path $crate 'target\release\blockforge.exe' }
if (-not $OutDir) { $OutDir = Join-Path $crate 'dist' }

if ($Version -notmatch '^\d+\.\d+\.\d+(\.\d+)?$') {
    throw "-Version must look like 4.0.0 (or 4.0.0.1), got '$Version'"
}
$quad = if (($Version -split '\.').Count -eq 3) { "$Version.0" } else { $Version }

if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) {
    throw "blockforge.exe not found at $Exe. Build it first: cargo build --release"
}
$Exe = (Resolve-Path -LiteralPath $Exe).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path

# ---- check the exe before it is wrapped: a wrong one would only show up on somebody's computer ----------------------
$info = (Get-Item -LiteralPath $Exe).VersionInfo
if ($info.ProductName -ne 'BlockForge') {
    throw "$Exe has no BlockForge version information (ProductName is '$($info.ProductName)'): build.rs embeds it with the Windows resource compiler, so the exe must be built on Windows with cargo build --release"
}
$exeVersion = ($info.ProductVersion -split '\.')[0..2] -join '.'
if ($exeVersion -ne (($Version -split '\.')[0..2] -join '.')) {
    throw "$Exe is version $($info.ProductVersion) but -Version is $Version. Rebuild it, or pass the version it has."
}
# the Subsystem field of the PE header: 2 is a windowed program (no console window), 3 is a console program
$bytes = [IO.File]::ReadAllBytes($Exe)
$peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
$subsystem = [BitConverter]::ToUInt16($bytes, $peOffset + 24 + 68)
if ($subsystem -ne 2) {
    throw "$Exe is a console program (subsystem $subsystem), the release build must be a windowed one. Build it with --release, without a custom windows_subsystem override."
}
Write-Output "blockforge.exe $($info.ProductVersion), $([math]::Round($bytes.Length / 1MB, 1)) MB, windowed program, version information present"

# ---- Inno Setup --------------------------------------------------------------------------------------------------------
function Find-Iscc {
    $found = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    $folders = @(${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA 'Programs')) |
        Where-Object { $_ }
    foreach ($folder in $folders) {
        $candidate = Join-Path $folder 'Inno Setup 6\ISCC.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    return $null
}
$iscc = Find-Iscc
if (-not $iscc) {
    Write-Output 'Inno Setup 6 not found, installing it with Chocolatey'
    if (-not (Get-Command choco.exe -ErrorAction SilentlyContinue)) {
        throw 'Inno Setup 6 is not installed and Chocolatey is not available to install it. Get it from https://jrsoftware.org/isdl.php'
    }
    choco install innosetup -y --no-progress
    if ($LASTEXITCODE -ne 0) { throw "choco install innosetup failed with exit code $LASTEXITCODE" }
    $iscc = Find-Iscc
    if (-not $iscc) { throw 'Inno Setup was installed but ISCC.exe was not found' }
}
Write-Output "Using $iscc"

$setupName = "BlockForge-$Version-windows-x64-setup.exe"
$setupPath = Join-Path $OutDir $setupName
Remove-Item -LiteralPath $setupPath -Force -ErrorAction SilentlyContinue
& $iscc '/Q' "/DAppVersion=$Version" "/DAppVersionQuad=$quad" "/DExeSource=$Exe" "/O$OutDir" `
    (Join-Path $PSScriptRoot 'installer.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
if (-not (Test-Path -LiteralPath $setupPath)) { throw "ISCC finished but $setupPath does not exist" }

# ---- portable zip ------------------------------------------------------------------------------------------------------
$zipName = "BlockForge-$Version-windows-x64-portable.zip"
$zipPath = Join-Path $OutDir $zipName
$stage = Join-Path ([IO.Path]::GetTempPath()) ('blockforge-portable-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $stage 'blockforge.exe')
    $readme = (Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'PORTABLE-README.txt')).Replace('{{VERSION}}', $Version)
    # Notepad on older Windows needs the CR LF line endings
    [IO.File]::WriteAllText((Join-Path $stage 'README.txt'), ($readme -replace "`r?`n", "`r`n"), (New-Object Text.UTF8Encoding $false))
    Remove-Item -LiteralPath $zipPath -Force -ErrorAction SilentlyContinue
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal
}
finally {
    Remove-Item -Recurse -Force -LiteralPath $stage -ErrorAction SilentlyContinue
}
if (-not (Test-Path -LiteralPath $zipPath)) { throw "$zipPath was not made" }

foreach ($file in @($setupPath, $zipPath)) {
    $item = Get-Item -LiteralPath $file
    $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Output ("Built {0} ({1:N1} MB) sha256 {2}" -f $item.FullName, ($item.Length / 1MB), $hash)
}
