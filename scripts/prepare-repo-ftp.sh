#!/usr/bin/env bash
# Copyright (C) 2026 Gaultier HUBERT
# SPDX-License-Identifier: GPL-3.0-or-later
# Initialize empty signed feature repo and upload via SFTP (preferred) or FTPS.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="${HECATE_REPO_DIR:-./repository}"
binary="${HECATE_REPO_BIN:-hecate-repo}"

if [[ ! -x "$binary" ]] && [[ -x "${SCRIPT_DIR}/../../hecate-repo/target/release/hecate-repo" ]]; then
  binary="${SCRIPT_DIR}/../../hecate-repo/target/release/hecate-repo"
fi

if [[ ! -e "${repo_dir}/repo.toml" ]]; then
  "$binary" init "$repo_dir"
fi

echo "Repository public key:"
"$binary" pubkey

if [[ -z "${HECATE_REPO_FTP_HOST:-}" ]]; then
  echo "Set HECATE_REPO_FTP_* env vars to upload" >&2
  exit 0
fi

: "${HECATE_REPO_FTP_USER:?HECATE_REPO_FTP_USER required}"
: "${HECATE_REPO_FTP_PASSWORD:?HECATE_REPO_FTP_PASSWORD required}"
HECATE_REPO_FTP_REMOTE_DIR="${HECATE_REPO_FTP_REMOTE_DIR:-./repo/}"
HECATE_REPO_FTP_PORT="${HECATE_REPO_FTP_PORT:-22}"
HECATE_REPO_FTP_PROTOCOL="${HECATE_REPO_FTP_PROTOCOL:-}"

if [[ -z "${HECATE_REPO_FTP_PROTOCOL}" ]]; then
  if [[ "${HECATE_REPO_FTP_PORT}" == "22" ]]; then
    HECATE_REPO_FTP_PROTOCOL=sftp
  else
    HECATE_REPO_FTP_PROTOCOL=ftp
  fi
fi

if [[ "${CI:-}" == "true" || -n "${GITHUB_ACTIONS:-}" ]]; then
  if [[ "${HECATE_REPO_FTP_PROTOCOL}" != "sftp" ]]; then
    echo "CI publish requires HECATE_REPO_FTP_PROTOCOL=sftp" >&2
    exit 1
  fi
fi

if ! command -v lftp >/dev/null 2>&1; then
  echo "lftp not found; repository initialized locally only" >&2
  exit 0
fi

remote_dir="${HECATE_REPO_FTP_REMOTE_DIR#./}"
remote_dir="${remote_dir%/}"
[[ -n "${remote_dir}" ]] || remote_dir="repo"

lftp_open_block() {
  case "${HECATE_REPO_FTP_PROTOCOL}" in
    sftp)
      cat <<EOF
set net:max-retries 2
set sftp:auto-confirm yes
open -u ${HECATE_REPO_FTP_USER} -p ${HECATE_REPO_FTP_PORT} sftp://${HECATE_REPO_FTP_HOST}
EOF
      ;;
    ftp|ftps)
      cat <<EOF
set net:max-retries 2
set ssl:verify-certificate yes
set ftp:ssl-force true
set ftp:ssl-protect-data true
open -u ${HECATE_REPO_FTP_USER} -p ${HECATE_REPO_FTP_PORT} ftp://${HECATE_REPO_FTP_HOST}
EOF
      ;;
    *)
      echo "unsupported HECATE_REPO_FTP_PROTOCOL: ${HECATE_REPO_FTP_PROTOCOL}" >&2
      exit 1
      ;;
  esac
}

export LFTP_PASSWORD="${HECATE_REPO_FTP_PASSWORD}"
{
  lftp_open_block
  cat <<EOF
cd ${remote_dir}
mirror -R --verbose ${repo_dir}/ .
bye
EOF
} | lftp

echo "Initial repository uploaded"
