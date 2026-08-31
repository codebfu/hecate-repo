#!/usr/bin/env bash
# Copyright (C) 2026 Gaultier HUBERT
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail

usage() {
  echo "Usage: $0 [--build] [--repo DIR]"
}

build=0
repo_dir="${HECATE_REPO_DIR:-./hecate-feature-repo}"

while (($#)); do
  case "$1" in
    --build)
      build=1
      shift
      ;;
    --repo)
      repo_dir="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

crate_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${HECATE_REPO_BIN:-$crate_dir/target/release/hecate-repo}"

if ((build)); then
  cargo build --release --manifest-path "$crate_dir/Cargo.toml"
fi

if [[ ! -x "$binary" ]]; then
  echo "hecate-repo binary not found at $binary; run with --build or set HECATE_REPO_BIN" >&2
  exit 1
fi

if [[ ! -e "$repo_dir/repo.toml" ]]; then
  "$binary" init "$repo_dir"
fi

echo "Repository public key:"
"$binary" pubkey

if [[ -n "${HECATE_REPO_RSYNC_TARGET:-}" ]]; then
  if [[ "$HECATE_REPO_RSYNC_TARGET" != *:* ]]; then
    echo "HECATE_REPO_RSYNC_TARGET must use host:path syntax" >&2
    exit 1
  fi
  remote_host="${HECATE_REPO_RSYNC_TARGET%%:*}"
  remote_path="${HECATE_REPO_RSYNC_TARGET#*:}"
  if ! ssh "$remote_host" test ! -e "$remote_path/dists/stable/Release"; then
    echo "Remote Release exists or could not be checked; refusing to overwrite it" >&2
    exit 1
  fi
  rsync -a --delete "$repo_dir/" "$HECATE_REPO_RSYNC_TARGET/"
fi
