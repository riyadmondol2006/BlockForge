# Layout of the BlockForge disk image, read by dmgbuild (https://dmgbuild.readthedocs.io). dmgbuild writes the Finder
# window settings itself, so it works on a computer without a screen, which is why build.sh uses it instead of
# scripting Finder. build.sh passes the paths with -D app=... -D background=... [-D volicon=...].
import os.path

app = defines["app"]  # noqa: F821 (defines is provided by dmgbuild)
app_name = os.path.basename(app)

format = "UDZO"
compression_level = 9
filesystem = "HFS+"

files = [app]
symlinks = {"Applications": "/Applications"}

# dmg-background@2x.png next to this file is picked up for Retina screens
background = defines["background"]  # noqa: F821
if "volicon" in defines:  # noqa: F821
    icon = defines["volicon"]  # noqa: F821

# the window's content is 660 x 400 like the background; the extra 20 points are for the title bar
window_rect = ((200, 140), (660, 420))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
arrange_by = None
icon_size = 128
text_size = 13
label_pos = "bottom"
icon_locations = {
    app_name: (170, 190),
    "Applications": (490, 190),
}
