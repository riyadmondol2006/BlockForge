# Built by packaging/linux/build.sh, which fills in the staged files (_stage) and the version (bf_version, and
# bf_date for the changelog). It packs a program that is already built, so there is nothing to compile.
%global debug_package %{nil}
%global _build_id_links none
%global __os_install_post %{nil}
%global _binary_payload w19.xzdio

Name:           blockforge
Version:        %{bf_version}
Release:        1%{?dist}
Summary:        A fast, modern Minecraft launcher
License:        GPL-3.0-or-later
URL:            https://github.com/riyadmondol2006/BlockForge
ExclusiveArch:  x86_64

# the Java edition (3.4.x) was this package too, under /opt/blockforge
Obsoletes:      blockforge < 4.0.0

# libraries the program opens at run time (graphics, windowing, the file chooser), which rpm cannot see in the file
Requires:       libEGL.so.1()(64bit)
Requires:       libGL.so.1()(64bit)
Requires:       libdbus-1.so.3()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libX11.so.6()(64bit)
Requires:       libX11-xcb.so.1()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libXrender.so.1()(64bit)
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       hicolor-icon-theme
Recommends:     xdg-desktop-portal
Recommends:     xdg-utils
Recommends:     shared-mime-info
Recommends:     desktop-file-utils
Recommends:     libXrandr.so.2()(64bit)
Recommends:     libXxf86vm.so.1()(64bit)
Recommends:     libasound.so.2()(64bit)
Recommends:     libpulse.so.0()(64bit)
Suggests:       gamemode

%description
BlockForge installs and starts Minecraft: Java Edition. Every game lives in its own instance, with its own mods,
settings and Java. It installs mods and modpacks from Modrinth, CurseForge, FTB, ATLauncher and Technic, imports
instances from other launchers, supports Microsoft accounts and offline play, and starts quickly on modest computers.

This is the Rust edition. It replaces the Java edition and keeps its instances, accounts and settings in
~/.local/share/blockforge.

%prep

%build

%install
cp -a %{_stage}/. %{buildroot}/

%post
update-desktop-database -q %{_datadir}/applications > /dev/null 2>&1 || :
update-mime-database %{_datadir}/mime > /dev/null 2>&1 || :
gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor > /dev/null 2>&1 || :

%postun
update-desktop-database -q %{_datadir}/applications > /dev/null 2>&1 || :
update-mime-database %{_datadir}/mime > /dev/null 2>&1 || :
gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor > /dev/null 2>&1 || :

# Runs once the old package is gone completely: removes what the Java edition left in /opt/blockforge (things it
# wrote at run time, which no package owned) and the menu entry it added by itself.
%posttrans
rm -f %{_datadir}/applications/blockforge-BlockForge.desktop || :
if [ -d /opt/blockforge/bin ] || [ -d /opt/blockforge/lib ]; then
    rm -rf /opt/blockforge || :
fi

%files
%defattr(-,root,root,-)
%license %{_datadir}/licenses/blockforge/LICENSE
%{_bindir}/blockforge
%{_datadir}/applications/blockforge.desktop
%{_datadir}/icons/hicolor/*/apps/blockforge.*
%{_datadir}/metainfo/io.github.riyadmondol2006.BlockForge.metainfo.xml
%{_datadir}/mime/packages/blockforge.xml
%{_mandir}/man1/blockforge.1*

%changelog
* %{bf_date} BlockForge contributors <riyadmondol2006@gmail.com> - %{bf_version}-1
- The Rust edition of BlockForge. It replaces the Java edition.
