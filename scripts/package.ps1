# Builds the Windows installer (an .exe that installs for the current user, no administrator needed). It carries its
# own Java, so people don't need to install Java first.
#
#   scripts\package.ps1 [-Jar path\to\BlockForge-x.y.z.jar] [-Out build\installers] [-Type exe|msi|app-image]
#
# Needs a JDK 17 or newer with jpackage (JAVA_HOME is used when set). The exe and msi types also need the
# WiX Toolset 3 (https://wixtoolset.org); the jpackage of JDK 17 and 21 doesn't work with WiX 4 or newer.
# The result is build\installers\BlockForge-<version>-windows-x64.exe
param(
    [string]$Jar,
    [string]$Out = (Join-Path $PSScriptRoot '..\build\installers'),
    [ValidateSet('exe', 'msi', 'app-image')]
    [string]$Type = 'exe'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Jar) {
    $found = Get-ChildItem (Join-Path $root 'build\libs\BlockForge-*.jar') -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $found) { throw 'no jar in build\libs (build it with gradlew shadowJar, or pass -Jar)' }
    $Jar = $found.FullName
}
if (-not (Test-Path -LiteralPath $Jar -PathType Leaf)) { throw "jar not found: $Jar" }
$Jar = (Resolve-Path -LiteralPath $Jar).Path

# BlockForge-3.4.41.4.jar -> 3.4.41.4. The installer keeps all four numbers: jpackage compares the fourth one too
# when it decides whether to replace an older install, and the id of the install depends on the whole version.
$version = [IO.Path]::GetFileNameWithoutExtension($Jar) -replace '^BlockForge-', '' -replace '\.Beta$', ''
if ($version -notmatch '^\d+(\.\d+){1,3}$') { throw "can't read a version from the jar name: $Jar" }

$jpackage = 'jpackage'
if ($env:JAVA_HOME -and (Test-Path (Join-Path $env:JAVA_HOME 'bin\jpackage.exe'))) {
    $jpackage = Join-Path $env:JAVA_HOME 'bin\jpackage.exe'
}

$modules = 'java.se,jdk.unsupported,jdk.crypto.ec,jdk.crypto.cryptoki,jdk.localedata,jdk.zipfs,jdk.charsets,' +
    'jdk.management,jdk.management.agent,jdk.net,jdk.httpserver,jdk.jfr,jdk.xml.dom,jdk.naming.dns,' +
    'jdk.security.auth,jdk.security.jgss'

$stage = Join-Path ([IO.Path]::GetTempPath()) ('blockforge-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -LiteralPath $Jar -Destination $stage
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path -LiteralPath $Out).Path

# Every argument is a quoted string: PowerShell splits a bare -Dname.with.dots=value into two arguments.
$arguments = @(
    '--type', $Type,
    '--name', 'BlockForge',
    '--app-version', $version,
    '--vendor', 'BlockForge',
    '--description', 'A Minecraft launcher with a modern interface that plays offline without an account.',
    '--copyright', 'Copyright ATLauncher and BlockForge contributors, GPL-3.0',
    '--input', $stage,
    '--main-jar', (Split-Path -Leaf $Jar),
    '--main-class', 'com.blockforge.launcher.App',
    '--add-modules', $modules,
    '--jlink-options', '--strip-debug --no-header-files --no-man-pages',
    '--java-options', '-Dblockforge.installed=true',
    '--java-options', '-Dawt.useSystemAAFontSettings=on',
    '--java-options', '-Dswing.aatext=true',
    # one garbage collector thread and a heap that gives unused memory back, for slower computers; no -Xmx on purpose
    '--java-options', '-XX:+UseSerialGC',
    '--java-options', '-XX:MinHeapFreeRatio=10',
    '--java-options', '-XX:MaxHeapFreeRatio=30',
    # a small starting heap: the serial collector never shrinks below where it started (1/64 of the memory)
    '--java-options', '-Xms64m',
    '--icon', (Join-Path $root 'src\main\resources\assets\image\icon.ico'),
    '--dest', $Out
)
if ($Type -ne 'app-image') {
    # the upgrade id must never change, it's how a newer installer recognises and replaces an older install
    $arguments += @(
        '--license-file', (Join-Path $root 'LICENSE'),
        '--about-url', 'https://github.com/riyadmondol2006/BlockForge',
        '--win-menu', '--win-menu-group', 'BlockForge',
        '--win-shortcut', '--win-dir-chooser', '--win-per-user-install',
        '--win-upgrade-uuid', 'bfd7c18e-448a-4abd-80c9-5df7629af779'
    )
}

try {
    if ($Type -eq 'app-image') {
        # jpackage refuses to write over an earlier app image
        Remove-Item -Recurse -Force -LiteralPath (Join-Path $Out 'BlockForge') -ErrorAction SilentlyContinue
    }
    & $jpackage @arguments
    if ($LASTEXITCODE -ne 0) { throw "jpackage failed with exit code $LASTEXITCODE" }

    if ($Type -ne 'app-image') {
        # jpackage names it BlockForge-<version>.exe; give it the name the releases and the updater use
        $built = Join-Path $Out "BlockForge-$version.$Type"
        if (-not (Test-Path -LiteralPath $built)) { throw "jpackage didn't make $built" }
        $final = "BlockForge-$version-windows-x64.$Type"
        Move-Item -LiteralPath $built -Destination (Join-Path $Out $final) -Force
        Write-Output "Built $(Join-Path $Out $final)"
    }
}
finally {
    Remove-Item -Recurse -Force -LiteralPath $stage -ErrorAction SilentlyContinue
}
