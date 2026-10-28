"""Check an extracted Windows package via its Python SDK and actual native ABI.

Usage: python verify-dynamic-clusters.py PACKAGE_ROOT NEW_AUDIT_DIRECTORY
No LLM requests or external tools run. Restart checks use fresh OS processes.
"""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def registration(role: str = "cluster-role-a") -> dict:
    return {
        "schema": "agent-runtime-agent-cluster-registration/v1",
        "id": "native-team", "description": "Native cluster audit",
        "focus_agent_id": "primary",
        "agents": [{"id": "primary", "name": "Primary", "role": role,
                    "features": ["cluster-feature"],
                    "system_skills": {"thinking": "thinking-pro"}}],
    }


def phase(package: Path, audit: Path, name: str) -> None:
    sys.path.insert(0, str(package / "sdk/python"))
    from corework_runtime import Runtime, RuntimeError as NativeError

    source_files = [*audit.glob("skills/*/SKILL.md"), audit / "author-cluster.json"]
    before = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in source_files}
    runtime = Runtime(package / "bin/agent_runtime.dll", {
        "schema": "agent-runtime-create-options/v1", "data_dir": str(audit / f"data-{name}"),
        "language": "en", "log_level": "error", "restore_policy": "strict",
    })
    try:
        assert {"runtime.add_agent_cluster", "runtime.update_agent_cluster"} <= set(runtime.capabilities["commands"])
        runtime.register_resources({"schema": "agent-runtime-resource-registration/v1",
            "id": "cluster-audit-resources", "skills": {"root_dir": str(audit / "skills")}})
        runtime.register_llm({"schema": "agent-runtime-llm-registration/v1", "id": "cluster-audit-llm",
            "current_model_uid": 7333, "providers": [{"id": 1, "name": "test-provider", "type": "deepseek",
                "base_url": "https://example.invalid", "api_key": "sk-native-audit",
                "api_paradigm": "openai_chat_completions", "enabled_models": [{"uid": 7333, "model_id": "native-cluster-model"}]}]})
        runtime.start()

        def rejected(command: str, payload: dict, kind: str | None = None) -> dict:
            try:
                runtime.invoke(command, payload)
            except NativeError as error:
                if kind:
                    assert error.error["kind"] == kind, error.error
                return dict(error.error)
            raise AssertionError(f"Unexpectedly accepted {command}")

        original = registration()
        if name == "lifecycle":
            baseline = runtime.agent_cluster_definitions()
            invalid = copy.deepcopy(original)
            invalid["agents"][0].pop("role")
            rejected("runtime.add_agent_cluster", {"registration": invalid})
            rejected("runtime.add_agent_cluster", {"registration": registration("cluster-feature")})
            assert runtime.agent_cluster_definitions() == baseline
            accepted = runtime.add_agent_cluster(original)
            version = accepted["cluster"]["version"]
            assert version.startswith("sha256:")
            assert accepted["cluster"]["agents"][0]["role"] == "cluster-role-a"
            rejected("runtime.add_agent_cluster", {"registration": original})
            first = runtime.spawn_conversation(cluster_id="native-team")["conversation_id"]
            second = runtime.spawn_conversation(cluster_id="native-team")["conversation_id"]
            old = runtime.export_conversation_snapshot(first)
            assert old["cluster_definition"]["version"] == version
            runtime.pause_conversation(second)
            replacement = registration("cluster-role-b")
            blocked = rejected("runtime.update_agent_cluster", {
                "registration": replacement, "expected_version": version}, "cluster_in_use")
            assert sorted(blocked["details"]["conversation_ids"]) == sorted([first, second])
            runtime.close_conversation(first)
            blocked_one = rejected("runtime.update_agent_cluster", {
                "registration": replacement, "expected_version": version}, "cluster_in_use")
            assert blocked_one["details"]["conversation_ids"] == [second]
            runtime.close_conversation(second)
            changed = runtime.update_agent_cluster(replacement, version)
            current_version = changed["cluster"]["version"]
            assert current_version != version
            rejected("runtime.update_agent_cluster", {"registration": original,
                "expected_version": version}, "cluster_version_conflict")
            rejected("conversation.spawn_from_snapshot", {"spawn": {"cluster_id": "native-team"},
                "snapshot": old}, "cluster_version_conflict")
            # A no-op replacement is accepted only if no conversation of the
            # cluster remains live, including any accidentally leaked restore.
            assert runtime.update_agent_cluster(replacement, current_version)["cluster"]["version"] == current_version
            fresh = runtime.spawn_conversation(cluster_id="native-team")["conversation_id"]
            current = runtime.export_conversation_snapshot(fresh)
            rejected("conversation.import_snapshot", {"snapshot": old,
                "options": {"conversation_id": fresh}}, "cluster_version_conflict")
            assert runtime.export_conversation_snapshot(fresh)["ledger"] == current["ledger"]
            runtime.close_conversation(fresh)
            for filename, document in (("current-snapshot.json", current), ("old-snapshot.json", old),
                                       ("accepted-registration.json", replacement)):
                (audit / filename).write_text(json.dumps(document, ensure_ascii=False), encoding="utf-8")
            result = {"invalid_role_rejected": True, "waiting_and_paused_block_updates": True,
                "occupants": blocked["details"]["conversation_ids"], "one_remaining_still_blocks": True,
                "closed_all_allows_update": True, "stale_version_rejected": True,
                "old_snapshot_restore_and_import_rejected": True, "failed_restore_created_no_conversation": True,
                "accepted_version": current_version}
        elif name == "restart":
            saved = json.loads((audit / "accepted-registration.json").read_text(encoding="utf-8"))
            current = json.loads((audit / "current-snapshot.json").read_text(encoding="utf-8"))
            old = json.loads((audit / "old-snapshot.json").read_text(encoding="utf-8"))
            accepted = runtime.add_agent_cluster(saved)
            assert accepted["cluster"]["version"] == current["cluster_definition"]["version"]
            restored = runtime.spawn_conversation_from_snapshot(spawn={"cluster_id": "native-team"}, snapshot=current)
            runtime.close_conversation(restored["conversation_id"])
            rejected("conversation.spawn_from_snapshot", {"spawn": {"cluster_id": "native-team"},
                "snapshot": old}, "cluster_version_conflict")
            missing = copy.deepcopy(current)
            missing.pop("cluster_definition")
            rejected("conversation.spawn_from_snapshot", {"spawn": {"cluster_id": "native-team"}, "snapshot": missing})
            assert runtime.update_agent_cluster(saved, accepted["cluster"]["version"])["cluster"]["version"] == accepted["cluster"]["version"]
            result = {"version_stable_across_process_restart": True, "matching_snapshot_restored": True,
                "stale_snapshot_rejected": True, "missing_snapshot_version_rejected": True}
        elif name == "skill-reload":
            accepted = runtime.add_agent_cluster(original)
            version = accepted["cluster"]["version"]
            first = runtime.spawn_conversation(cluster_id="native-team")["conversation_id"]
            old = runtime.export_conversation_snapshot(first)
            runtime.close_conversation(first)
            shared = copy.deepcopy(original)
            shared["id"] = "shared-team"
            runtime.add_agent_cluster(shared)
            other = runtime.spawn_conversation(cluster_id="shared-team")["conversation_id"]
            runtime.pause_conversation(other)
            role_path = audit / "skills/cluster-role-a/SKILL.md"
            feature_path = audit / "skills/cluster-feature/SKILL.md"
            role_file = role_path.read_text(encoding="utf-8")
            feature_file = feature_path.read_text(encoding="utf-8")

            def save(path: Path, content: str) -> None:
                temporary = path.with_suffix(".tmp")
                temporary.write_text(content, encoding="utf-8")
                temporary.replace(path)

            save(role_path, role_file.replace("Perform assigned work.", "Use the revised identity instructions."))
            blocked = rejected("runtime.update_agent_cluster", {"registration": original,
                "expected_version": version, "reload_skills": ["cluster-role-a"]}, "skill_in_use")
            assert blocked["details"]["dependencies"] == [{"conversation_id": other,
                "agent_id": "primary", "skill_names": ["cluster-role-a"]}]
            runtime.close_conversation(other)
            save(feature_path, '---\nname: cluster-feature\ndescription: "Native audit"\nkind: capability\n---\n')
            rejected("runtime.update_agent_cluster", {"registration": original,
                "expected_version": version, "reload_skills": ["cluster-role-a", "cluster-feature"]})
            first = runtime.spawn_conversation(cluster_id="native-team")["conversation_id"]
            assert runtime.export_conversation_snapshot(first)["skill_versions"] == old["skill_versions"]
            runtime.close_conversation(first)
            save(feature_path, feature_file)
            before = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in source_files}
            changed = runtime.update_agent_cluster(original, version, reload_skills=["cluster-role-a"])
            assert changed["cluster"]["version"] == version
            assert changed["reloaded_skills"]["cluster-role-a"] != old["skill_versions"]["cluster-role-a"]
            stale = rejected("conversation.spawn_from_snapshot", {"spawn": {"cluster_id": "native-team"}, "snapshot": old})
            assert stale["kind"] == "skill_version_conflict", stale
            # A failed restore must not leak a conversation into the cluster.
            runtime.update_agent_cluster(original, version)
            fresh = runtime.spawn_conversation(cluster_id="native-team")["conversation_id"]
            current = runtime.export_conversation_snapshot(fresh)
            stale = rejected("conversation.import_snapshot", {"snapshot": old, "options": {"conversation_id": fresh}})
            assert stale["kind"] == "skill_version_conflict", stale
            assert runtime.export_conversation_snapshot(fresh)["ledger"] == current["ledger"]
            runtime.close_conversation(fresh)
            for filename, document in (("skill-current-snapshot.json", current), ("skill-old-snapshot.json", old)):
                (audit / filename).write_text(json.dumps(document, ensure_ascii=False), encoding="utf-8")
            result = {"cross_cluster_paused_agent_blocks_reload": True, "dependency_ids": blocked["details"]["dependencies"],
                "invalid_batch_keeps_all_cached_bodies": True, "sdk_reload_succeeds_after_close": True,
                "body_only_reload_keeps_cluster_version": True, "stale_skill_restore_and_import_rejected": True}
        else:
            current = json.loads((audit / "skill-current-snapshot.json").read_text(encoding="utf-8"))
            old = json.loads((audit / "skill-old-snapshot.json").read_text(encoding="utf-8"))
            runtime.add_agent_cluster(original)
            restored = runtime.spawn_conversation_from_snapshot(spawn={"cluster_id": "native-team"}, snapshot=current)
            assert runtime.export_conversation_snapshot(restored["conversation_id"])["skill_versions"] == current["skill_versions"]
            runtime.close_conversation(restored["conversation_id"])
            stale = rejected("conversation.spawn_from_snapshot", {"spawn": {"cluster_id": "native-team"}, "snapshot": old})
            assert stale["kind"] == "skill_version_conflict", stale
            result = {"skill_versions_stable_across_process_restart": True, "updated_snapshot_restored": True,
                "old_body_archive_rejected_after_restart": True}
        after = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in source_files}
        assert before == after, "Native operations changed host source files"
        (audit / f"{name}.json").write_text(json.dumps({"phase": name, "runtime_version": runtime.version,
            "host_source_files_unchanged": True, "result": result}, ensure_ascii=False, indent=2), encoding="utf-8")
    finally:
        runtime.close()


def main() -> None:
    package, audit = (Path(p).resolve() for p in sys.argv[1:3])
    if len(sys.argv) == 4:
        phase(package, audit, sys.argv[3])
        return
    audit.mkdir(parents=True, exist_ok=False)
    for name, kind in (("cluster-role-a", "role"), ("cluster-role-b", "role"), ("cluster-feature", "capability")):
        folder = audit / "skills" / name
        folder.mkdir(parents=True)
        (folder / "SKILL.md").write_text(f'---\nname: {name}\ndescription: "Native audit"\nkind: {kind}\n---\n\nPerform assigned work.\n', encoding="utf-8")
    (audit / "author-cluster.json").write_text(json.dumps(registration(), indent=2), encoding="utf-8")
    for name in ("lifecycle", "restart", "skill-reload", "skill-restart"):
        run = subprocess.run([sys.executable, str(Path(__file__).resolve()), str(package), str(audit), name],
            capture_output=True, text=True, encoding="utf-8", errors="replace")
        (audit / f"{name}.log").write_text(run.stdout + run.stderr, encoding="utf-8")
        if run.returncode:
            raise RuntimeError(f"Cluster native audit failed: {audit / (name + '.log')}")
    print(json.dumps({"passed": True, "package": str(package), "reports": str(audit)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
