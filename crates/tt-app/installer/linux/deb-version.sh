#!/usr/bin/env bash
# Print the Debian package version for a release version:
# Any pre-release: 0.2.0-rc.1 -> 0.2.0~rc.1, 0.2.0-beta.2 -> 0.2.0~beta.2 (both sort before
# 0.2.0); 0.2.0 -> 0.2.0.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <version>" >&2
  exit 2
fi

# A literal `~` in the replacement would be tilde-expanded, hence the variable.
tilde='~'
printf '%s\n' "${1/-/$tilde}"
