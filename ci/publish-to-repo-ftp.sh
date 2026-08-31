#!/usr/bin/env bash
# Publish feature repository content via SFTP (preferred) or FTP/FTPS to OVH hosting.
#
# Protocol selection:
# - Prefer HECATE_REPO_FTP_PROTOCOL=sftp (default when port is 22).
# - CI should use SFTP; plain FTP is legacy-only.
# - For FTP, FTPS with certificate verification is used when TLS is available.
#   Plain FTP without TLS is insecure (credentials and data in cleartext).
set -euo pipefail

usage() {
  echo "Usage: $0 --feature-json PATH --artifact PATH --os OS --arch ARCH [--installer-type TYPE] [--kind agent|helper]"
}

feature_json=
artifact=
artifact_os=
artifact_arch=
installer_type=raw
kind=

while (($#)); do
  case "$1" in
    --feature-json) feature_json="$2"; shift 2 ;;
    --artifact) artifact="$2"; shift 2 ;;
    --os) artifact_os="$2"; shift 2 ;;
    --arch) artifact_arch="$2"; shift 2 ;;
    --installer-type) installer_type="$2"; shift 2 ;;
    --kind) kind="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

: "${feature_json:?--feature-json is required}"
: "${artifact:?--artifact is required}"
: "${artifact_os:?--os is required}"
: "${artifact_arch:?--arch is required}"

HECATE_REPO_FTP_HOST="${HECATE_REPO_FTP_HOST:?HECATE_REPO_FTP_HOST required}"
HECATE_REPO_FTP_USER="${HECATE_REPO_FTP_USER:?HECATE_REPO_FTP_USER required}"
HECATE_REPO_FTP_PASSWORD="${HECATE_REPO_FTP_PASSWORD:?HECATE_REPO_FTP_PASSWORD required}"
HECATE_REPO_FTP_REMOTE_DIR="${HECATE_REPO_FTP_REMOTE_DIR:-./repo/}"
HECATE_REPO_FTP_PORT="${HECATE_REPO_FTP_PORT:-22}"
HECATE_REPO_FTP_PROTOCOL="${HECATE_REPO_FTP_PROTOCOL:-}"
VERSION="${VERSION:-}"

# Fail closed in CI unless SFTP is selected (passwords must not travel over plain FTP).
if [[ "${CI:-}" == "true" || -n "${GITHUB_ACTIONS:-}" ]]; then
  if [[ -z "${HECATE_REPO_FTP_PROTOCOL}" ]]; then
    HECATE_REPO_FTP_PROTOCOL=sftp
  fi
  if [[ "${HECATE_REPO_FTP_PROTOCOL}" != "sftp" ]]; then
    echo "CI publish requires HECATE_REPO_FTP_PROTOCOL=sftp (got: ${HECATE_REPO_FTP_PROTOCOL})" >&2
    exit 1
  fi
fi

remote_dir="${HECATE_REPO_FTP_REMOTE_DIR#./}"
remote_dir="${remote_dir%/}"
[[ -n "${remote_dir}" ]] || remote_dir="repo"

if [[ -z "${HECATE_REPO_FTP_PROTOCOL}" ]]; then
  if [[ "${HECATE_REPO_FTP_PORT}" == "22" ]]; then
    HECATE_REPO_FTP_PROTOCOL=sftp
  else
    HECATE_REPO_FTP_PROTOCOL=ftp
  fi
fi

work_dir="${HECATE_REPO_WORK_DIR:-${RUNNER_TEMP:-/tmp}/hecate-feature-repo}"
binary="${HECATE_REPO_BIN:-hecate-repo}"

mkdir -p "$work_dir"

if ! command -v lftp >/dev/null 2>&1; then
  echo "lftp is required for repository publish" >&2
  exit 1
fi

# Feed lftp via stdin so credentials never appear on process argv / ps.
run_lftp() {
  if [[ -z "${HECATE_REPO_FTP_USER:-}" ]]; then
    echo "HECATE_REPO_FTP_USER is empty" >&2
    exit 1
  fi
  if [[ -z "${HECATE_REPO_FTP_PASSWORD:-}" ]]; then
    echo "HECATE_REPO_FTP_PASSWORD is empty" >&2
    exit 1
  fi
  # Embed credentials in the lftp script (stdin). LFTP_PASSWORD alone is unreliable
  # on some lftp builds in non-interactive CI ("GetPass() failed -- assume anonymous login").
  local user_enc pass_enc
  user_enc="$(python3 -c 'import os,urllib.parse; print(urllib.parse.quote(os.environ["HECATE_REPO_FTP_USER"], safe=""))')"
  pass_enc="$(python3 -c 'import os,urllib.parse; print(urllib.parse.quote(os.environ["HECATE_REPO_FTP_PASSWORD"], safe=""))')"
  {
    case "${HECATE_REPO_FTP_PROTOCOL}" in
      sftp)
        cat <<EOF
set net:timeout 30
set net:max-retries 2
set net:reconnect-interval-base 5
set cmd:fail-exit yes
set sftp:auto-confirm yes
open -p ${HECATE_REPO_FTP_PORT} sftp://${user_enc}:${pass_enc}@${HECATE_REPO_FTP_HOST}
EOF
        ;;
      ftp|ftps)
        cat <<EOF
set net:timeout 30
set net:max-retries 2
set net:reconnect-interval-base 5
set cmd:fail-exit yes
set ssl:verify-certificate yes
set ftp:ssl-force true
set ftp:ssl-protect-data true
open -p ${HECATE_REPO_FTP_PORT} ftp://${user_enc}:${pass_enc}@${HECATE_REPO_FTP_HOST}
EOF
        ;;
      *)
        echo "unsupported HECATE_REPO_FTP_PROTOCOL: ${HECATE_REPO_FTP_PROTOCOL} (use sftp or ftp)" >&2
        exit 1
        ;;
    esac
    cat
    echo "bye"
  } | lftp
}

ftp_sync_up() {
  local local_dir="$1"
  local remote_sub="$2"
  local delete_flag="${3:-}"
  if [[ ! -d "${local_dir}" ]]; then
    echo "skip missing local dir ${local_dir}"
    return 0
  fi
  if [[ "${delete_flag}" == "--delete" ]]; then
    run_lftp <<EOF
cd ${remote_dir}
mirror -R --delete --verbose ${local_dir} ${remote_sub}
EOF
  else
    run_lftp <<EOF
cd ${remote_dir}
mirror -R --verbose ${local_dir} ${remote_sub}
EOF
  fi
}

# Metadata-only pull: exclude installer blobs so CI stays fast, but keep every
# feature.json so multi-platform publishes merge and older versions stay indexed.
ftp_download_tree() {
  run_lftp <<EOF
cd ${remote_dir}
get -e repo.toml -o ${work_dir}/repo.toml
mirror --verbose dists ${work_dir}/dists
mirror --verbose \
  --exclude-glob '*.deb' \
  --exclude-glob '*.msi' \
  --exclude-glob '*.pkg' \
  --exclude-glob '*.sha256' \
  --exclude-glob '*.sig' \
  pool ${work_dir}/pool
EOF
}

ftp_put_file() {
  local local_file="$1"
  local remote_name="$2"
  run_lftp <<EOF
cd ${remote_dir}
put ${local_file} -o ${remote_name}
EOF
}

acquire_remote_lock() {
  # Default ~5 minutes. Cancelled CI jobs often leave .publish.lock behind.
  local tries="${HECATE_REPO_PUBLISH_LOCK_TRIES:-30}"
  local break_stale="${HECATE_REPO_PUBLISH_BREAK_STALE_LOCK:-1}"
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
      echo "SFTP/FTP authentication failed while acquiring publish lock" >&2
      return 1
    fi
    if [[ "${break_stale}" == "1" && "${i}" -eq 3 ]]; then
      echo "Publish lock still held; attempting to remove stale .publish.lock" >&2
      release_remote_lock || true
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

acquire_remote_lock
trap release_remote_lock EXIT

# Always refresh from remote under the lock so concurrent publishes do not clobber.
if ! ftp_download_tree; then
  echo "warning: metadata download reported errors; continuing if repo.toml is present" >&2
fi

if [[ ! -e "${work_dir}/repo.toml" ]]; then
  echo "Remote repository missing repo.toml; initializing local work dir" >&2
  "$binary" init "$work_dir"
fi

# Ensure signed feature.json files survived the metadata sync (needed to merge platforms
# and to keep older versions in the regenerated index).
feature_json_count="$(find "${work_dir}/pool" -type f -name feature.json 2>/dev/null | wc -l | tr -d ' ')"
echo "Loaded ${feature_json_count} feature.json file(s) from remote pool metadata"

command=add
case "$kind" in
  "") ;;
  agent) command=add-agent ;;
  helper) command=add-helper ;;
  *) echo "--kind must be agent or helper" >&2; exit 2 ;;
esac

manifest="${feature_json}"
if [[ -n "${VERSION}" ]]; then
  manifest="$(mktemp)"
  sed -E "s/(\"version\"[[:space:]]*:[[:space:]]*)\"[^\"]*\"/\1\"${VERSION}\"/" \
    "${feature_json}" > "${manifest}"
fi

"$binary" "$command" \
  --repo "$work_dir" \
  --feature-json "$manifest" \
  --artifact "$artifact" \
  --os "$artifact_os" \
  --arch "$artifact_arch" \
  --installer-type "$installer_type"
"$binary" prune --repo "$work_dir"

# Do not --delete pool: local work dir has metadata (+ the new artifact) only; wiping remote
# would remove other platforms' installers for existing versions.
ftp_sync_up "${work_dir}/pool" "pool"
ftp_sync_up "${work_dir}/dists" "dists" --delete
if [[ -f "${work_dir}/repo.toml" ]]; then
  ftp_put_file "${work_dir}/repo.toml" "repo.toml"
fi

echo "Published to ${HECATE_REPO_FTP_PROTOCOL}://${HECATE_REPO_FTP_HOST}:${HECATE_REPO_FTP_PORT}/${remote_dir}"
