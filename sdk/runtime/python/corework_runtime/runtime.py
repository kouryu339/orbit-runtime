from __future__ import annotations

import ctypes
import json
import queue
import threading
from collections.abc import Callable, Iterator, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ABI_VERSION = 1
DEFAULT_RUNTIME_VERSION = "0.4.5"
CONVERSATION_CREATED_EVENT_TYPE = "conversation:created"
CONVERSATION_CLOSED_EVENT_TYPE = "conversation:closed"
FRONTEND_STATE_SNAPSHOT_EVENT_TYPE = "frontend:state_snapshot"
LEDGER_DELTA_EVENT_TYPE = "conversation.ledger_delta"
LEDGER_DELTA_SCHEMA = "agent-runtime-ledger-delta/v1"
STATE_DELTA_EVENT_TYPE = "conversation.state_delta"
STATE_DELTA_SCHEMA = "agent-runtime-state-delta/v1"
WORKFLOW_RESOURCE_CHANGED_EVENT_TYPE = "workflow.resource_changed"
WORKFLOW_EXECUTION_COMPLETED_EVENT_TYPE = "workflow.execution_completed"
PUBLIC_RUNTIME_EVENT_TYPES = frozenset(
    (
        CONVERSATION_CREATED_EVENT_TYPE,
        CONVERSATION_CLOSED_EVENT_TYPE,
        LEDGER_DELTA_EVENT_TYPE,
        STATE_DELTA_EVENT_TYPE,
        FRONTEND_STATE_SNAPSHOT_EVENT_TYPE,
        WORKFLOW_RESOURCE_CHANGED_EVENT_TYPE,
        WORKFLOW_EXECUTION_COMPLETED_EVENT_TYPE,
    )
)

_OK = 0
_ERR_BAD_STATE = 3
_ERR_TIMEOUT = 4
_ERR_UNSUPPORTED = 5


class RuntimeError(Exception):
    def __init__(self, code: int, error: Mapping[str, Any] | str):
        self.code = code
        self.error = error
        message = error.get("message", str(error)) if isinstance(error, Mapping) else error
        super().__init__(f"runtime error {code}: {message}")


class RuntimeStateError(RuntimeError):
    pass


class RuntimeTimeout(RuntimeError):
    pass


class UnsupportedCommand(RuntimeError):
    pass


def ledger_delta_from_event(event: Mapping[str, Any] | str) -> Mapping[str, Any] | None:
    """Return the ledger delta payload from a runtime event, if present.

    Hosts can persist this payload to JSONL, upsert it into a database using
    (conversation_id, record_id), or ignore it when runtime-ledger replication
    is not needed.
    """

    if not isinstance(event, Mapping):
        return None
    if event.get("type") != LEDGER_DELTA_EVENT_TYPE:
        return None
    payload = event.get("payload")
    if not isinstance(payload, Mapping):
        return None
    if payload.get("schema") != LEDGER_DELTA_SCHEMA:
        return None
    return payload


def is_ledger_delta_event(event: Mapping[str, Any] | str) -> bool:
    return ledger_delta_from_event(event) is not None


def state_delta_from_event(event: Mapping[str, Any] | str) -> Mapping[str, Any] | None:
    """Return the internal runtime-state delta payload from an event, if present."""

    if not isinstance(event, Mapping):
        return None
    if event.get("type") != STATE_DELTA_EVENT_TYPE:
        return None
    payload = event.get("payload")
    if not isinstance(payload, Mapping):
        return None
    if payload.get("schema") != STATE_DELTA_SCHEMA:
        return None
    return payload


def is_state_delta_event(event: Mapping[str, Any] | str) -> bool:
    return state_delta_from_event(event) is not None


def is_public_runtime_event(event: Mapping[str, Any] | str) -> bool:
    """Return whether an event is part of the stable host-facing event stream.

    Internal diagnostics, studio/test events, and provider-specific telemetry are
    intentionally not exposed through the SDK event bus. Hosts that need raw FFI
    events can still poll ``Runtime.next_event`` directly.
    """

    if not isinstance(event, Mapping):
        return False
    return event.get("type") in PUBLIC_RUNTIME_EVENT_TYPES


def conversation_id_from_event(event: Mapping[str, Any] | str) -> str | None:
    if not isinstance(event, Mapping):
        return None
    direct = event.get("conversation_id")
    if isinstance(direct, str) and direct:
        return direct
    payload = event.get("payload")
    if isinstance(payload, Mapping):
        nested = payload.get("conversation_id")
        if isinstance(nested, str) and nested:
            return nested
    return None


def workflow_id_from_event(event: Mapping[str, Any] | str) -> str | None:
    if not isinstance(event, Mapping):
        return None
    payload = event.get("payload")
    if not isinstance(payload, Mapping):
        return None
    event_line = event.get("event_line", payload.get("event_line"))
    workflow_id = payload.get("workflow_id")
    if event_line == "workflow" and isinstance(workflow_id, str) and workflow_id:
        return workflow_id
    return None


def is_workflow_event(event: Mapping[str, Any] | str) -> bool:
    if not isinstance(event, Mapping):
        return False
    payload = event.get("payload")
    nested_line = payload.get("event_line") if isinstance(payload, Mapping) else None
    return event.get("event_line", nested_line) == "workflow"


@dataclass
class RuntimeDiagnostic:
    kind: str
    message: str
    event: Mapping[str, Any] | str | None = None
    error: BaseException | None = None


class RuntimeEventSubscription:
    def __init__(
        self,
        bus: "RuntimeEventBus",
        event_filter: "RuntimeEventFilter | None",
        maxsize: int,
    ):
        self._bus = bus
        self._event_filter = event_filter
        self._queue: queue.Queue[Mapping[str, Any] | str | None] = queue.Queue(maxsize)
        self._closed = False

    def _offer(self, event: Mapping[str, Any] | str) -> bool:
        if self._closed:
            return True
        if self._event_filter is not None and not self._event_filter(event):
            return True
        try:
            self._queue.put_nowait(event)
            return True
        except queue.Full:
            return False

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._bus.unsubscribe(self)
        try:
            self._queue.put_nowait(None)
        except queue.Full:
            pass

    def next(self, timeout: float | None = None) -> Mapping[str, Any] | str | None:
        item = self._queue.get(timeout=timeout)
        if item is None:
            self.close()
        return item

    def __iter__(self) -> Iterator[Mapping[str, Any] | str]:
        while True:
            item = self.next()
            if item is None:
                return
            yield item

    def __enter__(self) -> "RuntimeEventSubscription":
        return self

    def __exit__(self, exc_type: Any, exc: Any, traceback: Any) -> None:
        self.close()


RuntimeEventFilter = Callable[[Mapping[str, Any] | str], bool]


class RuntimeEventBus:
    """Small in-process fanout bus for the public Runtime event stream."""

    def __init__(self, *, public_only: bool = True):
        self._public_only = public_only
        self._lock = threading.RLock()
        self._subscribers: list[RuntimeEventSubscription] = []
        self._closed = False

    def subscribe(
        self,
        event_filter: RuntimeEventFilter | None = None,
        *,
        maxsize: int = 1000,
    ) -> RuntimeEventSubscription:
        subscription = RuntimeEventSubscription(self, event_filter, maxsize)
        with self._lock:
            if self._closed:
                subscription.close()
            else:
                self._subscribers.append(subscription)
        return subscription

    def unsubscribe(self, subscription: RuntimeEventSubscription) -> None:
        with self._lock:
            self._subscribers = [
                item for item in self._subscribers if item is not subscription
            ]

    def publish(self, event: Mapping[str, Any] | str) -> None:
        if self._public_only and not is_public_runtime_event(event):
            return
        with self._lock:
            subscribers = list(self._subscribers)
        dropped = [item for item in subscribers if not item._offer(event)]
        for item in dropped:
            item.close()

    def close(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._closed = True
            subscribers = list(self._subscribers)
            self._subscribers.clear()
        for item in subscribers:
            item.close()


@dataclass
class RuntimeEventPump:
    runtime: "Runtime"
    bus: RuntimeEventBus
    timeout_ms: int = 250
    diagnostics: Any | None = None
    _stop: threading.Event = field(default_factory=threading.Event, init=False)
    _thread: threading.Thread | None = field(default=None, init=False)

    def start(self) -> "RuntimeEventPump":
        if self._thread is not None:
            return self
        self._thread = threading.Thread(
            target=self._run,
            name="corework-runtime-event-pump",
            daemon=True,
        )
        self._thread.start()
        return self

    def stop(self, timeout: float | None = 2.0) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=timeout)
        self.bus.close()

    def join(self, timeout: float | None = None) -> None:
        if self._thread is not None:
            self._thread.join(timeout=timeout)

    def _emit_diagnostic(self, diagnostic: RuntimeDiagnostic) -> None:
        if self.diagnostics is None:
            return
        self.diagnostics(diagnostic)

    def _run(self) -> None:
        try:
            while not self._stop.is_set():
                try:
                    event = self.runtime.next_event(self.timeout_ms)
                except RuntimeTimeout:
                    continue
                except RuntimeStateError:
                    break
                except BaseException as exc:
                    self._emit_diagnostic(
                        RuntimeDiagnostic(
                            kind="event_pump_error",
                            message=str(exc),
                            error=exc,
                        )
                    )
                    break
                if event is not None:
                    self.bus.publish(event)
        finally:
            self.bus.close()


class ConversationPosition:
    CURRENT = "current"
    BACKGROUND = "background"
    DATA_ONLY = "data_only"


@dataclass
class ConversationEntry:
    conversation_id: str
    position: str = ConversationPosition.BACKGROUND
    waiting: bool = False


@dataclass
class ConversationRegistryAction:
    kind: str
    conversation_id: str


class ConversationRegistry:
    """Optional host helper for current/background conversation ownership."""

    def __init__(self):
        self._lock = threading.RLock()
        self._entries: dict[str, ConversationEntry] = {}
        self._current_id: str | None = None

    def track(
        self,
        conversation_id: str,
        *,
        position: str = ConversationPosition.BACKGROUND,
    ) -> None:
        with self._lock:
            self._entries[conversation_id] = ConversationEntry(
                conversation_id=conversation_id,
                position=position,
            )
            if position == ConversationPosition.CURRENT:
                self._current_id = conversation_id

    def set_current(self, conversation_id: str | None) -> list[ConversationRegistryAction]:
        actions: list[ConversationRegistryAction] = []
        with self._lock:
            previous = self._current_id
            if previous and previous in self._entries:
                entry = self._entries[previous]
                entry.position = ConversationPosition.BACKGROUND
                if entry.waiting:
                    actions.append(
                        ConversationRegistryAction("close_background", previous)
                    )
            self._current_id = conversation_id
            if conversation_id:
                self.track(
                    conversation_id,
                    position=ConversationPosition.CURRENT,
                )
        return actions

    def observe_event(
        self,
        event: Mapping[str, Any] | str,
    ) -> list[ConversationRegistryAction]:
        event_type = event.get("type") if isinstance(event, Mapping) else None
        conversation_id = conversation_id_from_event(event)
        if not conversation_id:
            return []
        with self._lock:
            if event_type == CONVERSATION_CREATED_EVENT_TYPE:
                self._entries.setdefault(
                    conversation_id,
                    ConversationEntry(conversation_id=conversation_id),
                )
                return []
            if event_type == CONVERSATION_CLOSED_EVENT_TYPE:
                self._entries.pop(conversation_id, None)
                if self._current_id == conversation_id:
                    self._current_id = None
                return []
            if event_type == STATE_DELTA_EVENT_TYPE:
                entry = self._entries.get(conversation_id)
                if entry is not None:
                    entry.waiting = _event_marks_waiting(event)
                    if (
                        entry.waiting
                        and entry.position == ConversationPosition.BACKGROUND
                    ):
                        return [
                            ConversationRegistryAction(
                                "close_background",
                                conversation_id,
                            )
                        ]
        return []


def _event_marks_waiting(event: Mapping[str, Any] | str) -> bool:
    payload = event.get("payload") if isinstance(event, Mapping) else None
    if not isinstance(payload, Mapping):
        return False
    status = payload.get("status") or payload.get("state")
    return status == "waiting"


def _read_string(pointer: int | None) -> str:
    if not pointer:
        return ""
    value = ctypes.cast(pointer, ctypes.c_char_p).value
    return value.decode("utf-8", errors="replace") if value else ""


class Runtime:
    """Owns one Agent Runtime ABI 1 handle.

    Calls on one handle are serialized by the native runtime. This wrapper adds
    a host-side lock so close cannot race a Python invoke or event poll.
    """

    def __init__(
        self,
        library_path: str | Path,
        create_options: str | Mapping[str, Any] | None = None,
        *,
        required_commands: tuple[str, ...] = (),
    ):
        self._library_path = Path(library_path)
        self._lib = ctypes.CDLL(str(self._library_path))
        self._handle = ctypes.c_uint64(0)
        self._lock = threading.RLock()
        self._closed = False
        self._bind()

        actual_abi = int(self._lib.agent_runtime_abi_version_v1())
        if actual_abi != ABI_VERSION:
            raise RuntimeError(
                0,
                f"incompatible runtime ABI: expected {ABI_VERSION}, got {actual_abi}",
            )

        self.version = _read_string(self._lib.agent_runtime_version_v1())
        self.capabilities = json.loads(
            _read_string(self._lib.agent_runtime_capabilities_v1())
        )
        supported = set(self.capabilities.get("commands") or ())
        missing = sorted(set(required_commands) - supported)
        if missing:
            raise UnsupportedCommand(
                _ERR_UNSUPPORTED,
                f"runtime does not support required commands: {', '.join(missing)}",
            )

        if create_options is None:
            create_options_json = ""
        elif isinstance(create_options, Mapping):
            create_options_json = json.dumps(create_options, ensure_ascii=False)
        else:
            create_options_json = str(create_options)
        code = self._lib.agent_runtime_create_v1(
            create_options_json.encode("utf-8"), ctypes.byref(self._handle)
        )
        self._raise_for_code(code)

    @classmethod
    def from_release(
        cls,
        create_options: str | Mapping[str, Any] | None = None,
        *,
        version: str = DEFAULT_RUNTIME_VERSION,
        cache_dir: str | Path | None = None,
        platform_id: str | None = None,
        force_download: bool = False,
        required_commands: tuple[str, ...] = (),
    ) -> "Runtime":
        from .native import ensure_runtime_library

        library_path = ensure_runtime_library(
            version=version,
            cache_dir=cache_dir,
            platform_id=platform_id,
            force=force_download,
        )
        return cls(
            library_path,
            create_options,
            required_commands=required_commands,
        )

    def _bind(self) -> None:
        handle = ctypes.c_uint64
        string_out = ctypes.POINTER(ctypes.c_void_p)

        self._lib.agent_runtime_abi_version_v1.argtypes = []
        self._lib.agent_runtime_abi_version_v1.restype = ctypes.c_uint32
        self._lib.agent_runtime_version_v1.argtypes = []
        self._lib.agent_runtime_version_v1.restype = ctypes.c_void_p
        self._lib.agent_runtime_capabilities_v1.argtypes = []
        self._lib.agent_runtime_capabilities_v1.restype = ctypes.c_void_p
        self._lib.agent_runtime_create_v1.argtypes = [
            ctypes.c_char_p,
            ctypes.POINTER(handle),
        ]
        self._lib.agent_runtime_create_v1.restype = ctypes.c_int
        self._lib.agent_runtime_start_v1.argtypes = [handle]
        self._lib.agent_runtime_start_v1.restype = ctypes.c_int
        self._lib.agent_runtime_invoke_v1.argtypes = [
            handle,
            ctypes.c_char_p,
            string_out,
        ]
        self._lib.agent_runtime_invoke_v1.restype = ctypes.c_int
        self._lib.agent_runtime_next_event_v1.argtypes = [
            handle,
            ctypes.c_uint32,
            string_out,
        ]
        self._lib.agent_runtime_next_event_v1.restype = ctypes.c_int
        self._lib.agent_runtime_shutdown_v1.argtypes = [handle, ctypes.c_uint32]
        self._lib.agent_runtime_shutdown_v1.restype = ctypes.c_int
        self._lib.agent_runtime_destroy_v1.argtypes = [handle]
        self._lib.agent_runtime_destroy_v1.restype = ctypes.c_int
        self._lib.agent_runtime_last_error_json_v1.argtypes = []
        self._lib.agent_runtime_last_error_json_v1.restype = ctypes.c_void_p
        self._lib.agent_runtime_free_string_v1.argtypes = [ctypes.c_void_p]
        self._lib.agent_runtime_free_string_v1.restype = None

    def _last_error(self) -> Mapping[str, Any] | str:
        text = _read_string(self._lib.agent_runtime_last_error_json_v1())
        if not text:
            return "runtime call failed without an error document"
        try:
            value = json.loads(text)
            return value if isinstance(value, Mapping) else text
        except json.JSONDecodeError:
            return text

    def _raise_for_code(
        self,
        code: int,
        error: Mapping[str, Any] | str | None = None,
    ) -> None:
        if code == _OK:
            return
        error = error or self._last_error()
        if code == _ERR_TIMEOUT:
            raise RuntimeTimeout(code, error)
        if code == _ERR_BAD_STATE:
            raise RuntimeStateError(code, error)
        if code == _ERR_UNSUPPORTED:
            raise UnsupportedCommand(code, error)
        raise RuntimeError(code, error)

    def _ensure_open(self) -> None:
        if self._closed or not self._handle.value:
            raise RuntimeStateError(_ERR_BAD_STATE, "runtime is closed")

    def start(self) -> None:
        with self._lock:
            self._ensure_open()
            self._raise_for_code(self._lib.agent_runtime_start_v1(self._handle))

    def invoke(
        self,
        command_type: str,
        payload: Mapping[str, Any] | None = None,
        *,
        request_id: str | None = None,
        command_id: str | None = None,
    ) -> Any:
        request: dict[str, Any] = {
            "schema": "agent-runtime-command/v1",
            "type": command_type,
            "payload": dict(payload or {}),
        }
        if request_id is not None:
            request["id"] = request_id
        if command_id is not None:
            request["command_id"] = command_id

        with self._lock:
            self._ensure_open()
            output = ctypes.c_void_p()
            code = self._lib.agent_runtime_invoke_v1(
                self._handle,
                json.dumps(request, ensure_ascii=False).encode("utf-8"),
                ctypes.byref(output),
            )
            envelope: Mapping[str, Any] = {}
            if output.value:
                try:
                    decoded = json.loads(_read_string(output.value))
                    if isinstance(decoded, Mapping):
                        envelope = decoded
                finally:
                    self._lib.agent_runtime_free_string_v1(output)
            self._raise_for_code(code, envelope.get("error"))
            return envelope.get("result")

    def register_resources_file(self, path: str | Path) -> Any:
        return self.invoke("runtime.register_resources", {"input": str(path)})

    def register_resources(self, registration: Mapping[str, Any]) -> Any:
        return self.invoke("runtime.register_resources", {"registration": registration})

    def create_workflow_draft(self, resource: Mapping[str, Any]) -> Any:
        return self.invoke("workflow.create", {"resource": dict(resource)})

    def read_workflow(self, workflow_id: str) -> Any:
        return self.invoke("workflow.read", {"id": workflow_id})

    def register_workflow_draft(
        self,
        workflow_id: str,
        *,
        expected_revision: int | None = None,
        name: str | None = None,
    ) -> Any:
        return self.invoke(
            "workflow.register",
            {
                "id": workflow_id,
                "expected_revision": expected_revision,
                "name": name,
            },
        )

    def update_workflow(
        self,
        resource: Mapping[str, Any],
        *,
        expected_revision: int | None = None,
    ) -> Any:
        return self.invoke(
            "workflow.update",
            {"resource": dict(resource), "expected_revision": expected_revision},
        )

    def compile_workflow_draft(self, workflow_id: str) -> Any:
        return self.invoke("workflow.compile", {"id": workflow_id})

    def workflow_script_to_blueprint(self, script: str) -> Any:
        return self.invoke(
            "workflow.convert.script_to_blueprint", {"script": script}
        )

    def workflow_blueprint_to_script(self, blueprint: Mapping[str, Any]) -> Any:
        return self.invoke(
            "workflow.convert.blueprint_to_script",
            {"blueprint": dict(blueprint)},
        )

    def delete_workflow(
        self, workflow_id: str, *, expected_revision: int | None = None
    ) -> Any:
        return self.invoke(
            "workflow.delete",
            {"id": workflow_id, "expected_revision": expected_revision},
        )

    def list_workflows(self, *, kind: str | None = None) -> Any:
        return self.invoke("workflow.list", {"kind": kind})

    def execute_workflow(
        self,
        workflow_id: str,
        inputs: Mapping[str, Any] | None = None,
        *,
        trace: bool = False,
    ) -> Any:
        return self.invoke(
            "workflow.execute",
            {
                "id": workflow_id,
                "inputs": dict(inputs or {}),
                "trace": trace,
            },
        )

    def test_workflow_draft(
        self,
        workflow_id: str,
        inputs: Mapping[str, Any] | None = None,
        *,
        trace: bool = False,
    ) -> Any:
        return self.invoke(
            "workflow.execute",
            {
                "id": workflow_id,
                "mode": "test",
                "inputs": dict(inputs or {}),
                "trace": trace,
            },
        )

    def execute_workflow_script(
        self,
        script: str,
        inputs: Mapping[str, Any] | None = None,
        *,
        trace: bool = False,
    ) -> Any:
        return self.invoke(
            "workflow.execute_script",
            {"script": script, "inputs": dict(inputs or {}), "trace": trace},
        )

    def register_llm_file(self, path: str | Path) -> Any:
        return self.invoke("runtime.register_llm", {"input": str(path)})

    def register_llm(self, registration: Mapping[str, Any]) -> Any:
        return self.invoke("runtime.register_llm", {"registration": registration})

    def reload_llm_file(self, path: str | Path) -> Any:
        return self.invoke("runtime.reload_llm", {"input": str(path)})

    def reload_llm(self, registration: Mapping[str, Any]) -> Any:
        return self.invoke("runtime.reload_llm", {"registration": registration})

    def register_agent_cluster_file(self, path: str | Path) -> Any:
        return self.invoke("runtime.register_agent_cluster", {"input": str(path)})

    def register_agent_cluster(self, registration: Mapping[str, Any]) -> Any:
        return self.invoke(
            "runtime.register_agent_cluster",
            {"registration": registration},
        )

    def set_current_model(self, model_uid: int) -> Any:
        return self.invoke("runtime.set_current_model", {"model_uid": model_uid})

    def provider_definitions(self) -> Any:
        return self.invoke("runtime.get_provider_definitions")

    def tool_definitions(self) -> Any:
        """Return Runtime-registered local and RPC tool definitions."""
        return self.invoke("runtime.get_tool_definitions")

    def workflow_node_definitions(self) -> Any:
        """Return the unified Corework, local-tool, and RPC workflow node catalog."""
        return self.invoke("runtime.get_workflow_node_definitions")

    def agent_cluster_definitions(self) -> Any:
        """Return effective registered and built-in Agent cluster definitions."""
        return self.invoke("runtime.get_agent_cluster_definitions")

    def rpc_endpoint_definitions(self) -> Any:
        """Return sanitized RPC endpoint registration and startup state."""
        return self.invoke("runtime.get_rpc_endpoint_definitions")

    def spawn_conversation(
        self,
        *,
        cluster_id: str,
        tool_host_context: Mapping[str, Any] | None = None,
        permissions: Mapping[str, str] | None = None,
        **extra: Any,
    ) -> Any:
        payload: dict[str, Any] = {
            "schema": "agent-runtime-conversation-spawn/v1",
            "cluster_id": cluster_id,
        }
        if tool_host_context is not None:
            payload["tool_host_context"] = dict(tool_host_context)
        if permissions is not None:
            payload["permissions"] = dict(permissions)
        payload.update(extra)
        return self.invoke("conversation.spawn", payload)

    def send_message(self, conversation_id: str, content: str) -> Any:
        return self.invoke(
            "conversation.send_message",
            {"conversation_id": conversation_id, "content": content},
        )

    def pause_conversation(self, conversation_id: str) -> Any:
        return self.invoke("conversation.pause", {"conversation_id": conversation_id})

    def close_conversation(self, conversation_id: str) -> Any:
        return self.invoke("conversation.close", {"conversation_id": conversation_id})

    def export_snapshot(self) -> Any:
        return self.invoke("runtime.export_snapshot")

    def export_conversation_snapshot(
        self,
        conversation_id: str,
        options: Mapping[str, Any] | None = None,
    ) -> Any:
        return self.invoke(
            "conversation.export_snapshot",
            {"conversation_id": conversation_id, "options": dict(options or {})},
        )

    def spawn_conversation_from_snapshot(
        self,
        *,
        spawn: Mapping[str, Any],
        snapshot: Mapping[str, Any],
    ) -> Any:
        return self.invoke(
            "conversation.spawn_from_snapshot",
            {"spawn": dict(spawn), "snapshot": dict(snapshot)},
        )

    def import_conversation_snapshot(
        self,
        snapshot: Mapping[str, Any],
        options: Mapping[str, Any] | None = None,
    ) -> Any:
        return self.invoke(
            "conversation.import_snapshot",
            {"snapshot": dict(snapshot), "options": dict(options or {})},
        )

    def materialize_conversation(
        self,
        conversation_id: str,
        options: Mapping[str, Any] | None = None,
    ) -> Any:
        return self.invoke(
            "conversation.materialize",
            {"conversation_id": conversation_id, "options": dict(options or {})},
        )

    def set_dynamic_snapshot(
        self,
        conversation_id: str,
        agent_id: str,
        field_name: str,
        text: str,
    ) -> Any:
        return self.invoke(
            "conversation.set_dynamic_snapshot",
            {
                "conversation_id": conversation_id,
                "agent_id": agent_id,
                "field_name": field_name,
                "text": text,
            },
        )

    def resolve_tool_permission(
        self,
        conversation_id: str,
        tool_call_id: str,
        decision: str,
    ) -> Any:
        return self.invoke(
            "conversation.resolve_tool_permission",
            {
                "conversation_id": conversation_id,
                "tool_call_id": tool_call_id,
                "decision": decision,
            },
        )

    def agent_tasks(self, conversation_id: str) -> Any:
        return self.invoke("conversation.agent_tasks", {"conversation_id": conversation_id})

    def set_summary_model(self, conversation_id: str, model_name: str) -> Any:
        return self.invoke(
            "conversation.set_summary_model",
            {"conversation_id": conversation_id, "model_name": model_name},
        )

    def compact_history(
        self,
        conversation_id: str,
        agent_ids: list[str] | None = None,
    ) -> Any:
        return self.invoke(
            "conversation.compact_history",
            {"conversation_id": conversation_id, "agent_ids": list(agent_ids or [])},
        )

    def next_event(self, timeout_ms: int = 0) -> Mapping[str, Any] | str | None:
        if timeout_ms < 0 or timeout_ms > 0xFFFFFFFF:
            raise ValueError("timeout_ms must be between 0 and 4294967295")
        with self._lock:
            self._ensure_open()
            output = ctypes.c_void_p()
            code = self._lib.agent_runtime_next_event_v1(
                self._handle,
                timeout_ms,
                ctypes.byref(output),
            )
            if code == _ERR_TIMEOUT:
                return None
            self._raise_for_code(code)
            try:
                text = _read_string(output.value)
                try:
                    value = json.loads(text)
                    return value if isinstance(value, Mapping) else text
                except json.JSONDecodeError:
                    return text
            finally:
                if output.value:
                    self._lib.agent_runtime_free_string_v1(output)

    def events(
        self,
        *,
        timeout_ms: int = 250,
        stop: threading.Event | None = None,
    ) -> Iterator[Mapping[str, Any] | str]:
        while stop is None or not stop.is_set():
            event = self.next_event(timeout_ms)
            if event is not None:
                yield event

    def shutdown(self, timeout_ms: int = 10_000) -> None:
        with self._lock:
            if self._closed:
                return
            self._ensure_open()
            code = self._lib.agent_runtime_shutdown_v1(self._handle, timeout_ms)
            self._raise_for_code(code)

    def destroy(self) -> None:
        with self._lock:
            if self._closed:
                return
            self._ensure_open()
            self._raise_for_code(self._lib.agent_runtime_destroy_v1(self._handle))
            self._handle = ctypes.c_uint64(0)
            self._closed = True

    def close(self, timeout_ms: int = 10_000) -> None:
        self.shutdown(timeout_ms)
        self.destroy()

    def __enter__(self) -> "Runtime":
        return self

    def __exit__(self, exc_type: Any, exc: Any, traceback: Any) -> None:
        self.close()


class RuntimeApp:
    """Recommended host-facing wrapper around one Runtime handle."""

    def __init__(
        self,
        runtime: Runtime,
        *,
        event_bus: RuntimeEventBus | None = None,
        event_pump: RuntimeEventPump | None = None,
    ):
        self.runtime = runtime
        self.event_bus = event_bus or RuntimeEventBus()
        self._event_pump = event_pump

    def subscribe(
        self,
        event_filter: RuntimeEventFilter | None = None,
        *,
        maxsize: int = 1000,
    ) -> RuntimeEventSubscription:
        return self.event_bus.subscribe(event_filter, maxsize=maxsize)

    def subscribe_workflow(
        self, workflow_id: str, *, maxsize: int = 1000
    ) -> RuntimeEventSubscription:
        return self.event_bus.subscribe(
            lambda event: workflow_id_from_event(event) == workflow_id,
            maxsize=maxsize,
        )

    def invoke(self, command_type: str, payload: Mapping[str, Any] | None = None) -> Any:
        return self.runtime.invoke(command_type, payload)

    def shutdown(self, timeout_ms: int = 10_000) -> None:
        self.runtime.shutdown(timeout_ms)

    def destroy(self) -> None:
        if self._event_pump is not None:
            self._event_pump.stop()
            self._event_pump = None
        else:
            self.event_bus.close()
        self.runtime.destroy()

    def close(self, timeout_ms: int = 10_000) -> None:
        self.runtime.shutdown(timeout_ms)
        if self._event_pump is not None:
            self._event_pump.join(timeout=2.0)
            self._event_pump.stop()
            self._event_pump = None
        else:
            self.event_bus.close()
        self.runtime.destroy()

    def __getattr__(self, name: str) -> Any:
        return getattr(self.runtime, name)

    def __enter__(self) -> "RuntimeApp":
        return self

    def __exit__(self, exc_type: Any, exc: Any, traceback: Any) -> None:
        self.close()


class RuntimeHostBuilder:
    """Builder for the host bootstrap sequence.

    The order is fixed and shared with the Rust SDK:
    load dynamic library, create runtime, register resources/LLM/cluster, start,
    then publish one public event stream through an in-process bus.
    """

    def __init__(self, library_path: str | Path):
        self._library_path = Path(library_path)
        self._create_options: str | Mapping[str, Any] | None = None
        self._required_commands: tuple[str, ...] = ()
        self._resources_path: str | None = None
        self._resources_json: Mapping[str, Any] | None = None
        self._llm_path: str | None = None
        self._llm_json: Mapping[str, Any] | None = None
        self._cluster_path: str | None = None
        self._cluster_json: Mapping[str, Any] | None = None
        self._public_events_only = True
        self._diagnostics: Any | None = None
        self._event_timeout_ms = 250

    @classmethod
    def from_release(
        cls,
        *,
        version: str = DEFAULT_RUNTIME_VERSION,
        cache_dir: str | Path | None = None,
        platform_id: str | None = None,
        force_download: bool = False,
    ) -> "RuntimeHostBuilder":
        from .native import ensure_runtime_library

        library_path = ensure_runtime_library(
            version=version,
            cache_dir=cache_dir,
            platform_id=platform_id,
            force=force_download,
        )
        return cls(library_path)

    def create_options(
        self,
        options: str | Mapping[str, Any] | None,
    ) -> "RuntimeHostBuilder":
        self._create_options = options
        return self

    def required_commands(self, *commands: str) -> "RuntimeHostBuilder":
        self._required_commands = tuple(commands)
        return self

    def resources_path(self, path: str | Path) -> "RuntimeHostBuilder":
        self._resources_path = str(path)
        return self

    def resources_json(self, registration: Mapping[str, Any]) -> "RuntimeHostBuilder":
        self._resources_json = registration
        return self

    def llm_path(self, path: str | Path) -> "RuntimeHostBuilder":
        self._llm_path = str(path)
        return self

    def llm_json(self, registration: Mapping[str, Any]) -> "RuntimeHostBuilder":
        self._llm_json = registration
        return self

    def agent_cluster_path(self, path: str | Path) -> "RuntimeHostBuilder":
        self._cluster_path = str(path)
        return self

    def agent_cluster_json(self, registration: Mapping[str, Any]) -> "RuntimeHostBuilder":
        self._cluster_json = registration
        return self

    def public_events_only(self, enabled: bool) -> "RuntimeHostBuilder":
        self._public_events_only = enabled
        return self

    def diagnostics(self, callback: Any | None) -> "RuntimeHostBuilder":
        self._diagnostics = callback
        return self

    def event_timeout_ms(self, timeout_ms: int) -> "RuntimeHostBuilder":
        self._event_timeout_ms = timeout_ms
        return self

    def start(self) -> RuntimeApp:
        runtime = Runtime(
            self._library_path,
            self._create_options,
            required_commands=self._required_commands,
        )
        try:
            if self._resources_path is not None:
                runtime.register_resources_file(self._resources_path)
            if self._resources_json is not None:
                runtime.register_resources(self._resources_json)
            if self._llm_path is not None:
                runtime.register_llm_file(self._llm_path)
            if self._llm_json is not None:
                runtime.register_llm(self._llm_json)
            if self._cluster_path is not None:
                runtime.register_agent_cluster_file(self._cluster_path)
            if self._cluster_json is not None:
                runtime.register_agent_cluster(self._cluster_json)
            runtime.start()
            bus = RuntimeEventBus(public_only=self._public_events_only)
            pump = RuntimeEventPump(
                runtime,
                bus,
                timeout_ms=self._event_timeout_ms,
                diagnostics=self._diagnostics,
            ).start()
            return RuntimeApp(runtime, event_bus=bus, event_pump=pump)
        except BaseException:
            try:
                runtime.shutdown()
            except RuntimeError:
                pass
            try:
                runtime.destroy()
            except RuntimeError:
                pass
            raise
