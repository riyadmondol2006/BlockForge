# Installs the BlockForge setup silently into a temporary folder, checks what it left behind, runs the program, then
# uninstalls it silently and checks that it is gone. It also pretends that the Java edition is installed, to check
# that the setup finds it, and that the data folder survives everything. Meant for CI, but fine on a dev machine that
# has no BlockForge installed (it stops if one is).
#
#   packaging\windows\smoke.ps1 [-Installer dist\BlockForge-4.0.0-windows-x64-setup.exe] [-LaunchGui] [-SkipLegacyTest]
#
#   -LaunchGui        also starts the window for a few seconds and checks that it stays up (needs OpenGL 2)
#   -SkipLegacyTest   leaves out the pretend Java edition and the marker file in %APPDATA%\BlockForge
param(
    [string]$Installer,
    [switch]$LaunchGui,
    [switch]$SkipLegacyTest
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$crate = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Installer) {
    $found = Get-ChildItem (Join-Path $crate 'dist\BlockForge-*-windows-x64-setup.exe') -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $found) { throw 'no setup in dist\ (build it with packaging\windows\build.ps1, or pass -Installer)' }
    $Installer = $found.FullName
}
if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) { throw "setup not found: $Installer" }
$Installer = (Resolve-Path -LiteralPath $Installer).Path
if ((Split-Path -Leaf $Installer) -match '^BlockForge-(\d+(\.\d+){2,3})-windows-x64-setup\.exe$') {
    $version = $Matches[1]
}
else {
    throw "the setup should be called BlockForge-<version>-windows-x64-setup.exe, this one is $(Split-Path -Leaf $Installer)"
}

$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$appKey = "$uninstallRoot\{32084B46-4379-48D5-AF59-3010DEB6CBFA}_is1"
$appPathsKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\blockforge.exe'
$startMenuLink = Join-Path ([Environment]::GetFolderPath('Programs')) 'BlockForge.lnk'
$desktopLink = Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'BlockForge.lnk'
$dataDir = Join-Path $env:APPDATA 'BlockForge'
$marker = Join-Path $dataDir 'smoke-test-marker.txt'
$work = Join-Path ([IO.Path]::GetTempPath()) ('blockforge-smoke-' + [guid]::NewGuid())
$installDir = Join-Path $work 'app'
$setupLog = Join-Path $work 'setup.log'
$fakeCode = '{' + [guid]::NewGuid().ToString().ToUpperInvariant() + '}'
$fakeKey = "$uninstallRoot\$fakeCode"
$script:problems = 0

function Pass([string]$what) { Write-Output "  ok    $what" }
function Check([bool]$condition, [string]$what) {
    if ($condition) { Pass $what } else { Write-Output "  FAIL  $what"; $script:problems++ }
}
function Run-Program([string]$exe, [string[]]$arguments, [int]$seconds = 60) {
    # catches what a windowed program prints to a file, the way CI sees it
    $outFile = Join-Path $work ([guid]::NewGuid().ToString() + '.out')
    $errFile = Join-Path $work ([guid]::NewGuid().ToString() + '.err')
    $p = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru -NoNewWindow `
        -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    if (-not $p.WaitForExit($seconds * 1000)) { $p.Kill(); throw "$exe $arguments did not finish in $seconds seconds" }
    $p.WaitForExit()
    [pscustomobject]@{
        Code = $p.ExitCode
        Out  = (Get-Content -Raw -LiteralPath $outFile -ErrorAction SilentlyContinue)
        Err  = (Get-Content -Raw -LiteralPath $errFile -ErrorAction SilentlyContinue)
    }
}

if (Test-Path -LiteralPath $appKey) {
    throw "BlockForge $((Get-ItemProperty $appKey).DisplayVersion) is installed on this computer already. Uninstall it first: this test installs and removes it."
}
New-Item -ItemType Directory -Path $work | Out-Null
$haveMarker = $false
try {
    Write-Output "Smoke test of $Installer"

    # ---- a pretend Java edition and a file in the data folder -------------------------------------------------------
    if (-not $SkipLegacyTest) {
        New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
        Set-Content -LiteralPath $marker -Value 'this file must survive the install and the uninstall'
        $haveMarker = $true
        New-Item -Path $fakeKey -Force | Out-Null
        Set-ItemProperty -Path $fakeKey -Name DisplayName -Value 'BlockForge'
        Set-ItemProperty -Path $fakeKey -Name UninstallString -Value "MsiExec.exe /X$fakeCode"
        Set-ItemProperty -Path $fakeKey -Name InstallLocation -Value (Join-Path $work 'java-edition')
    }

    # ---- install ----------------------------------------------------------------------------------------------------
    Write-Output 'Installing silently'
    $setup = Start-Process -FilePath $Installer -Wait -PassThru -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', "/DIR=`"$installDir`"", "/LOG=`"$setupLog`"")
    Check ($setup.ExitCode -eq 0) "setup exit code is 0 (was $($setup.ExitCode))"
    if ($setup.ExitCode -ne 0 -and (Test-Path $setupLog)) { Get-Content $setupLog -Tail 40 }

    $exe = Join-Path $installDir 'blockforge.exe'
    Check (Test-Path -LiteralPath $exe) 'blockforge.exe was installed'
    Check (Test-Path -LiteralPath (Join-Path $installDir 'unins000.exe')) 'the uninstaller was installed'
    if (Test-Path -LiteralPath $exe) {
        $vi = (Get-Item -LiteralPath $exe).VersionInfo
        Check ($vi.ProductName -eq 'BlockForge') "exe product name is BlockForge (was '$($vi.ProductName)')"
        Check ($vi.ProductVersion -like "$(($version -split '\.')[0..2] -join '.')*") "exe version is $version (was '$($vi.ProductVersion)')"
    }

    # shortcuts
    $shell = New-Object -ComObject WScript.Shell
    foreach ($link in @($startMenuLink, $desktopLink)) {
        $name = if ($link -eq $startMenuLink) { 'Start Menu' } else { 'desktop' }
        Check (Test-Path -LiteralPath $link) "$name shortcut exists ($link)"
        if (Test-Path -LiteralPath $link) {
            $target = $shell.CreateShortcut($link).TargetPath
            Check ($target -ieq $exe) "$name shortcut points at the installed exe ($target)"
            $folder = (New-Object -ComObject Shell.Application).NameSpace((Split-Path -Parent $link))
            $id = $folder.ParseName((Split-Path -Leaf $link)).ExtendedProperty('System.AppUserModel.ID')
            if ($id) {
                Check ($id -eq 'io.github.riyadmondol2006.BlockForge') "$name shortcut has the AppUserModelID ($id)"
            }
            else {
                Write-Warning "could not read the AppUserModelID of the $name shortcut"
            }
        }
    }

    # the entry in Settings > Apps
    Check (Test-Path -LiteralPath $appKey) 'uninstall entry exists'
    if (Test-Path -LiteralPath $appKey) {
        $entry = Get-ItemProperty -LiteralPath $appKey
        Check ($entry.DisplayName -eq 'BlockForge') "uninstall entry name is BlockForge (was '$($entry.DisplayName)')"
        Check ($entry.DisplayVersion -eq $version) "uninstall entry version is $version (was '$($entry.DisplayVersion)')"
        Check ($entry.DisplayIcon -like '*blockforge.exe*') "uninstall entry has the icon ($($entry.DisplayIcon))"
        Check ([bool]$entry.UninstallString) 'uninstall entry has an uninstall command'
        Check ($entry.Publisher -eq 'BlockForge') "uninstall entry publisher is BlockForge (was '$($entry.Publisher)')"
    }
    Check (Test-Path -LiteralPath $appPathsKey) 'App Paths entry exists (Win+R "blockforge")'

    # the Java edition
    if (-not $SkipLegacyTest) {
        $log = Get-Content -Raw -LiteralPath $setupLog
        Check ($log -match [regex]::Escape("Java edition found: product $fakeCode")) 'the setup found the (pretend) Java edition'
        Check ($log -match [regex]::Escape("Removing the Java edition $fakeCode")) 'the setup tried to remove it'
        Check (Test-Path -LiteralPath $marker) 'the data folder was not touched by the install'
    }

    # ---- run it -----------------------------------------------------------------------------------------------------
    $env:BLOCKFORGE_DIR = Join-Path $work 'data'
    $v = Run-Program $exe @('--version')
    Check ($v.Code -eq 0) "blockforge.exe --version exits with 0 (was $($v.Code))"
    Check ("$($v.Out)" -match "^blockforge\s+$([regex]::Escape(($version -split '\.')[0..2] -join '.'))") "--version prints the version, redirected to a file (was '$("$($v.Out)".Trim())')"
    $h = Run-Program $exe @('--help')
    Check ($h.Code -eq 0 -and "$($h.Out)" -match 'launch') '--help lists the commands'
    $pt = Run-Program $exe @('paths')
    Check ($pt.Code -eq 0 -and "$($pt.Out)" -match [regex]::Escape($env:BLOCKFORGE_DIR)) "paths prints the data folder (BLOCKFORGE_DIR is honoured)"
    $ls = Run-Program $exe @('list')
    Check ($ls.Code -eq 0) "blockforge.exe list exits with 0 (was $($ls.Code)) $($ls.Err)"

    if ($LaunchGui) {
        Write-Output 'Starting the window'
        $gui = Start-Process -FilePath $exe -PassThru
        Start-Sleep -Seconds 8
        Check (-not $gui.HasExited) 'the window stays open for 8 seconds'
        if (-not $gui.HasExited) { $gui.Kill(); $gui.WaitForExit() }
    }
    Remove-Item Env:\BLOCKFORGE_DIR -ErrorAction SilentlyContinue

    # ---- uninstall --------------------------------------------------------------------------------------------------
    Write-Output 'Uninstalling silently'
    $unins = Join-Path $installDir 'unins000.exe'
    if (Test-Path -LiteralPath $unins) {
        Start-Process -FilePath $unins -Wait -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') | Out-Null
        # the uninstaller finishes in a copy of itself, so give it a moment
        $deadline = (Get-Date).AddSeconds(60)
        while ((Test-Path -LiteralPath $exe) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    }
    Check (-not (Test-Path -LiteralPath $exe)) 'blockforge.exe was removed'
    Check (-not (Test-Path -LiteralPath $appKey)) 'the uninstall entry was removed'
    Check (-not (Test-Path -LiteralPath $appPathsKey)) 'the App Paths entry was removed'
    Check (-not (Test-Path -LiteralPath $startMenuLink)) 'the Start Menu shortcut was removed'
    Check (-not (Test-Path -LiteralPath $desktopLink)) 'the desktop shortcut was removed'
    if (-not $SkipLegacyTest) {
        Check (Test-Path -LiteralPath $marker) 'the data folder survived the uninstall'
    }
}
catch {
    Write-Output "  FAIL  $($_.Exception.Message)"
    $script:problems++
}
finally {
    Remove-Item Env:\BLOCKFORGE_DIR -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $fakeKey -Force -ErrorAction SilentlyContinue
    if ($haveMarker) { Remove-Item -LiteralPath $marker -Force -ErrorAction SilentlyContinue }
    # something failed halfway: don't leave an install behind
    $leftover = Join-Path $installDir 'unins000.exe'
    if (Test-Path -LiteralPath $leftover) {
        Start-Process -FilePath $leftover -Wait -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') -ErrorAction SilentlyContinue | Out-Null
        Start-Sleep -Seconds 3
    }
    Remove-Item -Recurse -Force -LiteralPath $work -ErrorAction SilentlyContinue
}

if ($script:problems -gt 0) {
    Write-Output "Smoke test FAILED: $script:problems problem(s)"
    exit 1
}
Write-Output 'Smoke test passed'
