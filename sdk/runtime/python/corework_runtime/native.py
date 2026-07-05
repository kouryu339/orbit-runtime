from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import sys
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path


DEFAULT_RUNTIME_VERSION = "0.4.0"
DEFAULT_RELEASE_TAG = f"v{DEFAULT_RUNTIME_VERSION}"
DEFAULT_REPOSITORY = "kouryu339/orbit-runtime"
ABI_VERSION = 1

_ASSETS = {
    "windows-x86_64": {
        "archive": "orbit-runtime-runtime-v0.4.0-windows-x86_64.zip",
        "sha256": "948fa39b07155640e591bff94ca7d5c770f69c300e836f1dceda6f3d3b2e82ca",
        "library": "bin/agent_runtime.dll",
    },
    "linux-x86_64": {
        "archive": "orbit-runtime-runtime-v0.4.0-linux-x86_64.zip",
        "sha256": "7d6c8daa15287a828b861c8c5365f9c5dde432fdca90757943a4a845062e991a",
        "library": "lib/libagent_runtime.so",
    },
}


@dataclass(frozen=True)
class RuntimeArtifact:
    platform_id: str
    archive: str
    sha256: str
    library: str
    url: str


class UnsupportedRuntimePlatform(RuntimeError):
    pass


def runtime_platform_id() -> str:
    system = platform.system().lower()
    machine = platform.machine().lower()
    if machine in {"amd64", "x86_64"}:
        arch = "x86_64"
    else:
        arch = machine
    if system == "windows" and arch == "x86_64":
        return "windows-x86_64"
    if system == "linux" and arch == "x86_64":
        return "linux-x86_64"
    raise UnsupportedRuntimePlatform(
        f"unsupported runtime platform: system={platform.system()} arch={platform.machine()}"
    )


def runtime_artifact(
    *,
    version: str = DEFAULT_RUNTIME_VERSION,
    platform_id: str | None = None,
    repository: str = DEFAULT_REPOSITORY,
) -> RuntimeArtifact:
    if version != DEFAULT_RUNTIME_VERSION:
        raise ValueError(
            f"this SDK release knows native runtime {DEFAULT_RUNTIME_VERSION}; got {version}"
        )
    platform_id = platform_id or runtime_platform_id()
    asset = _ASSETS.get(platform_id)
    if asset is None:
        raise UnsupportedRuntimePlatform(f"unsupported runtime platform: {platform_id}")
    archive = asset["archive"]
    tag = f"v{version}" if not version.startswith("v") else version
    return RuntimeArtifact(
        platform_id=platform_id,
        archive=archive,
        sha256=asset["sha256"],
        library=asset["library"],
        url=f"https://github.com/{repository}/releases/download/{tag}/{archive}",
    )


def default_cache_dir() -> Path:
    root = os.environ.get("ORBIT_RUNTIME_CACHE")
    if root:
        return Path(root)
    if os.name == "nt":
        base = os.environ.get("LOCALAPPDATA") or os.environ.get("APPDATA")
        if base:
            return Path(base) / "orbit-runtime"
    return Path.home() / ".cache" / "orbit-runtime"


def resolve_runtime_library(library_path: str | Path | None = None) -> Path | None:
    if library_path is not None:
        return Path(library_path)
    for name in ("ORBIT_RUNTIME_LIBRARY", "COREWORK_RUNTIME_LIBRARY"):
        value = os.environ.get(name)
        if value:
            return Path(value)
    return None


def ensure_runtime_package(
    *,
    version: str = DEFAULT_RUNTIME_VERSION,
    cache_dir: str | Path | None = None,
    platform_id: str | None = None,
    force: bool = False,
) -> Path:
    artifact = runtime_artifact(version=version, platform_id=platform_id)
    cache_root = Path(cache_dir) if cache_dir is not None else default_cache_dir()
    install_dir = cache_root / artifact.platform_id / f"v{version}"
    library_path = install_dir / artifact.library
    if library_path.exists() and not force:
        return install_dir

    archive_dir = cache_root / "_downloads"
    archive_dir.mkdir(parents=True, exist_ok=True)
    archive_path = archive_dir / artifact.archive
    if force or not archive_path.exists():
        _download_artifact(artifact, archive_path)

    digest = _sha256_file(archive_path)
    if digest.lower() != artifact.sha256.lower():
        archive_path.unlink(missing_ok=True)
        raise RuntimeError(
            f"checksum mismatch for {archive_path.name}: expected {artifact.sha256}, got {digest}"
        )

    if install_dir.exists():
        shutil.rmtree(install_dir)
    install_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive_path) as package:
        package.extractall(install_dir)

    roots = [item for item in install_dir.iterdir() if item.is_dir()]
    if len(roots) == 1 and (roots[0] / artifact.library).exists():
        staged_root = roots[0]
        for child in staged_root.iterdir():
            shutil.move(str(child), install_dir / child.name)
        staged_root.rmdir()

    if not library_path.exists():
        raise RuntimeError(f"runtime package did not contain {artifact.library}")
    return install_dir


def ensure_runtime_library(
    *,
    version: str = DEFAULT_RUNTIME_VERSION,
    cache_dir: str | Path | None = None,
    platform_id: str | None = None,
    force: bool = False,
) -> Path:
    artifact = runtime_artifact(version=version, platform_id=platform_id)
    package_dir = ensure_runtime_package(
        version=version,
        cache_dir=cache_dir,
        platform_id=artifact.platform_id,
        force=force,
    )
    return package_dir / artifact.library


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _download_artifact(artifact: RuntimeArtifact, destination: Path) -> None:
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        url = _private_asset_api_url(artifact, token)
        request = urllib.request.Request(url)
        request.add_header("Authorization", f"Bearer {token}")
        request.add_header("Accept", "application/octet-stream")
    else:
        request = urllib.request.Request(artifact.url)
    request.add_header("User-Agent", "orbit-runtime-sdk")
    with urllib.request.urlopen(request) as response:
        with destination.open("wb") as file:
            shutil.copyfileobj(response, file)


def _private_asset_api_url(artifact: RuntimeArtifact, token: str) -> str:
    api_url = f"https://api.github.com/repos/{DEFAULT_REPOSITORY}/releases/tags/{DEFAULT_RELEASE_TAG}"
    request = urllib.request.Request(api_url)
    if token:
        request.add_header("Authorization", f"Bearer {token}")
    request.add_header("Accept", "application/vnd.github+json")
    request.add_header("User-Agent", "orbit-runtime-sdk")
    with urllib.request.urlopen(request) as response:
        release = json.loads(response.read().decode("utf-8"))
    for asset in release.get("assets", []):
        if asset.get("name") == artifact.archive:
            return asset["url"]
    raise RuntimeError(f"release asset not found: {artifact.archive}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Download the Orbit Runtime native package.")
    parser.add_argument("--version", default=DEFAULT_RUNTIME_VERSION)
    parser.add_argument("--cache-dir")
    parser.add_argument("--platform", dest="platform_id")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--print-package-dir", action="store_true")
    args = parser.parse_args(argv)

    try:
        library = ensure_runtime_library(
            version=args.version,
            cache_dir=args.cache_dir,
            platform_id=args.platform_id,
            force=args.force,
        )
    except Exception as error:
        print(str(error), file=sys.stderr)
        return 1

    print(library.parent.parent if args.print_package_dir else library)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
