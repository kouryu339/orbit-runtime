"""Verify local release archives and SDK pins before uploading them.

Usage: python scripts/verify-release-artifacts.py OUTPUT_DIR VERSION [--local-preview]
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tarfile
import zipfile


def digest(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def main() -> None:
    output = Path(sys.argv[1]).resolve()
    version = sys.argv[2].removeprefix("v")
    local_preview = "--local-preview" in sys.argv[3:]
    repo = Path(__file__).resolve().parent.parent
    prefix = "orbit-runtime"
    report = {}

    def archive(name: str) -> dict[str, bytes]:
        filename = f"{name}.zip"
        path = output / filename
        checksum = digest(path.read_bytes())
        assert (output / f"{filename}.sha256").read_text().split() == [checksum, filename]
        with zipfile.ZipFile(path) as bundle:
            assert bundle.testzip() is None
            files = {}
            for entry in bundle.infolist():
                parts = PurePosixPath(entry.filename.replace("\\", "/")).parts
                assert parts[0] == name and ".." not in parts and not entry.filename.startswith("/")
                if not entry.is_dir():
                    key = "/".join(parts[1:])
                    assert key not in files, key
                    files[key] = bundle.read(entry)
        report[filename] = {"bytes": path.stat().st_size, "sha256": checksum, "files": len(files)}
        return files

    manifest = json.loads((repo / "sdk/runtime/release_manifest.json").read_text(encoding="utf-8-sig"))
    if not local_preview:
        assert manifest["runtime_version"] == version and manifest["release_tag"] == f"v{version}"
    for platform, library in (("windows-x86_64", "bin/agent_runtime.dll"), ("linux-x86_64", "lib/libagent_runtime.so")):
        name = f"{prefix}-runtime-v{version}-{platform}"
        files = archive(name)
        assert not any(key.startswith("sdk/") for key in files), "SDK pins must be outside native archives"
        assert files[library] == (repo / "dist/runtime" / Path(library).name).read_bytes()
        assert files["include/agent_runtime.h"] == (repo / "agent_runtime_ffi/include/agent_runtime.h").read_bytes()
        for schema in ("logic.schema.json", "resource.schema.json"):
            assert files[f"schemas/{schema}"] == (repo / "corework/src/workflow" / schema).read_bytes()
        assert files["schemas/skill-transaction.schema.json"] == (repo / "agent_runtime_ffi/docs/skill-transaction.schema.json").read_bytes()
        assert files["schemas/rpc-endpoint-add.schema.json"] == (repo / "agent_runtime_ffi/docs/rpc-endpoint-add.schema.json").read_bytes()
        for language in ("zh", "en"):
            assert files[f"docs/{language}/workflow-json-source-syntax.md"] == (repo / f"ai-assistant/prompts/{language}/workflow_tool_rules.md").read_bytes()
            assert files[f"docs/{language}/17-dynamic-rpc-additions.md"] == (repo / f"agent_runtime_ffi/docs/{language}/17-dynamic-rpc-additions.md").read_bytes()
        info = json.loads(files["build-info.json"])
        assert info["runtime_version"] == version, info
        if local_preview:
            assert info["distribution"] == "local-preview", info
        else:
            assert not info["worktree_dirty"], info
        assert info["native_sha256"] == digest(files[library])
        assert info["workflow_schema"] == "orbit.workflow.resource/v1"
        if platform.startswith("windows"):
            assert files[library][:2] == b"MZ"
        else:
            assert files[library][:6] == b"\x7fELF\x02\x01"
            assert int.from_bytes(files[library][18:20], "little") == 62
        if not local_preview:
            asset = manifest["assets"][platform]
            assert asset["archive"] == f"{name}.zip"
            assert asset["sha256"] == report[f"{name}.zip"]["sha256"]

    name = f"{prefix}-frontend-svelte-v{version}"
    files = archive(name)
    assert set(files) == {"orbit-conversation-svelte.js", "THIRD_PARTY_NOTICES.txt", "LICENSE", "NOTICE", "README.md", "manifest.json"}
    frontend = json.loads(files["manifest.json"])
    assert frontend["runtime_version"] == version and frontend["format"] == "esm"
    for filename, checksum in frontend["files"].items():
        assert digest(files[filename]) == checksum
    script = (output / f"{name}.js").read_bytes()
    assert script == files["orbit-conversation-svelte.js"] == (repo / "ui/svelte/dist-library/orbit-conversation-svelte.js").read_bytes()
    assert (output / f"{name}.js.sha256").read_text().split() == [digest(script), f"{name}.js"]

    name = f"{prefix}-browser-v{version}"
    browser_files = archive(name)
    package = json.loads(browser_files["package.json"])
    assert package["version"] == version and package["name"] == "@orbit-runtime/browser"
    browser_manifest = json.loads(browser_files["dist/manifest.json"])
    assert browser_manifest["version"] == version
    assert browser_manifest["target"] == "wasm32-unknown-unknown"
    assert {"README.md", "USAGE.zh-CN.md", "LICENSE", "NOTICE", "examples/quickstart.js"} <= browser_files.keys()
    for filename, info in browser_manifest["files"].items():
        content = browser_files[f"dist/{filename}"]
        assert len(content) == info["bytes"] and digest(content) == info["sha256"]
    assert browser_files["dist/wasm/orbit_runtime_wasm_bg.wasm"][:8] == b"\0asm\x01\0\0\0"
    for filename, content in browser_files.items():
        assert content == (repo / "sdk/runtime/js" / filename).read_bytes(), filename
    tgz = output / f"{name}.tgz"
    tgz_hash = digest(tgz.read_bytes())
    assert (output / f"{name}.tgz.sha256").read_text().split() == [tgz_hash, tgz.name]
    with tarfile.open(tgz, "r:gz") as bundle:
        packed = {}
        for entry in bundle.getmembers():
            parts = PurePosixPath(entry.name).parts
            assert parts[0] == "package" and ".." not in parts and not entry.name.startswith("/")
            assert entry.isfile(), entry.name
            key = "/".join(parts[1:])
            assert key not in packed, key
            packed[key] = bundle.extractfile(entry).read()
        assert packed == browser_files, "browser ZIP and npm package differ"
    report[tgz.name] = {"bytes": tgz.stat().st_size, "sha256": tgz_hash, "files": len(packed)}

    files = archive(f"{prefix}-sdk-v{version}")
    sdk_manifest = json.loads(files["sdk/release_manifest.json"])
    assert sdk_manifest == manifest
    tracked = subprocess.check_output(["git", "ls-files", "--", "sdk/runtime"], cwd=repo, text=True).splitlines()
    assert set(files) == {"sdk/" + path.removeprefix("sdk/runtime/") for path in tracked} | {"LICENSE", "NOTICE"}
    for path in tracked:
        assert files["sdk/" + path.removeprefix("sdk/runtime/")] == (repo / path).read_bytes()
    for source in ("sdk/rust/src/release.rs", "sdk/go/release.go", "sdk/python/corework_runtime/native.py"):
        for asset in manifest["assets"].values():
            assert asset["sha256"].encode() in files[source]
            assert asset["archive"].encode() in files[source]
    print(json.dumps({"passed": True, "version": version, "artifacts": report}, indent=2))


if __name__ == "__main__":
    main()
