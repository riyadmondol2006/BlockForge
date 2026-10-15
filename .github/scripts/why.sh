#!/usr/bin/env bash
# Shows the end of a log as one error annotation on the run page, so a failed step can be understood without
# opening the full log (the page of a run lists annotations to everyone).
#   .github/scripts/why.sh <log file> <title>
set -u
log="$1"
title="$2"
body="$(tail -n 40 "$log" 2>/dev/null | tr -d '\r' | sed 's/%/%25/g' | awk 'BEGIN { ORS = "%0A" } { print }')"
echo "::error title=${title} failed::${body}"
