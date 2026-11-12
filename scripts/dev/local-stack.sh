#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Discover PostgreSQL's versioned bin directory on Debian as well as macOS.
if command -v pg_config >/dev/null; then
  export PATH="$(pg_config --bindir):${PATH}"
fi
if [[ -d /opt/sentinel-demo-tools ]]; then export PATH="/opt/sentinel-demo-tools:${PATH}"; fi
# PostgreSQL refuses root by design. Use the repository's existing owner,
# without creating users, moving files, or initializing a root-owned cluster.
if [[ "$(id -u)" == 0 ]]; then
  repo_owner="${SENTINEL_DEV_USER:-$(stat -c '%U' "${repo_root}")}"
  if [[ "${repo_owner}" == root ]]; then
    echo 'Run make run as an unprivileged repository owner, or set SENTINEL_DEV_USER.' >&2
    exit 1
  fi
  owner_home="$(getent passwd "${repo_owner}" | cut -d: -f6)"
  task_state_dir="${STATE_DIR:-${repo_root}/.local/server-run}"
  if [[ ! -e "${task_state_dir}" ]]; then
    install -d -m 700 -o "${repo_owner}" -g "$(id -gn "${repo_owner}")" "${task_state_dir}"
  fi
  exec runuser -u "${repo_owner}" -- env PATH="${owner_home}/.cargo/bin:${PATH}" \
    SENTINEL_SOURCE_MODE="${SENTINEL_SOURCE_MODE:-none}" STATE_DIR="${task_state_dir}" \
    bash "${repo_root}/scripts/dev/local-webcam-stack.sh"
fi
export SENTINEL_SOURCE_MODE="${SENTINEL_SOURCE_MODE:-none}"
export STATE_DIR="${STATE_DIR:-${repo_root}/.local/server-run}"
exec bash "${repo_root}/scripts/dev/local-webcam-stack.sh"
