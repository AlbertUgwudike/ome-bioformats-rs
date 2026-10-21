#!/usr/bin/env bash
# Reads a file with Java Bio-Formats and prints its dimensions plus an MD5 per
# plane (RGB normalised to interleaved), so Rust output can be compared with
# the reference implementation.
#
#   BF_JAR=/path/to/bioformats_package.jar tools/bf-check/check.sh FILE [validate]
#
# Any jar containing Bio-Formats works, e.g. bioformats_package.jar from
# https://www.openmicroscopy.org/bio-formats/downloads/
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
jar="${BF_JAR:?set BF_JAR to a Bio-Formats jar}"
out="${TMPDIR:-/tmp}/bf-check-classes"
mkdir -p "$out"
if [[ ! -f "$out/Check.class" || "$here/Check.java" -nt "$out/Check.class" ]]; then
  javac -nowarn -cp "$jar" -d "$out" "$here/Check.java"
fi
java --enable-native-access=ALL-UNNAMED -cp "$jar:$out" Check "$@" 2> >(grep -v ClassNotFoundException >&2)
