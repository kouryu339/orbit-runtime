#!/usr/bin/env bash
# Pull a docker.io image via the first reachable Chinese mirror, then retag
# to its bare name. Usage: pull-via-mirror.sh <image:tag> [<image:tag> ...]
set -e
MIRRORS=(docker.m.daocloud.io docker.1panel.live docker.xuanyuan.me)
for img in "$@"; do
  if docker image inspect "$img" >/dev/null 2>&1; then
    echo "[skip] $img already present"
    continue
  fi
  pulled=""
  for src in "${MIRRORS[@]}"; do
    echo "=== pull $img via $src ==="
    if docker pull "$src/library/$img" 2>&1 | tail -3; then
      if docker image inspect "$src/library/$img" >/dev/null 2>&1; then
        docker tag "$src/library/$img" "$img"
        pulled=1
        break
      fi
    fi
  done
  if [ -z "$pulled" ]; then
    echo "ERROR: all mirrors failed for $img"
    exit 1
  fi
done
docker images --format '{{.Repository}}:{{.Tag}}' | sort
