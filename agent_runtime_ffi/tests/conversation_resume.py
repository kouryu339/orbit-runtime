"""Exercise pause/resume through a real DLL and a local, deterministic model.

Usage: python agent_runtime_ffi/tests/conversation_resume.py <dll>
"""

import argparse
import faulthandler
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "sdk/runtime/python"))
from corework_runtime import Runtime


class ModelFixture:
    def __init__(self):
        self.requests = []
        self.started = threading.Event()
        self.release = threading.Event()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                fixture.requests.append(request)
                first_request = len(fixture.requests) == 1
                if not first_request:
                    fixture.started.set()
                    if not fixture.release.wait(20):
                        self.send_error(504)
                        return
                message = {"role": "assistant", "content": "RESUME_FIXTURE_DONE"}
                if first_request:
                    message = {
                        "role": "assistant", "content": "",
                        "tool_calls": [{
                            "id": "resume-probe", "type": "function",
                            "function": {"name": "PlainRpcProbe", "arguments": '{"url":"https://fixture.invalid/"}'},
                        }],
                    }
                response = {
                    "id": "resume-fixture",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": message,
                        "finish_reason": "tool_calls" if first_request else "stop",
                    }],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12},
                }
                if request.get("stream"):
                    response["choices"][0]["delta"] = response["choices"][0].pop("message")
                    if first_request:
                        response["choices"][0]["delta"]["tool_calls"][0]["index"] = 0
                    body = ("data: " + json.dumps(response) + "\n\ndata: [DONE]\n\n").encode()
                    content_type = "text/event-stream"
                else:
                    body = json.dumps(response).encode()
                    content_type = "application/json"
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def wait_snapshot(runtime, conversation_id, predicate):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        event = runtime.next_event(100)
        if not isinstance(event, dict) or event.get("conversation_id") != conversation_id:
            continue
        if event.get("type") == "frontend:state_snapshot":
            snapshot = event["payload"]
            if predicate(snapshot):
                return snapshot
    raise AssertionError("Timed out waiting for the expected Runtime snapshot")


def run(dll):
    fixture = ModelFixture()
    runtime = None
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        rpc_address = f"127.0.0.1:{sock.getsockname()[1]}"
    sidecar = subprocess.Popen(
        ["node", str(REPO / "agent_runtime_ffi/tests/fixtures/rpc_ai_metadata.mjs"), rpc_address],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    try:
        ready = sidecar.stdout.readline()
        assert ready, sidecar.stderr.read()
        stats_url = json.loads(ready)["http"] + "/stats"
        with tempfile.TemporaryDirectory(prefix="orbit-resume-") as directory:
            root = Path(directory)
            role = root / "skills/role/resume-fixture"
            role.mkdir(parents=True)
            (role / "SKILL.md").write_text(
                "---\nname: resume-fixture\ndescription: Pause resume fixture\nkind: role\ntools: [PlainRpcProbe]\n---\n"
                "Answer the user's request.\n", encoding="utf-8",
            )
            runtime = Runtime(Path(dll), {
                "schema": "agent-runtime-create-options/v1", "data_dir": str(root / "data"),
                "language": "en", "restore_policy": "strict",
            })
            assert "conversation.resume" in runtime.capabilities["commands"]
            runtime.register_resources({
                "schema": "agent-runtime-resource-registration/v1", "id": "resume-fixture",
                "skills": {"root_dir": str(root / "skills")},
            })
            runtime.register_llm({
                "schema": "agent-runtime-llm-registration/v1", "id": "mock", "current_model_uid": 1001,
                "providers": [{
                    "id": 1, "name": "fixture", "type": "openai", "api_key": "fixture",
                    "base_url": f"http://127.0.0.1:{fixture.server.server_port}/v1",
                    "api_paradigm": "openai_chat_completions",
                    "enabled_models": [{"uid": 1001, "model_id": "gpt-4o"}],
                }],
            })
            runtime.start()
            runtime.add_rpc_endpoint({
                "id": "resume-fixture", "protocol": "grpc", "endpoint": "http://" + rpc_address,
                "timeout_ms": 3000,
            })
            runtime.add_agent_cluster({
                "schema": "agent-runtime-agent-cluster-registration/v1", "id": "resume-fixture",
                "description": "Pause resume regression", "focus_agent_id": "primary",
                "agents": [{"id": "primary", "role": "resume-fixture", "model_uid": 1001, "tool_protocol": "native_fc"}],
            })
            conversation_id = runtime.spawn_conversation(cluster_id="resume-fixture")["conversation_id"]
            payload = {"conversation_id": conversation_id}
            idle_resume = runtime.invoke("conversation.resume", payload)
            assert idle_resume["decision"] == "rejected", idle_resume
            assert idle_resume["reason"] == "not_paused", idle_resume
            runtime.send_message(conversation_id, "Remember ORIGINAL_RESUME_REQUEST and finish the task.")
            assert fixture.started.wait(10), "The model did not receive the first request"
            assert runtime.invoke("conversation.resume", payload)["decision"] == "rejected"
            assert runtime.invoke("conversation.pause", payload)["decision"] == "accepted"
            stopping = wait_snapshot(runtime, conversation_id, lambda s: s["conversation_state"] == "stopping")
            assert stopping["can_resume"] is False
            assert runtime.invoke("conversation.resume", payload)["decision"] == "rejected"
            fixture.release.set()
            paused = wait_snapshot(runtime, conversation_id, lambda s: s.get("can_resume") is True)
            assert paused["conversation_state"] == "waiting"
            assert len(fixture.requests) == 2
            accepted = runtime.invoke("conversation.resume", payload)
            assert accepted["decision"] == "accepted", accepted
            assert runtime.invoke("conversation.resume", payload)["decision"] == "rejected"
            wait_snapshot(runtime, conversation_id, lambda s: s["conversation_state"] == "thinking")
            finished = wait_snapshot(runtime, conversation_id, lambda s: s["conversation_state"] == "waiting" and not s["can_resume"])
            assert not finished.get("pending_permissions")
            assert len(fixture.requests) == 3, fixture.requests
            user_messages = [m for m in fixture.requests[-1]["messages"] if m["role"] == "user"]
            initial_user_messages = [m for m in fixture.requests[0]["messages"] if m["role"] == "user"]
            assert len(user_messages) == len(initial_user_messages), user_messages
            assert "ORIGINAL_RESUME_REQUEST" in str(user_messages[0])
            snapshot = runtime.invoke("conversation.export_snapshot", payload)
            ledger_users = [record for record in snapshot["ledger"] if record["role"] == "user"]
            assert len(ledger_users) == 1, ledger_users
            assert any("RESUME_FIXTURE_DONE" in str(record.get("content")) for record in snapshot["ledger"] if record["role"] == "assistant")
            assert len([m for m in fixture.requests[-1]["messages"] if m["role"] == "tool"]) == 1
            with urllib.request.urlopen(stats_url, timeout=5) as response:
                assert len(json.load(response)["executions"]) == 1, "Resume repeated the finished tool"
            assert runtime.invoke("conversation.resume", payload)["decision"] == "rejected"
            runtime.invoke("conversation.close", payload)
            try:
                runtime.invoke("conversation.resume", payload)
            except Exception:
                pass
            else:
                raise AssertionError("A closed conversation must reject resume")
            runtime.close()
            runtime = None
        print(json.dumps({"pause_resume": "passed", "model_requests": len(fixture.requests), "synthetic_user_messages": 0}))
    finally:
        fixture.close()
        if runtime is not None:
            runtime.close()
        sidecar.terminate()
        try:
            sidecar.wait(timeout=5)
        except subprocess.TimeoutExpired:
            sidecar.kill()
            sidecar.wait()


if __name__ == "__main__":
    faulthandler.dump_traceback_later(45, repeat=True)
    parser = argparse.ArgumentParser()
    parser.add_argument("dll")
    run(parser.parse_args().dll)
