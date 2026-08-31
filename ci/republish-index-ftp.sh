#!/usr/bin/env bash
# Copyright (C) 2026 Gaultier HUBERT
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Download the remote feature repo metadata, regenerate dists/stable (features.json + Release)
# with the current hecate-repo binary, and upload dists/ only.
# Intended to run after every hecate-repo CLI release so index format updates ship to prod.
set -euo pipefail

HECATE_REPO_FTP_HOST="${HECATE_REPO_FTP_HOST:?HECATE_REPO_FTP_HOST required}"
HECATE_REPO_FTP_USER="${HECATE_REPO_FTP_USER:?HECATE_REPO_FTP_USER required}"
HECATE_REPO_FTP_PASSWORD="${HECATE_REPO_FTP_PASSWORD:?HECATE_REPO_FTP_PASSWORD required}"
HECATE_REPO_FTP_REMOTE_DIR="${HECATE_REPO_FTP_REMOTE_DIR:-./repo/}"
HECATE_REPO_FTP_PORT="${HECATE_REPO_FTP_PORT:-22}"
HECATE_REPO_FTP_PROTOCOL="${HECATE_REPO_FTP_PROTOCOL:-sftp}"

work_dir="${HECATE_REPO_WORK_DIR:-${RUNNER_TEMP:-/tmp}/hecate-feature-repo-reindex}"
binary="${HECATE_REPO_BIN:-hecate-repo}"

remote_dir="${HECATE_REPO_FTP_REMOTE_DIR#./}"
remote_dir="${remote_dir%/}"
[[ -n "${remote_dir}" ]] || remote_dir="repo"

if [[ "${CI:-}" == "true" || -n "${GITHUB_ACTIONS:-}" ]]; then
  if [[ "${HECATE_REPO_FTP_PROTOCOL}" != "sftp" ]]; then
    echo "CI reindex requires HECATE_REPO_FTP_PROTOCOL=sftp" >&2
    exit 1
  fi
fi

if ! command -v lftp >/dev/null 2>&1; then
  echo "lftp is required" >&2
  exit 1
fi

if [[ ! -x "${binary}" ]]; then
  if command -v "${binary}" >/dev/null 2>&1; then
    :
  else
    echo "hecate-repo binary not found or not executable: ${binary}" >&2
    exit 1
  fi
fi

run_lftp() {
  if [[ -z "${HECATE_REPO_FTP_USER:-}" || -z "${HECATE_REPO_FTP_PASSWORD:-}" ]]; then
    echo "HECATE_REPO_FTP_USER/PASSWORD must be set" >&2
    exit 1
  fi
  local user_enc pass_enc
  user_enc="$(python3 -c 'import os,urllib.parse; print(urllib.parse.quote(os.environ["HECATE_REPO_FTP_USER"], safe=""))')"
  pass_enc="$(python3 -c 'import os,urllib.parse; print(urllib.parse.quote(os.environ["HECATE_REPO_FTP_PASSWORD"], safe=""))')"
  {
    cat <<EOF
set net:timeout 30
set net:max-retries 2
set net:reconnect-interval-base 5
set cmd:fail-exit yes
set sftp:auto-confirm yes
open -p ${HECATE_REPO_FTP_PORT} sftp://${user_enc}:${pass_enc}@${HECATE_REPO_FTP_HOST}
EOF
    cat
    echo "bye"
  } | lftp
}

acquire_remote_lock() {
  local tries="${HECATE_REPO_PUBLISH_LOCK_TRIES:-18}"
  local i
  local out
  for i in $(seq 1 "${tries}"); do
    if out="$(run_lftp <<EOF 2>&1
cd ${remote_dir}
mkdir .publish.lock
EOF
)"; then
      echo "Acquired remote publish lock"
      return 0
    fi
    if echo "${out}" | grep -qiE 'login failed|password required|authentication|permission denied'; then
      echo "${out}" >&2
      echo "SFTP authentication failed while acquiring publish lock" >&2
      return 1
    fi
    echo "Waiting for remote publish lock (${i}/${tries})..."
    sleep 10
  done
  echo "Failed to acquire remote publish lock" >&2
  return 1
}

release_remote_lock() {
  run_lftp <<EOF || true
cd ${remote_dir}
rmdir .publish.lock
EOF
  echo "Released remote publish lock"
}

mkdir -p "$work_dir"

acquire_remote_lock
trap release_remote_lock EXIT

# Metadata-only sync: regenerate_index reads pool/*/feature.json, not installer blobs.
run_lftp <<EOF
cd ${remote_dir}
get -e repo.toml -o ${work_dir}/repo.toml
mirror --verbose dists ${work_dir}/dists
mirror --verbose \
  --include-glob '*/' \
  --include-glob 'feature.json' \
  --include-glob 'feature.json.sig' \
  --exclude-glob '*' \
  pool ${work_dir}/pool
EOF

while IFS= read -r -d '' version_dir; do
  if [[ ! -f "${version_dir}/feature.json" ]]; then
    echo "Removing orphan pool version without feature.json: ${version_dir}"
    rm -rf "${version_dir}"
  fi
done < <(find "${work_dir}/pool" -mindepth 2 -maxdepth 2 -type d -print0 2>/dev/null || true)

if [[ ! -e "${work_dir}/repo.toml" ]]; then
  echo "Remote repository missing repo.toml under ${remote_dir}; nothing to reindex" >&2
  exit 1
fi

"$binary" reindex --repo "$work_dir"

index_path="${work_dir}/dists/stable/features.json"
if [[ ! -f "${index_path}" ]]; then
  echo "reindex did not produce ${index_path}" >&2
  exit 1
fi
if ! grep -q '"generated_at"' "${index_path}"; then
  echo "features.json missing generated_at after reindex; refusing to upload" >&2
  exit 1
fi

run_lftp <<EOF
cd ${remote_dir}
mirror -R --delete --verbose ${work_dir}/dists dists
EOF

echo "Republished dists/ to ${HECATE_REPO_FTP_PROTOCOL}://${HECATE_REPO_FTP_HOST}:${HECATE_REPO_FTP_PORT}/${remote_dir}"
