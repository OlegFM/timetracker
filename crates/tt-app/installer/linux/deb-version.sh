#!/usr/bin/env bash
# Print the Debian package version for a release version:
# 0.2.0-rc.1 -> 0.2.0~rc.1 (sorts before 0.2.0), 0.2.0 -> 0.2.0.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <version>" >&2
  exit 2
fi

printf '%s\n' "${1/-rc./~rc.}"
