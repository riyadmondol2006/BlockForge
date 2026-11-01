#!/bin/sh
# Bedrock Edition's build needs (protoc for the vendored Xodus crates, GTK and WebKitGTK for the sign-in helper, the static
# runtime helper) are for Linux only. This fails when a Windows or macOS build would pull one of them in, which would break
# those builds on machines that have none of the tools. Needs cargo and the lock file only, not the targets.
#
#   .github/scripts/linux-only-deps.sh
set -eu
cd "$(dirname "$0")/../.."

bad=0
for target in x86_64-pc-windows-msvc aarch64-apple-darwin; do
    found="$(cargo tree --locked --target "$target" --edges normal,build,dev --prefix none 2> /dev/null \
        | awk '{print $1}' | sort -u \
        | grep -E '^(xodus|msixvc|msixvc-common|bedrock-runtime|blockforge-bedrock-auth|prost-build|protobuf|wry|tao|webkit2gtk|webkit2gtk-sys|javascriptcore-rs|soup3|gtk|gtk-sys)$' || true)"
    if [ -n "$found" ]; then
        echo "::error title=Linux-only dependencies leak into $target::$(echo "$found" | tr '\n' ' ')"
        bad=1
    else
        echo "$target: no Linux-only Bedrock dependencies"
    fi
done
exit "$bad"
