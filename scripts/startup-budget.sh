#!/bin/sh
# Fails if BlockForge takes longer than the budget to show its window. Run after building the jar.
#   scripts/startup-budget.sh [jar] [budget-ms]
JAR="${1:-$(ls build/libs/BlockForge-*.jar | head -n 1)}"
exec java -cp "$JAR" scripts/StartupBudget.java "${2:-4000}"
