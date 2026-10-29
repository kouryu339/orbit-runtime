# RPC metadata regression through the packaged DLL

The affected DLL can list a tool in `runtime.get_tool_definitions` but rejects
`conversation.send_message` with `active tool 'BrowserOpenPage' has no registered
metadata` before sending any model request. Ordinary RPC tools are affected too.

Install Node dependencies with `npm ci --prefix sdk/rpctools/node`. Python uses
only its standard library and the SDK in this checkout. This fixture opens local
HTTP/gRPC listeners; it does not launch a browser or contact a model provider.

```powershell
python agent_runtime_ffi/tests/rpc_ai_metadata.py C:/path/to/old/agent_runtime.dll --expect-bug --output old-result.json
python agent_runtime_ffi/tests/rpc_ai_metadata.py C:/path/to/fixed/agent_runtime.dll --output fixed-result.json
```

It tests `BrowserOpenPage` and `PlainRpcProbe` independently. The old DLL must
show a visible definition, missing Agent metadata, zero model requests and zero
RPC executions. The fixed DLL must send two model requests, execute the RPC once
and expose the final assistant reply. The simulated provider rejects orphaned
tool results; tool schema presence and conversation/Agent/call IDs are checked.

When reusing dependencies from another checkout, set
`ORBIT_RPC_FIXTURE_SDK_ROOT` to that checkout's root. Temporary Runtime data can
be placed on another drive with `ORBIT_RPC_FIXTURE_DATA_ROOT`.

The Rust regression `agent_schema_inherits_initial_and_late_runtime_rpc_metadata`
also constructs an actual Conversation Agent, adds a second tool after creating
its execution context, verifies both native FC definitions, and rejects leakage
into the process-global registry.
