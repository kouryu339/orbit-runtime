#!/usr/bin/env bash
set -e
for src in docker.1panel.live docker.xuanyuan.me docker.m.daocloud.io registry.cn-hangzhou.aliyuncs.com; do
  echo "=== try $src ==="
  if docker pull "$src/library/redis:7-alpine" 2>&1 | tail -3; then
    if docker image inspect "$src/library/redis:7-alpine" >/dev/null 2>&1; then
      docker tag "$src/library/redis:7-alpine" redis:7-alpine
      echo "SUCCESS via $src"
      docker images | grep -E '^(postgres|redis)'
      exit 0
    fi
  fi
done
echo "all mirrors failed"
exit 1
