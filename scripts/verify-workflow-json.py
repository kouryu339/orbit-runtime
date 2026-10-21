"""Exercise an extracted local package through its Python SDK and native ABI.

Usage: python verify-workflow-json.py PACKAGE_ROOT EMPTY_AUDIT_DIRECTORY
Runs each restart phase in a fresh OS process. No LLM or external tools run.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys

CHILD = r'''{
  "schema":"orbit.workflow.resource/v1","id":"z-echo",
  "metadata":{"name":"Echo","description":"Native audit","extensions":{"host":{"asset":"中文"}}},
  "state":"registered","revision":7,
  "logic":{"schema":"orbit.workflow.logic/v1","inputs":{"text":{"type":"String"}},
  "steps":[],"returns":{"text":{"value":input.text}}}
}'''
SOURCE = r'''{
  "schema":"orbit.workflow.resource/v1","id":"a-main",
  "metadata":{"name":"Main","description":"Native lifecycle audit","extensions":{}},
  "state":"draft","revision":1,
  "logic":{
    "schema":"orbit.workflow.logic/v1",
    "inputs":{"names":{"type":"String","disablename":"Names","description":"Comma separated names."},"selector":{"type":"String"}},
    "variables":{"count":{"type":"i64","initial":0,"disablename":"Count"},"label":{"type":"String","initial":""}},
    "steps":[
      {"id":1,"for":{"fn":"split","args":[{"input":"names"},[","]]},"body":[
        {"id":2,"set":"count","value":{"fn":"add","args":[{"var":"count"},1]}},
        {"id":3,"call":"DebugPrintNode","args":{"Value":{"item":var.item,"index":var.index}}}
      ]},
      {"id":10,"switch":{"input":"selector"},"cases":[
        {"value":"a","body":[{"id":11,"set":"label","value":"A"}]},
        {"value":"b","body":[{"id":12,"set":"label","value":"B"}]},
        {"value":"c","body":[{"id":13,"set":"label","value":"C"}]}
      ],"default":[{"id":14,"set":"label","value":"D"}]},
      {"id":30,"call":"WorkflowRef_z-echo","args":{"text":input.names}}
    ],
    "returns":{"result":{"value":{"text":30.text,"count":var.count,"label":var.label,
      "parts":split(input.names,[","]),"constant":"var.count","nested":[true,null,add(var.count,1)]},
      "disablename":"Result","description":"Return count, switch label, and referenced output."}}
  }
}'''


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def phase(package: Path, audit: Path, name: str) -> None:
    sys.path.insert(0, str(package / "sdk/python"))
    from corework_runtime import Runtime, RuntimeError as NativeError

    directory = audit / "workflows"
    before = {p.name: sha(p) for p in directory.iterdir()}
    runtime = Runtime(package / "bin/agent_runtime.dll", {
        "schema": "agent-runtime-create-options/v1", "data_dir": str(audit / "data"),
        "language": "en", "log_level": "error", "restore_policy": "strict",
    })
    try:
        commands = set(runtime.capabilities["commands"])
        assert {c for c in commands if c.startswith("workflow.")} == {
            "workflow.list", "workflow.register", "workflow.validate", "workflow.promote", "workflow.execute", "workflow.remove",
        }
        assert not {"studio.open_workflow", "studio.open_agent_test"} & commands
        runtime.register_resources({"schema":"agent-runtime-resource-registration/v1","id":"native-audit",
            "skills":{"root_dir":str(audit / "skills")},"workflows":{"root_dir":str(directory)}})
        if name.startswith("reject-"):
            try:
                runtime.start()
            except NativeError as error:
                assert before == {p.name: sha(p) for p in directory.iterdir()}, "Failed startup rewrote source"
                (audit / f"{name}.json").write_text(json.dumps({"phase":name,"rejected":True,
                    "files_unchanged":True,"error":str(error)},indent=2),encoding="utf-8")
                return
            raise AssertionError(f"Invalid startup accepted: {name}")
        runtime.start()
        assert before == {p.name: sha(p) for p in directory.iterdir()}, "Startup rewrote source"
        listed = runtime.list_workflows()["workflows"]
        assert all("script" not in item and "logic" not in item for item in listed)
        assert runtime.list_workflows(kind="all") == runtime.list_workflows()

        def rejected(command: str, payload: dict, kind: str | None = None) -> None:
            try:
                runtime.invoke(command, payload)
            except NativeError as error:
                if kind:
                    assert error.error["kind"] == kind, error.error
            else:
                raise AssertionError(f"Unexpectedly accepted: {command}")

        def execute(identity: str, selector: str, label: str) -> dict:
            output = runtime.execute_workflow(identity, {"names":"a,b,c","selector":selector}, trace=True)
            result = output["result"]["outputs"]["result"]
            assert result == {"text":"a,b,c","count":3,"label":label,"parts":["a","b","c"],
                              "constant":"var.count","nested":[True,None,4]}, output
            return result

        if name == "register":
            for language in ("zh", "en"):
                syntax = (package / f"docs/{language}/workflow-json-source-syntax.md").read_text(encoding="utf-8")
                minimal = syntax.split("```text\n", 1)[1].split("```", 1)[0]
                assert runtime.validate_workflow(minimal)["valid"] is True
                assert not (directory / "example.workflow.json").exists()
            assert runtime.validate_workflow(SOURCE)["valid"] is True
            assert not (directory / "a-main.workflow.json").exists()
            assert runtime.register_workflow(SOURCE)["revision"] == 1
            drafts = runtime.list_workflows(kind="draft")["workflows"]
            assert [(item["id"],item["kind"],item["revision"]) for item in drafts] == [("a-main","draft",1)]
            assert [item["id"] for item in runtime.list_workflows(kind="registered")["workflows"]] == ["z-echo"]
            rejected("workflow.list", {"kind":"invalid"})
            rejected("workflow.list", {"kind":3})
            execute("a-main", "b", "B")
            assert runtime.register_workflow(SOURCE)["revision"] == 2
            saved = (directory / "a-main.workflow.json").read_bytes()
            rejected("workflow.register", {"source":SOURCE})
            rejected("workflow.register", {"source":SOURCE.replace('"revision":1','"revision":2').replace("30.text","999.missing")})
            assert saved == (directory / "a-main.workflow.json").read_bytes()
            runtime.promote_workflow("a-main", 2)
            execute("a-main", "other", "D")
            broken = CHILD.replace('"text":{"value":input.text}', '"renamed":{"value":input.text}')
            rejected("workflow.register", {"source":broken})
            assert (directory / "echo.workflow.json").read_text(encoding="utf-8") == CHILD
            rejected("workflow.remove", {"id":"z-echo","expected_revision":7})
            draft = SOURCE.replace("a-main","saved-draft").replace('"name":"Main"','"name":"Saved Draft"')
            runtime.register_workflow(draft)
            for command in ("workflow.execute_script","workflow.read","workflow.compile","studio.open_workflow","studio.open_agent_test"):
                rejected(command, {}, "deprecated_api")
            result = {"main":execute("a-main","a","A"),"draft":execute("saved-draft","c","C")}
        elif name == "restart":
            assert {item["id"] for item in runtime.list_workflows()["workflows"]} == {"a-main","saved-draft","z-echo"}
            result = {"main":execute("a-main","b","B"),"draft":execute("saved-draft","c","C")}
            runtime.remove_workflow("saved-draft",1)
            runtime.remove_workflow("a-main",3)
            runtime.remove_workflow("z-echo",7)
            assert runtime.list_workflows()["workflows"] == []
            assert {p.name for p in directory.iterdir()} == {"legacy.workflow"}
        else:
            assert runtime.list_workflows()["workflows"] == []
            # Execution failures use the existing Workflow result envelope.
            for identity in ("a-main", "z-echo"):
                missing = runtime.invoke("workflow.execute", {"id":identity})
                assert missing["code"] == 404, missing
            result = {"removed_resources_stay_removed":True}
        (audit / f"{name}.json").write_text(json.dumps({"version":runtime.version,"phase":name,"result":result},
            ensure_ascii=False,indent=2),encoding="utf-8")
    finally:
        runtime.close()


def main() -> None:
    package, audit = map(lambda p: Path(p).resolve(), sys.argv[1:3])
    if len(sys.argv) == 4:
        phase(package, audit, sys.argv[3])
        return
    audit.mkdir(parents=True, exist_ok=False)
    (audit / "workflows").mkdir()
    (audit / "skills").mkdir()
    (audit / "workflows/echo.workflow.json").write_text(CHILD,encoding="utf-8")
    (audit / "workflows/legacy.workflow").write_text("input x:String\nreturn x=input.x\n",encoding="utf-8")
    for name in ("register","restart","removed"):
        run = subprocess.run([sys.executable,str(Path(__file__).resolve()),str(package),str(audit),name],
            capture_output=True,text=True,encoding="utf-8",errors="replace")
        (audit / f"{name}.log").write_text(run.stdout+run.stderr,encoding="utf-8")
        if run.returncode:
            raise RuntimeError(f"Native audit phase {name} failed; see {audit / (name+'.log')}")
    failures = {
        "reject-invalid": {"echo.workflow.json":CHILD,"main.workflow.json":SOURCE.replace("30.text","999.missing")},
        "reject-duplicate": {"one.workflow.json":CHILD,"two.workflow.json":CHILD},
        "reject-cycle": {"main.workflow.json":SOURCE,"echo.workflow.json":CHILD.replace('"steps":[]',
            '"steps":[{"id":99,"call":"WorkflowRef_a-main","args":{"names":input.text,"selector":"a"}}]')},
    }
    for name, files in failures.items():
        case = audit / name
        (case / "workflows").mkdir(parents=True)
        (case / "skills").mkdir()
        for filename, source in files.items():
            (case / "workflows" / filename).write_text(source,encoding="utf-8")
        run = subprocess.run([sys.executable,str(Path(__file__).resolve()),str(package),str(case),name],
            capture_output=True,text=True,encoding="utf-8",errors="replace")
        (case / "output.log").write_text(run.stdout+run.stderr,encoding="utf-8")
        if run.returncode:
            raise RuntimeError(f"Invalid startup audit {name} failed; see {case / 'output.log'}")
    print(json.dumps({"passed":True,"package":str(package),"reports":str(audit)},ensure_ascii=False))


if __name__ == "__main__":
    main()
