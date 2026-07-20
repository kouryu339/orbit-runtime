from __future__ import annotations

import importlib
import inspect
import json
import os
import queue
import shlex
import sys
import tempfile
import threading
import uuid
from concurrent import futures
from dataclasses import dataclass
from enum import IntEnum
from pathlib import Path
from typing import Any, Callable

import grpc
from grpc_tools import protoc


SCHEMA = "corework-agent-tool/v1"


class ToolErrorCode(IntEnum):
    UNSPECIFIED = 0
    OK = 1
    INVALID_ARGUMENT = 100
    MISSING_ARGUMENT = 101
    PERMISSION_DENIED = 102
    NOT_FOUND = 103
    CONFLICT = 104
    INTERNAL = 200
    TIMEOUT = 201
    CANCELLED = 202
    UNAVAILABLE = 203
    HOST_CAPABILITY_DENIED = 300
    HOST_CAPABILITY_UNSUPPORTED = 301
    HOST_CALL_FAILED = 302
    PROTOCOL_ERROR = 400
    INVALID_OUTPUT = 401
    SCHEMA_MISMATCH = 402


@dataclass
class AIOutput:
    result: Any
    to_ai: str
    error_code: ToolErrorCode = ToolErrorCode.OK


@dataclass
class _RegisteredTool:
    metadata: dict[str, Any]
    handler: Callable[..., Any]


class ToolContext:
    def __init__(
        self,
        call_id: str,
        calls: queue.Queue[tuple[str, str, dict[str, Any], queue.Queue[Any]]],
        execute_request: Any | None = None,
    ) -> None:
        self.call_id = call_id
        self.tool_call_id = getattr(execute_request, "tool_call_id", "") if execute_request is not None else ""
        self.idempotency_key = getattr(execute_request, "idempotency_key", "") if execute_request is not None else ""
        self.session_id = getattr(execute_request, "session_id", "") if execute_request is not None else ""
        self.provider_id = getattr(execute_request, "provider_id", "") if execute_request is not None else ""
        self.cluster_id = getattr(execute_request, "cluster_id", "") if execute_request is not None else ""
        self.runtime_instance_id = getattr(execute_request, "runtime_instance_id", "") if execute_request is not None else ""
        self.conversation_id = getattr(execute_request, "conversation_id", "") if execute_request is not None else ""
        self.agent_id = getattr(execute_request, "agent_id", "") if execute_request is not None else ""
        self.turn_id = getattr(execute_request, "turn_id", "") if execute_request is not None else ""
        self.permissions = list(getattr(execute_request, "permissions", [])) if execute_request is not None else []
        self.host_context = _decode_host_context(getattr(execute_request, "host_context_json", "")) if execute_request is not None else None
        self._calls = calls

    def workspace_resolve_path(self, path: str) -> Any:
        return self._host_call("workspace.resolve_path", {"path": path})

    def workspace_resolve_working_path(self, path: str) -> Any:
        return self._host_call("workspace.resolve_working_path", {"path": path})

    def workspace_create_path(self, path: str) -> Any:
        return self._host_call("workspace.create_path", {"path": path})

    def workspace_create_working_path(self, path: str) -> Any:
        return self._host_call("workspace.create_working_path", {"path": path})

    def workspace_save_as_edited(self, source_path: str, suffix: str) -> Any:
        return self._host_call(
            "workspace.save_as_edited",
            {"source_path": source_path, "suffix": suffix},
        )

    def _host_call(self, op: str, args: dict[str, Any]) -> Any:
        result_queue: queue.Queue[Any] = queue.Queue(maxsize=1)
        call_id = uuid.uuid4().hex
        self._calls.put((call_id, op, args, result_queue))
        ok, value, code = result_queue.get()
        if ok:
            return value
        raise RuntimeError(f"host call {op} failed with code {code}: {value}")


_TOOLS: list[_RegisteredTool] = []


def register_tool(**metadata: Any) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    def decorator(func: Callable[..., Any]) -> Callable[..., Any]:
        if not metadata.get("name"):
            raise ValueError("tool metadata requires a non-empty name")
        _TOOLS.append(_RegisteredTool(metadata=dict(metadata), handler=func))
        return func

    return decorator


def serve(host: str = "127.0.0.1", port: int = 50051) -> None:
    pb2, pb2_grpc = _load_proto_modules()
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=16))
    pb2_grpc.add_AgentToolServiceServicer_to_server(_AgentToolService(pb2), server)
    server.add_insecure_port(f"{host}:{port}")
    server.start()
    try:
        server.wait_for_termination()
    except KeyboardInterrupt:
        server.stop(grace=1)


class _AgentToolService:
    def __init__(self, pb2: Any) -> None:
        self._pb2 = pb2

    def ListTools(self, request: Any, context: grpc.ServicerContext) -> Any:
        if request.accepted_schema and SCHEMA not in request.accepted_schema:
            context.abort(grpc.StatusCode.FAILED_PRECONDITION, f"unsupported schema; expected {SCHEMA}")
        return self._pb2.ListToolsResponse(
            schema=SCHEMA,
            tools=[self._tool_descriptor(tool.metadata) for tool in _TOOLS],
        )

    def Execute(self, request_iterator: Any, context: grpc.ServicerContext) -> Any:
        try:
            first = next(request_iterator)
        except StopIteration:
            yield self._tool_error("", "missing ExecuteRequest", ToolErrorCode.PROTOCOL_ERROR)
            return

        execute_request = first.execute_request
        if first.WhichOneof("message") != "execute_request" or not execute_request.tool_name:
            yield self._tool_error(first.call_id, "first stream message must be ExecuteRequest", ToolErrorCode.PROTOCOL_ERROR)
            return

        tool = _find_tool(execute_request.tool_name)
        if tool is None:
            yield self._tool_error(first.call_id, f"unknown tool {execute_request.tool_name}", ToolErrorCode.NOT_FOUND)
            return

        try:
            args = _request_args(tool.metadata, execute_request)
        except Exception as exc:
            yield self._tool_error(first.call_id, str(exc), ToolErrorCode.INVALID_ARGUMENT)
            return

        calls: queue.Queue[tuple[str, str, dict[str, Any], queue.Queue[Any]]] = queue.Queue()
        output_queue: queue.Queue[tuple[AIOutput | None, BaseException | None]] = queue.Queue(maxsize=1)
        tool_context = ToolContext(first.call_id, calls, execute_request)

        thread = threading.Thread(
            target=_run_handler,
            args=(tool.handler, tool_context, args, output_queue),
            daemon=True,
        )
        thread.start()

        while thread.is_alive() or not calls.empty():
            try:
                host_call_id, op, host_args, result_queue = calls.get(timeout=0.05)
            except queue.Empty:
                continue

            yield self._pb2.ToolStreamMessage(
                call_id=first.call_id,
                host_call=self._pb2.HostCall(
                    id=host_call_id,
                    op=op,
                    args_json=json.dumps(host_args, ensure_ascii=False),
                ),
            )

            result_message = next(request_iterator)
            if result_message.call_id != first.call_id or result_message.WhichOneof("message") != "host_result":
                result_queue.put((False, "invalid HostResult message", int(ToolErrorCode.PROTOCOL_ERROR)))
                continue
            host_result = result_message.host_result
            if host_result.id != host_call_id:
                result_queue.put((False, f"HostResult id mismatch: {host_result.id}", int(ToolErrorCode.PROTOCOL_ERROR)))
                continue
            try:
                value = json.loads(host_result.value_json or "null")
            except Exception:
                value = host_result.value_json
            result_queue.put((host_result.ok, value, host_result.code))

        output, error = output_queue.get()
        if error is not None:
            yield self._tool_error(first.call_id, str(error), ToolErrorCode.INTERNAL)
            return

        assert output is not None
        if not output.to_ai or not output.to_ai.strip():
            yield self._tool_error(first.call_id, "AIOutput.to_ai must be non-empty", ToolErrorCode.INVALID_OUTPUT)
            return

        yield self._pb2.ToolStreamMessage(
            call_id=first.call_id,
            ai_output=self._pb2.AIOutput(
                result_json=json.dumps(output.result, ensure_ascii=False),
                to_ai=output.to_ai,
                error_code=int(output.error_code),
            ),
        )

    def _tool_descriptor(self, metadata: dict[str, Any]) -> Any:
        return self._pb2.ToolDescriptor(
            name=metadata["name"],
            description=metadata.get("description", ""),
            parameters=[self._tool_parameter(item) for item in metadata.get("parameters", [])],
            outputs=[self._tool_output(item) for item in metadata.get("outputs", [])],
            destructive=bool(metadata.get("destructive", False)),
            readonly=bool(metadata.get("readonly", False)),
            idempotent=bool(metadata.get("idempotent", False)),
            open_world=bool(metadata.get("open_world", False)),
            secret=bool(metadata.get("secret", False)),
            category=metadata.get("category", ""),
            display_name=metadata.get("display_name", ""),
            required_capabilities=list(metadata.get("required_capabilities", [])),
        )

    def _tool_parameter(self, item: dict[str, Any]) -> Any:
        kwargs = {
            "name": item.get("name", ""),
            "param_type": item.get("param_type", ""),
            "required": bool(item.get("required", False)),
            "description": item.get("description", ""),
        }
        if item.get("default_value") is not None:
            kwargs["default_value"] = str(item["default_value"])
        return self._pb2.ToolParameter(**kwargs)

    def _tool_output(self, item: dict[str, Any]) -> Any:
        return self._pb2.ToolOutputField(
            name=item.get("name", ""),
            field_type=item.get("field_type", ""),
            description=item.get("description", ""),
        )

    def _tool_error(self, call_id: str, message: str, code: ToolErrorCode) -> Any:
        return self._pb2.ToolStreamMessage(
            call_id=call_id,
            error=self._pb2.ToolError(message=message, code=int(code)),
        )


def _run_handler(
    handler: Callable[..., Any],
    ctx: ToolContext,
    args: dict[str, Any],
    output_queue: queue.Queue[tuple[AIOutput | None, BaseException | None]],
) -> None:
    try:
        result = handler(ctx, **args)
        if inspect.isawaitable(result):
            import asyncio

            result = asyncio.run(result)
        if not isinstance(result, AIOutput):
            raise TypeError("tool handler must return AIOutput")
        output_queue.put((result, None))
    except BaseException as exc:
        output_queue.put((None, exc))


def _find_tool(name: str) -> _RegisteredTool | None:
    for tool in _TOOLS:
        if tool.metadata.get("name") == name:
            return tool
    return None


def _request_args(metadata: dict[str, Any], execute_request: Any) -> dict[str, Any]:
    args_json = (execute_request.args_json or "").strip()
    if args_json:
        args = json.loads(args_json)
        if not isinstance(args, dict):
            raise ValueError("args_json must encode an object")
        return args

    args_cli = (execute_request.args_cli or "").strip()
    if args_cli:
        return _args_from_cli(metadata, execute_request.tool_name, args_cli)

    return {}


def _args_from_cli(metadata: dict[str, Any], tool_name: str, args_cli: str) -> dict[str, Any]:
    parsed = _parse_cli_args(args_cli)
    args: dict[str, Any] = {}
    for parameter in metadata.get("parameters", []):
        name = parameter.get("name")
        if not name:
            continue
        if name in parsed:
            args[name] = parsed[name]
        elif parameter.get("default_value") is not None:
            args[name] = parameter.get("default_value")
        elif parameter.get("required"):
            raise ValueError(f"missing required argument '{name}' for tool '{tool_name}'")
    return args


def _parse_cli_args(args_cli: str) -> dict[str, str]:
    tokens = shlex.split(args_cli, posix=False)
    args: dict[str, str] = {}
    i = 0
    while i < len(tokens):
        token = tokens[i]
        if not token.startswith("--") or len(token) <= 2:
            i += 1
            continue
        key_value = token[2:]
        if "=" in key_value:
            key, value = key_value.split("=", 1)
        elif i + 1 < len(tokens) and not tokens[i + 1].startswith("--"):
            key = key_value
            i += 1
            value = tokens[i]
        else:
            key = key_value
            value = "true"
        args[key] = _strip_quotes(value)
        i += 1
    return args


def _strip_quotes(value: str) -> str:
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
        return value[1:-1]
    return value


def _decode_host_context(raw: str) -> Any:
    if not raw:
        return None
    try:
        return json.loads(raw)
    except Exception:
        return raw


def _load_proto_modules() -> tuple[Any, Any]:
    generated_dir = Path(tempfile.gettempdir()) / "corework_agent_tool_pb"
    generated_dir.mkdir(parents=True, exist_ok=True)
    proto_path = _proto_path()
    pb2_file = generated_dir / "corework_agent_tool_v1_pb2.py"
    pb2_grpc_file = generated_dir / "corework_agent_tool_v1_pb2_grpc.py"

    proto_mtime = proto_path.stat().st_mtime
    generated_is_stale = (
        not pb2_file.exists()
        or not pb2_grpc_file.exists()
        or pb2_file.stat().st_mtime < proto_mtime
        or pb2_grpc_file.stat().st_mtime < proto_mtime
    )

    if generated_is_stale:
        result = protoc.main(
            [
                "grpc_tools.protoc",
                f"-I{proto_path.parent}",
                f"--python_out={generated_dir}",
                f"--grpc_python_out={generated_dir}",
                str(proto_path),
            ]
        )
        if result != 0:
            raise RuntimeError(f"failed to compile Corework agent tool proto: {proto_path}")

    if str(generated_dir) not in sys.path:
        sys.path.insert(0, str(generated_dir))
    pb2 = importlib.import_module("corework_agent_tool_v1_pb2")
    pb2_grpc = importlib.import_module("corework_agent_tool_v1_pb2_grpc")
    return pb2, pb2_grpc


def _proto_path() -> Path:
    env_path = os.environ.get("COREWORK_AGENT_TOOL_PROTO")
    if env_path:
        return Path(env_path)
    repo_proto = Path(__file__).resolve().parents[4] / "corework" / "proto" / "corework_agent_tool_v1.proto"
    if repo_proto.exists():
        return repo_proto
    raise RuntimeError("Corework proto not found; set COREWORK_AGENT_TOOL_PROTO")


__all__ = [
    "AIOutput",
    "ToolContext",
    "ToolErrorCode",
    "register_tool",
    "serve",
]
