#!/usr/bin/env bash
# Builds the smoother-play agent jar: blockforge-lite.jar
#   ./build.sh                  writes ./blockforge-lite.jar (the file the launcher embeds) and runs the self-test
#   OUT=/some/dir ./build.sh    writes it there instead
#   NOTEST=1 ./build.sh         skips the self-test
# Needs a JDK 11 or newer that can still compile for Java 8 (javac --release 8; JDK 11 - 25 can). The jar runs on
# Java 8 and up.
#
# Layout of the jar:
#   com/blockforge/lite/*.class              the agent entry, its class loader and the self-test starter
#   META-INF/blockforge-lite/core.zip        the patch code and ASM (lib/), loaded only by the agent's own loader
#   META-INF/LICENSE-ASM.txt                 ASM's licence
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${OUT:-$here}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/agent" "$work/core" "$work/asm" "$work/jar/META-INF/blockforge-lite" "$out"

asm_jars=("$here"/lib/asm-9.10.1.jar "$here"/lib/asm-tree-9.10.1.jar "$here"/lib/asm-analysis-9.10.1.jar)
cp_sep=":"
asm_cp_jars=("${asm_jars[@]}")
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*)
        # Windows' javac wants Windows paths in a class path list, which the shell does not convert by itself
        cp_sep=";"
        if command -v cygpath > /dev/null 2>&1; then
            asm_cp_jars=()
            for j in "${asm_jars[@]}"; do asm_cp_jars+=("$(cygpath -m "$j")"); done
        fi
        ;;
esac
asm_cp="$(IFS="$cp_sep"; echo "${asm_cp_jars[*]}")"

javac_opts=(--release 8 -Xlint:-options -encoding UTF-8 -nowarn)
javac "${javac_opts[@]}" -d "$work/agent" $(find "$here/src/agent" -name '*.java')
# src/stubs: stand-ins of game classes that the patch templates compile against and the self-test runs on
javac "${javac_opts[@]}" -cp "$asm_cp" -d "$work/core" $(find "$here/src/core" "$here/src/stubs" -name '*.java')

# ASM's classes next to ours, without its manifest and module descriptor (our loader is not a module)
for j in "${asm_jars[@]}"; do
    (cd "$work/asm" && jar xf "$j")
done
rm -rf "$work/asm/META-INF" "$work/asm/module-info.class"
cp -R "$work/asm/." "$work/core/"

# fixed dates, so building the same sources gives the same bytes
date_opt=()
if jar --help 2>&1 | grep -q -- "--date"; then
    date_opt=(--date=2026-01-01T00:00:00Z)
fi
jar --create --no-manifest "${date_opt[@]}" --file "$work/jar/META-INF/blockforge-lite/core.zip" -C "$work/core" .
cp "$here/lib/LICENSE-ASM.txt" "$work/jar/META-INF/LICENSE-ASM.txt"
cp -R "$work/agent/." "$work/jar/"

cat > "$work/manifest.txt" <<MANIFEST
Premain-Class: com.blockforge.lite.Agent
Agent-Class: com.blockforge.lite.Agent
Implementation-Title: BlockForge Lite
MANIFEST

rm -f "$out/blockforge-lite.jar"
jar --create "${date_opt[@]}" --file "$out/blockforge-lite.jar" --manifest "$work/manifest.txt" -C "$work/jar" .
echo "Built $out/blockforge-lite.jar ($(wc -c < "$out/blockforge-lite.jar") bytes)"

if [ -z "${NOTEST:-}" ]; then
    java -cp "$out/blockforge-lite.jar" com.blockforge.lite.SelfTest
fi
