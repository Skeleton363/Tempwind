#!/bin/bash
# Start the Docker daemon if it isn't answering, and wait until it is.
#
# Cloud sessions restore the environment's files but not its processes, so
# dockerd must be started in every session; the SessionStart hook
# (.claude/hooks/session-start.sh) calls this. `service docker start` fails
# in this sandbox on a ulimit error, so dockerd is started directly.
# Idempotent. Exits 0 once `docker info` answers, 1 otherwise.
set -u

log=/tmp/dockerd.log
SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"

if ! command -v dockerd >/dev/null 2>&1; then
  echo "start-dockerd: dockerd is not installed" >&2
  exit 1
fi
if docker info >/dev/null 2>&1; then
  exit 0
fi

if ! pgrep -x dockerd >/dev/null; then
  # Pid files restored from the environment snapshot belong to no process
  # and may name a recycled pid; remove them before starting.
  $SUDO rm -f /var/run/docker.pid /var/run/docker/containerd/containerd.pid
  $SUDO setsid -f dockerd >> "$log" 2>&1 < /dev/null
fi

for _ in $(seq 1 60); do
  if docker info >/dev/null 2>&1; then
    echo "start-dockerd: docker $(docker info --format '{{.ServerVersion}}') is up"
    exit 0
  fi
  sleep 1
done
echo "start-dockerd: dockerd did not answer within 60 s; see $log" >&2
exit 1
