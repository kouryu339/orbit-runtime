"""Native candidate/Workflow transaction audit, without LLM or business tools.

Usage: python verify-runtime-transactions.py DLL SDK_PYTHON_DIR NEW_AUDIT_DIR
Each restart runs in a fresh OS process; no existing user data is read or changed.
"""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def source(name: str, kind: str, body: str) -> str:
    return f"---\nname: {name}\ndescription: Native transaction audit\nkind: {kind}\n---\n\n{body}\n"


def cluster(identity: str, role: str) -> dict:
    return {"schema": "agent-runtime-agent-cluster-registration/v1", "id": identity, "description": "Native employee audit",
            "focus_agent_id": "primary", "agents": [{"id": "primary", "role": role, "features": ["shared_feature"]}]}


def phase(dll: Path, sdk: Path, audit: Path, name: str) -> None:
    sys.path.insert(0, str(sdk))
    from corework_runtime import Runtime, RuntimeError as NativeError
    runtime = Runtime(dll, {"schema": "agent-runtime-create-options/v1", "data_dir": str(audit / ("data-" + name)),
                            "language": "en", "log_level": "error", "restore_policy": "strict"})
    original_files = {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in (audit / "skills").glob("*/SKILL.md")}
    try:
        assert runtime.capabilities["features"]["skill_candidate_transactions"] == 1
        assert runtime.capabilities["features"]["workflow_running_edit_guard"] is True
        runtime.register_resources({"schema": "agent-runtime-resource-registration/v1", "id": "transaction-audit",
                                    "skills": {"root_dir": str(audit / "skills"), "managed_dir": str(audit / "managed")},
                                    "workflows": {"root_dir": str(audit / "workflows")}})
        runtime.register_llm({"schema": "agent-runtime-llm-registration/v1", "id": "audit-model", "current_model_uid": 7333,
                             "providers": [{"id": 1, "name": "audit", "type": "deepseek", "base_url": "https://example.invalid",
                                            "api_key": "sk-audit", "api_paradigm": "openai_chat_completions",
                                            "enabled_models": [{"uid": 7333, "model_id": "audit"}]}]})
        runtime.start()

        def reject(command: str, payload: dict, kind: str) -> dict:
            try:
                runtime.invoke(command, payload)
            except NativeError as error:
                assert error.error["kind"] == kind, error.error
                return dict(error.error)
            raise AssertionError(f"Unexpectedly accepted {command}")

        if name == "commit":
            missing = runtime.read_skill("employee_role")
            assert missing["exists"] is False and missing["store_revision"] == 0
            request = {"expected_store_revision": 0, "skills": [
                {"name": "employee_role", "kind": "role", "expected_version": None,
                 "source": source("employee_role", "role", "Use the original responsibilities.")},
                {"name": "shared_feature", "kind": "capability", "expected_version": None,
                 "source": source("shared_feature", "capability", "Provide reusable knowledge.")}
            ], "cluster": {"registration": cluster("employee-team", "employee_role"), "expected_version": None}}
            assert runtime.validate_skill_transaction(request)["committed"] is False
            assert not (audit / "managed").exists()
            accepted = runtime.commit_skill_transaction(request)
            assert accepted["committed"] is True and accepted["store_revision"] == 1
            assert runtime.read_skill("employee_role")["source"] == request["skills"][0]["source"]
            current = runtime.spawn_conversation(cluster_id="employee-team")["conversation_id"]
            old = runtime.export_conversation_snapshot(current)
            runtime.close_conversation(current)
            runtime.add_agent_cluster(cluster("other-team", "other_role"))
            other = runtime.spawn_conversation(cluster_id="other-team")["conversation_id"]
            runtime.pause_conversation(other)
            feature = runtime.read_skill("shared_feature")
            blocked = reject("runtime.commit_skill_transaction", {"expected_store_revision": 1, "skills": [{
                "name": "shared_feature", "kind": "capability", "expected_version": feature["version"],
                "source": source("shared_feature", "capability", "Changed feature.")}]}, "skill_in_use")
            assert blocked["details"]["dependencies"] == [{"conversation_id": other, "agent_id": "primary", "skill_names": ["shared_feature"]}]
            runtime.close_conversation(other)
            version = runtime.read_skill("employee_role")["version"]
            update = {"expected_store_revision": 1, "skills": [{"name": "employee_role", "kind": "role",
                      "expected_version": version, "source": source("employee_role", "role", "Use revised responsibilities.")}]}
            invalid = json.loads(json.dumps(update))
            invalid["skills"].append({"name": "bad", "kind": "role", "source": source("bad", "role", "")})
            manifest = (audit / "managed/catalog.json").read_bytes()
            reject("runtime.commit_skill_transaction", invalid, "invalid_argument")
            assert (audit / "managed/catalog.json").read_bytes() == manifest
            assert runtime.read_skill("employee_role")["version"] == version
            updated = runtime.commit_skill_transaction(update)
            assert updated["store_revision"] == 2
            reject("runtime.commit_skill_transaction", update, "skill_store_conflict")
            reject("conversation.spawn_from_snapshot", {"spawn": {"cluster_id": "employee-team"}, "snapshot": old}, "skill_version_conflict")
            current = runtime.spawn_conversation(cluster_id="employee-team")["conversation_id"]
            snapshot = runtime.export_conversation_snapshot(current)
            runtime.close_conversation(current)
            (audit / "old.json").write_text(json.dumps(old), encoding="utf-8")
            (audit / "current.json").write_text(json.dumps(snapshot), encoding="utf-8")
            (audit / "version.json").write_text(json.dumps(updated), encoding="utf-8")
            result = {"validation_writes_nothing": True, "new_role_and_feature_with_cluster": True, "shared_feature_occupancy_ids": True,
                      "batch_failure_preserves_manifest_and_cache": True, "stale_store_rejected": True, "stale_snapshot_rejected": True}
        elif name == "restart":
            updated = json.loads((audit / "version.json").read_text(encoding="utf-8"))
            assert runtime.read_skill("employee_role")["version"] == updated["skills"][0]["version"]
            current = json.loads((audit / "current.json").read_text(encoding="utf-8"))
            restored = runtime.spawn_conversation_from_snapshot(spawn={"cluster_id": "employee-team"}, snapshot=current)
            runtime.close_conversation(restored["conversation_id"])
            old = json.loads((audit / "old.json").read_text(encoding="utf-8"))
            reject("conversation.spawn_from_snapshot", {"spawn": {"cluster_id": "employee-team"}, "snapshot": old}, "skill_version_conflict")
            child = {"schema": "orbit.workflow.resource/v1", "id": "child", "metadata": {"name": "Child", "description": "Echo", "extensions": {"host": {"asset": "preserved"}}},
                     "state": "registered", "revision": 1, "logic": {"schema": "orbit.workflow.logic/v1", "inputs": {"text": {"type": "String"}},
                     "steps": [], "returns": {"text": {"value": {"expr": "input.text"}}}}}
            # The authored JSON source supports bare references; keep them intact.
            child_source = json.dumps(child).replace('{"expr": "input.text"}', 'input.text')
            runtime.register_workflow(child_source)
            parent_source = child_source.replace('"id": "child"', '"id": "parent"').replace('"name": "Child"', '"name": "Parent"')
            parent_source = parent_source.replace('"steps": []', '"steps": [{"id":1,"call":"WorkflowRef_child","args":{"text":input.text}}]').replace('"value": input.text', '"value": 1.text')
            runtime.register_workflow(parent_source)
            assert runtime.execute_workflow("parent", {"text": "before"})["result"]["outputs"]["text"] == "before"
            changed = runtime.register_workflow(child_source.replace('"Echo"', '"Edited"'))
            assert changed["state"] == "draft" and changed["revision"] == 2
            caller = next(item for item in runtime.list_workflows()["workflows"] if item["id"] == "parent")
            assert caller["production_executable"] is False
            reject("workflow.remove", {"id": "child", "expected_revision": 2}, "invalid_argument")
            assert runtime.execute_workflow("parent", {"text": "blocked"})["code"] != 0
            result = {"managed_role_and_cluster_automatically_restored": True, "snapshot_versions_stable": True,
                      "workflow_edit_preserves_identity_and_drafts": True, "formal_caller_blocked": True}
        else:
            catalog = runtime.list_workflows()["workflows"]
            assert next(item for item in catalog if item["id"] == "child")["kind"] == "draft"
            assert next(item for item in catalog if item["id"] == "parent")["production_executable"] is False
            runtime.promote_workflow("child", 2)
            assert runtime.execute_workflow("parent", {"text": "after"})["result"]["outputs"]["text"] == "after"
            result = {"draft_and_blocked_callers_survive_restart": True, "promotion_restores_callers": True}
        assert original_files == {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in (audit / "skills").glob("*/SKILL.md")}
        (audit / f"{name}.json").write_text(json.dumps({"phase": name, "runtime_version": runtime.version, "result": result}, indent=2), encoding="utf-8")
    finally:
        runtime.close()


def main() -> None:
    dll, sdk, audit = (Path(value).resolve() for value in sys.argv[1:4])
    if len(sys.argv) == 5:
        phase(dll, sdk, audit, sys.argv[4])
        return
    audit.mkdir(parents=True, exist_ok=False)
    (audit / "skills/other_role").mkdir(parents=True)
    (audit / "skills/other_role/SKILL.md").write_text(source("other_role", "role", "Provide an independent identity."), encoding="utf-8")
    (audit / "workflows").mkdir()
    for name in ("commit", "restart", "workflow-restart"):
        run = subprocess.run([sys.executable, str(Path(__file__).resolve()), str(dll), str(sdk), str(audit), name], capture_output=True, text=True, encoding="utf-8", errors="replace")
        (audit / f"{name}.log").write_text(run.stdout + run.stderr, encoding="utf-8")
        if run.returncode:
            raise RuntimeError(f"Native transaction audit failed: {audit / (name + '.log')}")
    print(json.dumps({"passed": True, "dll": str(dll), "reports": str(audit)}))


if __name__ == "__main__":
    main()
