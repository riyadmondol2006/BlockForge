#!/bin/sh
# Checks the Bedrock Edition helpers of an installed package or an unpacked AppImage:
#
#   packaging/linux/check-helpers.sh [root]     root is / (the default) or the folder holding usr/bin
#
# Both must sit next to blockforge and be executable. bedrock-runtime must be static (it starts inside the Steam Linux
# Runtime container, which has none of our libraries). blockforge-bedrock-auth must find every library it links on this
# computer; where WebKitGTK is missing that is the failure players would see, so it is a failure here too. Nothing is
# started, so no display is needed.
set -eu

root="${1:-}"
root="${root%/}"
bin="$root/usr/bin"
bad=0
fail() { echo "check-helpers.sh: $*" >&2; bad=1; }

for f in blockforge blockforge-bedrock-auth bedrock-runtime; do
    [ -x "$bin/$f" ] || fail "$bin/$f is missing or not executable"
done
[ "$bad" = 0 ] || exit 1

if readelf -lW "$bin/bedrock-runtime" | grep -q INTERP || readelf -d "$bin/bedrock-runtime" 2> /dev/null | grep -q NEEDED; then
    fail "bedrock-runtime is not a static program"
fi
missing="$(ldd "$bin/blockforge-bedrock-auth" 2>&1 | grep 'not found' || true)"
[ -z "$missing" ] || fail "blockforge-bedrock-auth misses libraries: $missing"
# only the sign-in helper may need WebKit or GTK; blockforge itself must start without them
if readelf -d "$bin/blockforge" 2> /dev/null | grep NEEDED | grep -qi 'webkit\|gtk\|soup\|javascriptcore'; then
    fail "blockforge itself links WebKit or GTK; only blockforge-bedrock-auth may"
fi
[ "$bad" = 0 ] && echo "The Bedrock helpers in $bin are in place."
exit "$bad"
