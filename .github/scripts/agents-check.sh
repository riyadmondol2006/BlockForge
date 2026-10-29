#!/usr/bin/env bash
# Checks the two game helpers (packaging/boost and packaging/lite) with the Java on the PATH, whose major version is
# the first argument: the self-tests of the committed jars, both started as agents of one Java from a folder with a
# space in its name, and (Java 9 and newer) the jars built from their sources passing the self-tests too.
set -euo pipefail
JAVA=${1:?usage: agents-check.sh <java major version>}
# a run of several Javas on one machine starts clean each time
rm -rf "agent test" built

java -version
java -cp packaging/lite/blockforge-lite.jar com.blockforge.lite.SelfTest
if jar tf packaging/boost/blockforge-boost.jar | grep -q "com/blockforge/boost/SelfTest.class"; then
  java -cp packaging/boost/blockforge-boost.jar com.blockforge.boost.SelfTest
fi

mkdir -p "agent test"
java -javaagent:packaging/boost/blockforge-boost.jar=preload="agent test/x.classes" \
  -javaagent:packaging/lite/blockforge-lite.jar=report="agent test/lite.txt" -version 2>&1 | tee agents.txt
if grep -q "problem\|Error occurred\|agent library failed" agents.txt; then
  echo "::error::an agent reported a problem with Java $JAVA"
  exit 1
fi

# javac 8 can't compile for an older release; the jars are built with a newer JDK and run on Java 8
if [ "$JAVA" = 8 ]; then
  exit 0
fi
# a failure says what went wrong in its annotation (the log of a finished job needs a sign-in to read)
step() {
  if ! out="$("$@" 2>&1)"; then
    echo "$out" | tail -40
    echo "::error::$* failed with Java $JAVA: $(echo "$out" | tail -12 | tr '\n' '|' | cut -c1-1500)"
    exit 1
  fi
  echo "$out" | tail -3
}
export OUT="$PWD/built"
step ./packaging/boost/build.sh
step ./packaging/lite/build.sh
for j in boost lite; do
  if cmp -s "built/blockforge-$j.jar" "packaging/$j/blockforge-$j.jar"; then
    echo "blockforge-$j.jar: the build is identical to the committed jar"
  else
    echo "::notice::blockforge-$j.jar built with Java $JAVA differs from the committed one"
  fi
done
