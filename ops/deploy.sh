#!/usr/bin/env bash
# perihel-v2 deploy — one-command deploy of the sync server to the GCP VM.
#
# Copies server/sync-server/*.py and the systemd units to the VM, restarts
# perihel-sync (and perihel-telegram if it is enabled), smoke-tests
# GET /v1/health, and reports the AI queue depth.
#
# Usage:
#   ops/deploy.sh [--instance NAME] [--zone ZONE] [--project PROJECT]
#                 [--dry-run] [--allow-dirty]
#
# Defaults: instance perihel-sync, zone us-west1-b (see SELF-HOST.md §2).
# Project defaults to the gcloud active project; override with --project
# or PERIHEL_GCP_PROJECT. PERIHEL_INSTANCE / PERIHEL_ZONE also work.
#
# Safety:
# - Refuses to run with uncommitted changes to the files it deploys
#   (server/sync-server/*.py, server/systemd/*), so a deploy always
#   matches a committed tree. --allow-dirty overrides (prints a warning).
# - Copies only code and units. It never copies, reads, or prints a
#   secret file (token, nim.key, nim.*.key, telegram.key, ntfy-topic).
#   The health check reads the token ON the VM under sudo; the token
#   never appears in this script's output.
# - The ntfy units are left alone: perihel-ntfy.service still points at
#   the deleted ntfy_push.py, so nothing on the VM runs tick.py today.
#   That is issue #10's scope — this script does not touch it.
# - Requires passwordless sudo on the VM (the GCP default) for the
#   /opt/perihel-sync reads and the restarts.
#
# --dry-run prints every command it would run without running any of
# them. The local guards (clean tree, py_compile) still run in dry-run
# mode, so a dry run also tells you whether the tree is deployable.
set -euo pipefail

INSTANCE="${PERIHEL_INSTANCE:-perihel-sync}"
ZONE="${PERIHEL_ZONE:-us-west1-b}"
PROJECT="${PERIHEL_GCP_PROJECT:-}"
DRY_RUN=0
ALLOW_DIRTY=0

usage() {
    cat <<'EOF'
Usage: ops/deploy.sh [--instance NAME] [--zone ZONE] [--project PROJECT]
                     [--dry-run] [--allow-dirty]

Deploy the sync server (server/sync-server/*.py + server/systemd/*)
to the GCP VM, restart perihel-sync (and perihel-telegram if enabled),
smoke-test GET /v1/health, and report the AI queue depth.

  --instance NAME   VM name (default: perihel-sync, or $PERIHEL_INSTANCE)
  --zone ZONE       GCP zone (default: us-west1-b, or $PERIHEL_ZONE)
  --project PROJECT GCP project (default: gcloud active project,
                    or $PERIHEL_GCP_PROJECT)
  --dry-run         Print every command without running any of them.
  --allow-dirty     Deploy even with uncommitted changes to the files
                    being deployed (prints a warning; not recommended).
EOF
}

die() { echo "deploy: error: $*" >&2; exit 1; }
say() { echo "== $*"; }

# run: execute a command, or print it in dry-run mode.
run() {
    if [ "$DRY_RUN" = 1 ]; then
        printf '[dry-run]'
        printf ' %q' "$@"
        printf '\n'
    else
        "$@"
    fi
}

# run_ssh: run one command on the VM via gcloud ssh, or print it in
# dry-run mode. The remote command is shown verbatim so the dry run is
# readable; it is passed to ssh as a single argument.
run_ssh() {
    local remote="$1"
    if [ "$DRY_RUN" = 1 ]; then
        printf '[dry-run] gcloud --quiet'
        [ -n "$PROJECT" ] && printf ' --project %s' "$PROJECT"
        printf ' compute ssh --zone %s %s --\n' "$ZONE" "$INSTANCE"
        printf '%s\n' "$remote"
    else
        if [ -n "$PROJECT" ]; then
            gcloud --quiet --project "$PROJECT" compute ssh \
                --zone "$ZONE" "$INSTANCE" -- "$remote"
        else
            gcloud --quiet compute ssh \
                --zone "$ZONE" "$INSTANCE" -- "$remote"
        fi
    fi
}

while [ $# -gt 0 ]; do
    case "$1" in
        --instance) INSTANCE="$2"; shift 2 ;;
        --zone) ZONE="$2"; shift 2 ;;
        --project) PROJECT="$2"; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        --allow-dirty) ALLOW_DIRTY=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
done

# --- local guards -------------------------------------------------------
# The repo is the checkout this script lives in, not the caller's cwd,
# so the script works from any directory.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

[ "$(git rev-parse --show-toplevel 2>/dev/null)" = "$REPO_ROOT" ] \
    || die "not inside a git checkout — run from the perihel-v2 repo"

# The deploy must match a committed tree: refuse on uncommitted changes
# to exactly the files being deployed (code + units). Untracked files
# that are not deployed (e.g. __pycache__/) do not match these pathspecs
# and do not block a deploy.
if [ "$ALLOW_DIRTY" = 0 ]; then
    dirty="$(git status --porcelain -- 'server/sync-server/*.py' 'server/systemd' || true)"
    if [ -n "$dirty" ]; then
        echo "deploy: refusing — uncommitted changes to files being deployed:" >&2
        echo "$dirty" >&2
        echo "deploy: commit them first, or re-run with --allow-dirty." >&2
        exit 1
    fi
else
    echo "deploy: WARNING: --allow-dirty — deploying an uncommitted tree." >&2
fi

# Never ship a syntax error to the VM: compile everything first.
# This is a local check, so it runs for real even in --dry-run mode.
say "py_compile server/sync-server/*.py"
[ "$DRY_RUN" = 1 ] && echo "[dry-run] (local check, runs now) python3 -m py_compile server/sync-server/*.py"
python3 -m py_compile server/sync-server/*.py

py_files=(server/sync-server/*.py)
unit_files=(server/systemd/*.service server/systemd/*.timer)
[ -f "${py_files[0]}" ] || die "no server/sync-server/*.py files found"
[ -f "${unit_files[0]}" ] || die "no server/systemd unit files found"

if [ "$DRY_RUN" = 0 ]; then
    command -v gcloud >/dev/null 2>&1 || die "gcloud not found on PATH"
fi

# --- copy ---------------------------------------------------------------
# Staging dir on the VM first (scp fails if the target dir is missing),
# then the code + units, then one ssh call installs and restarts them.

say "staging files on $INSTANCE ($ZONE)"
run_ssh 'mkdir -p ~/perihel-deploy'
if [ -n "$PROJECT" ]; then
    run gcloud --quiet --project "$PROJECT" compute scp --zone "$ZONE" \
        "${py_files[@]}" "${unit_files[@]}" "$INSTANCE":~/perihel-deploy/
else
    run gcloud --quiet compute scp --zone "$ZONE" \
        "${py_files[@]}" "${unit_files[@]}" "$INSTANCE":~/perihel-deploy/
fi

say "installing + restarting on the VM"
REMOTE_INSTALL='set -e
sudo cp ~/perihel-deploy/*.py /opt/perihel-sync/
sudo cp ~/perihel-deploy/*.service ~/perihel-deploy/*.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl restart perihel-sync
if sudo systemctl is-enabled --quiet perihel-telegram; then sudo systemctl restart perihel-telegram; echo "restarted perihel-telegram"; else echo "perihel-telegram not enabled - skipping restart (see SELF-HOST.md section 6)"; fi
echo "ntfy units untouched - tick.py wiring is issue #10"'
run_ssh "$REMOTE_INSTALL"

# --- health check -------------------------------------------------------
# The server listens plain HTTP on 127.0.0.1:8443 (PERIHEL_SYNC_NO_TLS=1
# in perihel-sync.service). The token is read ON the VM under sudo, so it
# never appears in this script's output. Retried up to 6 times, 5 s apart,
# because the server takes a moment to start (it imports the AI module).

REMOTE_HEALTH="python3 -c \"import json,urllib.request; t=open('/opt/perihel-sync/token').read().strip(); req=urllib.request.Request('http://127.0.0.1:8443/v1/health', headers={'Authorization': 'Bearer '+t}); d=json.loads(urllib.request.urlopen(req, timeout=10).read()); print('health ok:', d)\""
REMOTE_HEALTH="sudo $REMOTE_HEALTH"

say "health check GET /v1/health (up to 6 attempts, 5 s apart)"
if [ "$DRY_RUN" = 1 ]; then
    echo "# retried up to 6 times, 5 s apart, until it succeeds"
    run_ssh "$REMOTE_HEALTH"
else
    health_ok=0
    for attempt in 1 2 3 4 5 6; do
        echo "attempt $attempt/6..."
        if run_ssh "$REMOTE_HEALTH"; then health_ok=1; break; fi
        sleep 5
    done
    [ "$health_ok" = 1 ] || die "health check failed after 6 attempts — check 'sudo systemctl status perihel-sync' on the VM"
fi

# --- AI queue depth -----------------------------------------------------
# Count captures waiting for AI, straight from the VM's SQLite DB.

# The SQL uses a bound parameter, not a literal: SQLite treats a
# double-quoted "queued" as a column name, and a single-quoted 'queued'
# cannot appear in this single-quoted assignment. The parameter keeps
# the quoting to plain \" throughout.
REMOTE_QUEUE='sudo python3 -c "import sqlite3; c=sqlite3.connect(\"/opt/perihel-sync/sync.db\"); n=c.execute(\"SELECT COUNT(*) FROM captures WHERE ai_state = ?\", (\"queued\",)).fetchone()[0]; print(\"AI queue depth:\", n)"'

say "AI queue depth (captures with ai_state=queued)"
run_ssh "$REMOTE_QUEUE"

say "deploy done: $INSTANCE ($ZONE)"
