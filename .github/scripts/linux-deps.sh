#!/usr/bin/env bash
# Build dependencies for the Linux CI jobs (ubuntu-22.04 runners).
#
# The runner's apt goes through /etc/apt/apt-mirrors.txt, whose first entry is the
# Azure mirror. When that host does not resolve, apt fails instead of falling back,
# so the Azure line is dropped and the main Ubuntu archive is used.
set -euo pipefail

mirrors=/etc/apt/apt-mirrors.txt
if [ -f "$mirrors" ]; then
  sudo sed -i '/azure\.archive\.ubuntu\.com/d' "$mirrors"
fi
apt_opts=(-o Acquire::Retries=3)
sudo apt-get "${apt_opts[@]}" update
sudo apt-get "${apt_opts[@]}" install -y --no-install-recommends \
  libfontconfig1-dev libxkbcommon-dev
