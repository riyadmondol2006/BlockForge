# Builds the Windows installer (an .exe that installs for the current user, no administrator needed). It carries its
# own Java, so people don't need to install Java first.
#
#   scripts\package.ps1 [-Jar path\to\BlockForge-x.y.z.jar] [-Out build\installers]
#
# Needs a JDK 17 or newer with jpackage, and the WiX Toolset 3 (https://wixtoolset.org).
param(
    [string]$Jar = (Get-ChildItem "$PSScriptRoot\..\build\libs\BlockForge-*.jar" | Select-Object -First 1).FullName,
    [string]$Out = "$PSScriptRoot\..\build\installers"
)
$ErrorActionPreference = "Stop"

if (-not (Test-Path $Jar)) { throw "jar not found: $Jar (build it with gradlew shadowJar)" }

$root = (Resolve-Path "$PSScriptRoot\..").Path
$version = [IO.Path]::GetFileNameWithoutExtension($Jar) -replace '^BlockForge-', '' -replace '\.Beta$', ''
# Windows installers only allow three numbers: 3.4.41.4 -> 3.4.41
$short = ($version.Split('.') | Select-Object -First 3) -join '.'

$modules = "java.se,jdk.unsupported,jdk.crypto.ec,jdk.crypto.cryptoki,jdk.localedata,jdk.zipfs,jdk.charsets,jdk.management,jdk.management.agent,jdk.net,jdk.httpserver,jdk.jfr,jdk.xml.dom,jdk.naming.dns,jdk.security.auth,jdk.security.jgss"

$stage = Join-Path ([IO.Path]::GetTempPath()) ("blockforge-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item $Jar $stage
New-Item -ItemType Directory -Force -Path $Out | Out-Null

try {
    # the upgrade id must never change, it's how a newer installer recognises and replaces an older install
    & jpackage --type exe --name BlockForge --app-version $short --vendor BlockForge `
        --description "A Minecraft launcher with a modern interface that plays offline without an account." `
        --copyright "Copyright ATLauncher and BlockForge contributors, GPL-3.0" `
        --license-file "$root\LICENSE" --about-url "https://github.com/riyadmondol2006/BlockForge" `
        --input $stage --main-jar (Split-Path $Jar -Leaf) --main-class com.blockforge.launcher.App `
        --add-modules $modules --jlink-options "--strip-debug --no-header-files --no-man-pages" `
        --java-options -Dblockforge.installed=true --java-options -Dawt.useSystemAAFontSettings=on `
        --java-options -Dswing.aatext=true `
        --icon "$root\src\main\resources\assets\image\icon.ico" `
        --win-menu --win-menu-group BlockForge --win-shortcut --win-dir-chooser --win-per-user-install `
        --win-upgrade-uuid bfd7c18e-448a-4abd-80c9-5df7629af779 `
        --dest $Out
    if ($LASTEXITCODE -ne 0) { throw "jpackage failed with exit code $LASTEXITCODE" }

    # name it like the other downloads, with the full version
    $built = Get-ChildItem "$Out\BlockForge-*.exe" | Select-Object -First 1
    Rename-Item $built.FullName "BlockForge-$version-windows-setup.exe" -Force
}
finally {
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
}
