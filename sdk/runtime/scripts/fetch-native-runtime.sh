#!/usr/bin/env sh
set -eu

platform="${ORBIT_RUNTIME_PLATFORM:-}"
output_dir="${ORBIT_RUNTIME_OUTPUT_DIR:-.runtime}"
force="${ORBIT_RUNTIME_FORCE:-}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --platform)
      platform="$2"
      shift 2
      ;;
    --output-dir)
      output_dir="$2"
      shift 2
      ;;
    --force)
      force="1"
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
runtime_dir="$(dirname "$script_dir")"
manifest="$runtime_dir/release_manifest.json"

if [ -z "$platform" ]; then
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
  esac
  case "$os:$arch" in
    Linux:x86_64) platform="linux-x86_64" ;;
    *) echo "unsupported platform. Pass --platform linux-x86_64." >&2; exit 1 ;;
  esac
fi

read_manifest() {
  python3 - "$manifest" "$platform" "$1" <<'PY'
import json
import sys
manifest_path, platform_id, field = sys.argv[1:4]
with open(manifest_path, "r", encoding="utf-8") as file:
    manifest = json.load(file)
asset = manifest["assets"].get(platform_id)
if asset is None:
    raise SystemExit(f"release manifest does not contain platform {platform_id!r}")
if field in manifest:
    print(manifest[field])
else:
    print(asset[field])
PY
}

repo="$(read_manifest repository)"
tag="$(read_manifest release_tag)"
archive="$(read_manifest archive)"
sha256="$(read_manifest sha256)"
library="$(read_manifest library)"
url="https://github.com/$repo/releases/download/$tag/$archive"

archive_dir="$output_dir/_downloads"
package_dir="$output_dir/$platform/$tag"
archive_path="$archive_dir/$archive"
library_path="$package_dir/$library"

if [ -f "$library_path" ] && [ -z "$force" ]; then
  printf '%s\n' "$library_path"
  exit 0
fi

mkdir -p "$archive_dir" "$package_dir"
if [ -n "$force" ] || [ ! -f "$archive_path" ]; then
  token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
  download_url="$url"
  accept_header=""
  auth_args=""
  if [ -n "$token" ]; then
    asset_api_url="$(python3 - "$repo" "$tag" "$archive" "$token" <<'PY'
import json
import sys
import urllib.request
repo, tag, archive, token = sys.argv[1:5]
request = urllib.request.Request(f"https://api.github.com/repos/{repo}/releases/tags/{tag}")
request.add_header("Authorization", f"Bearer {token}")
request.add_header("Accept", "application/vnd.github+json")
request.add_header("User-Agent", "orbit-runtime-sdk")
with urllib.request.urlopen(request) as response:
    release = json.loads(response.read().decode("utf-8"))
for asset in release.get("assets", []):
    if asset.get("name") == archive:
        print(asset["url"])
        raise SystemExit(0)
raise SystemExit(f"release asset not found: {archive}")
PY
)"
    download_url="$asset_api_url"
    accept_header="Accept: application/octet-stream"
    auth_args="Authorization: Bearer $token"
  fi
  if command -v curl >/dev/null 2>&1; then
    if [ -n "$token" ]; then
      curl -L -H "$auth_args" -H "$accept_header" -H "User-Agent: orbit-runtime-sdk" "$download_url" -o "$archive_path"
    else
      curl -L "$download_url" -o "$archive_path"
    fi
  else
    if [ -n "$token" ]; then
      wget --header="$auth_args" --header="$accept_header" --header="User-Agent: orbit-runtime-sdk" -O "$archive_path" "$download_url"
    else
      wget -O "$archive_path" "$download_url"
    fi
  fi
fi

actual="$(sha256sum "$archive_path" | awk '{print $1}')"
if [ "$actual" != "$sha256" ]; then
  rm -f "$archive_path"
  echo "checksum mismatch for $archive: expected $sha256, got $actual" >&2
  exit 1
fi

rm -rf "$package_dir"
mkdir -p "$package_dir"
unzip -q "$archive_path" -d "$package_dir"

nested="$(find "$package_dir" -mindepth 1 -maxdepth 1 -type d | head -n 1 || true)"
if [ -n "$nested" ] && [ -f "$nested/$library" ]; then
  find "$nested" -mindepth 1 -maxdepth 1 -exec mv {} "$package_dir" \;
  rmdir "$nested"
fi

if [ ! -f "$library_path" ]; then
  echo "runtime library missing after extraction: $library_path" >&2
  exit 1
fi

printf '%s\n' "$library_path"
