//go:build cgo

package runtimehost

/*
#cgo CFLAGS: -I../c/include
#cgo LDFLAGS: -lagent_runtime
#include <stdlib.h>
#include "agent_runtime.h"
*/
import "C"

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"time"
	"unsafe"
)

const expectedRuntimeABI = 1

const LedgerDeltaEventType = "conversation.ledger_delta"
const StateDeltaEventType = "conversation.state_delta"

type Runtime struct {
	mu        sync.Mutex
	handle    C.AgentRuntimeHandle
	events    []json.RawMessage
	eventSink func(json.RawMessage)
	stop      chan struct{}
	done      chan struct{}
}

type Options struct {
	CreateOptions        json.RawMessage
	EventSink            func(json.RawMessage)
	ResourceRegistration json.RawMessage
	LlmRegistration      json.RawMessage
	ClusterRegistration  json.RawMessage
}

type AdmissionResult struct {
	CommandID    string `json:"command_id"`
	Accepted     bool   `json:"accepted"`
	Decision     string `json:"decision"`
	RejectReason string `json:"reject_reason,omitempty"`
	Error        string `json:"error,omitempty"`
}

type RuntimeEvent struct {
	Type                 string          `json:"type"`
	EventLine            string          `json:"event_line,omitempty"`
	ConversationID       string          `json:"conversation_id,omitempty"`
	ConversationEventSeq uint64          `json:"conversation_event_seq,omitempty"`
	Payload              json.RawMessage `json:"payload"`
}

type WorkflowEvent struct {
	EventLine  string `json:"event_line"`
	WorkflowID string `json:"workflow_id"`
}

type LedgerDelta struct {
	Schema         string          `json:"schema"`
	Op             string          `json:"op"`
	ConversationID string          `json:"conversation_id"`
	RecordID       uint64          `json:"record_id"`
	Record         json.RawMessage `json:"record"`
}

type StateDelta struct {
	Schema         string          `json:"schema"`
	Op             string          `json:"op"`
	ConversationID string          `json:"conversation_id"`
	Payload        json.RawMessage `json:"-"`
}

func ParseLedgerDelta(event json.RawMessage) (LedgerDelta, bool, error) {
	var envelope RuntimeEvent
	if err := json.Unmarshal(event, &envelope); err != nil {
		return LedgerDelta{}, false, err
	}
	if envelope.Type != LedgerDeltaEventType {
		return LedgerDelta{}, false, nil
	}
	var delta LedgerDelta
	if err := json.Unmarshal(envelope.Payload, &delta); err != nil {
		return LedgerDelta{}, true, err
	}
	return delta, true, nil
}

func ParseStateDelta(event json.RawMessage) (StateDelta, bool, error) {
	var envelope RuntimeEvent
	if err := json.Unmarshal(event, &envelope); err != nil {
		return StateDelta{}, false, err
	}
	if envelope.Type != StateDeltaEventType {
		return StateDelta{}, false, nil
	}
	var delta StateDelta
	if err := json.Unmarshal(envelope.Payload, &delta); err != nil {
		return StateDelta{}, true, err
	}
	delta.Payload = envelope.Payload
	return delta, true, nil
}

func ParseWorkflowEvent(event json.RawMessage) (WorkflowEvent, bool, error) {
	var envelope RuntimeEvent
	if err := json.Unmarshal(event, &envelope); err != nil {
		return WorkflowEvent{}, false, err
	}
	var workflowEvent WorkflowEvent
	if err := json.Unmarshal(envelope.Payload, &workflowEvent); err != nil {
		return WorkflowEvent{}, envelope.EventLine == "workflow", err
	}
	if envelope.EventLine != "workflow" && workflowEvent.EventLine != "workflow" {
		return WorkflowEvent{}, false, nil
	}
	return workflowEvent, true, nil
}

type invokeEnvelope struct {
	OK      bool            `json:"ok"`
	Result  json.RawMessage `json:"result"`
	Error   json.RawMessage `json:"error"`
	Command string          `json:"command_id"`
}

func Start(options Options) (*Runtime, error) {
	if uint32(C.agent_runtime_abi_version_v1()) != expectedRuntimeABI {
		return nil, fmt.Errorf(
			"runtime ABI mismatch: expected %d, got %d",
			expectedRuntimeABI,
			uint32(C.agent_runtime_abi_version_v1()),
		)
	}
	createOptions := options.CreateOptions
	if len(createOptions) == 0 {
		createOptions = json.RawMessage(``)
	}
	createOptionsC := C.CString(string(createOptions))
	defer C.free(unsafe.Pointer(createOptionsC))
	var handle C.AgentRuntimeHandle
	if code := C.agent_runtime_create_v1(createOptionsC, &handle); code != C.AGENT_RUNTIME_OK {
		return nil, fmt.Errorf("create runtime failed (%d): %s", int(code), LastError())
	}
	runtime := &Runtime{
		handle:    handle,
		eventSink: options.EventSink,
		stop:      make(chan struct{}),
		done:      make(chan struct{}),
	}
	resources := options.ResourceRegistration
	if len(resources) > 0 {
		if _, err := runtime.invoke(
			context.Background(),
			"runtime.register_resources",
			map[string]any{"registration": json.RawMessage(resources)},
		); err != nil {
			runtime.Close()
			return nil, fmt.Errorf("register resources: %w", err)
		}
	}
	llm := options.LlmRegistration
	if len(llm) > 0 {
		if _, err := runtime.invoke(
			context.Background(),
			"runtime.register_llm",
			map[string]any{"registration": json.RawMessage(llm)},
		); err != nil {
			runtime.Close()
			return nil, fmt.Errorf("register llm: %w", err)
		}
	}
	cluster := options.ClusterRegistration
	if len(cluster) > 0 {
		if _, err := runtime.invoke(
			context.Background(),
			"runtime.register_agent_cluster",
			map[string]any{"registration": json.RawMessage(cluster)},
		); err != nil {
			runtime.Close()
			return nil, fmt.Errorf("register agent cluster: %w", err)
		}
	}
	if code := C.agent_runtime_start_v1(handle); code != C.AGENT_RUNTIME_OK {
		runtime.Close()
		return nil, fmt.Errorf("start runtime failed (%d): %s", int(code), LastError())
	}
	go runtime.pumpEvents()
	return runtime, nil
}

func (r *Runtime) RegisterResourcesFile(ctx context.Context, path string) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.register_resources", map[string]any{
		"input": path,
	})
}

func (r *Runtime) CreateWorkflowDraft(ctx context.Context, resource json.RawMessage) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.create", map[string]any{"resource": resource})
}

func (r *Runtime) ReadWorkflow(ctx context.Context, id string) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.read", map[string]any{"id": id})
}

func (r *Runtime) RegisterWorkflowDraft(ctx context.Context, id string, expectedRevision *uint64, name string) (json.RawMessage, error) {
	payload := map[string]any{"id": id, "expected_revision": expectedRevision}
	if name != "" {
		payload["name"] = name
	}
	return r.invoke(ctx, "workflow.register", payload)
}

func (r *Runtime) UpdateWorkflow(ctx context.Context, resource json.RawMessage, expectedRevision *uint64) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.update", map[string]any{"resource": resource, "expected_revision": expectedRevision})
}

func (r *Runtime) CompileWorkflowDraft(ctx context.Context, id string) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.compile", map[string]any{"id": id})
}

func (r *Runtime) WorkflowScriptToBlueprint(ctx context.Context, script string) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.convert.script_to_blueprint", map[string]any{"script": script})
}

func (r *Runtime) WorkflowBlueprintToScript(ctx context.Context, blueprint json.RawMessage) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.convert.blueprint_to_script", map[string]any{"blueprint": blueprint})
}

func (r *Runtime) DeleteWorkflow(ctx context.Context, id string, expectedRevision *uint64) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.delete", map[string]any{"id": id, "expected_revision": expectedRevision})
}

func (r *Runtime) ListWorkflows(ctx context.Context, kind string) (json.RawMessage, error) {
	payload := map[string]any{}
	if kind != "" {
		payload["kind"] = kind
	}
	return r.invoke(ctx, "workflow.list", payload)
}

func (r *Runtime) ExecuteWorkflow(ctx context.Context, id string, inputs map[string]any, trace bool) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.execute", map[string]any{"id": id, "inputs": inputs, "trace": trace})
}

func (r *Runtime) ExecuteWorkflowInContext(ctx context.Context, id string, inputs map[string]any, trace bool, conversationID string, agentID string) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.execute", map[string]any{"id": id, "inputs": inputs, "trace": trace, "conversation_id": conversationID, "agent_id": agentID})
}

func (r *Runtime) TestWorkflowDraft(ctx context.Context, id string, inputs map[string]any, trace bool) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.execute", map[string]any{"id": id, "mode": "test", "inputs": inputs, "trace": trace})
}

func (r *Runtime) TestWorkflowDraftInContext(ctx context.Context, id string, inputs map[string]any, trace bool, conversationID string, agentID string) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.execute", map[string]any{"id": id, "mode": "test", "inputs": inputs, "trace": trace, "conversation_id": conversationID, "agent_id": agentID})
}

func (r *Runtime) ExecuteWorkflowScript(ctx context.Context, script string, inputs map[string]any, trace bool) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.execute_script", map[string]any{
		"script": script,
		"inputs": inputs,
		"trace":  trace,
	})
}

func (r *Runtime) ExecuteWorkflowScriptInContext(ctx context.Context, script string, inputs map[string]any, trace bool, conversationID string, agentID string) (json.RawMessage, error) {
	return r.invoke(ctx, "workflow.execute_script", map[string]any{
		"script":          script,
		"inputs":          inputs,
		"trace":           trace,
		"conversation_id": conversationID,
		"agent_id":        agentID,
	})
}

func (r *Runtime) RegisterLlmFile(ctx context.Context, path string) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.register_llm", map[string]any{
		"input": path,
	})
}

func (r *Runtime) RegisterAgentClusterFile(ctx context.Context, path string) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.register_agent_cluster", map[string]any{
		"input": path,
	})
}

func (r *Runtime) Enabled() bool {
	return r != nil && r.handle != C.AgentRuntimeHandle(C.AGENT_RUNTIME_INVALID_HANDLE)
}

func (r *Runtime) Version() string {
	if r == nil {
		return ""
	}
	return C.GoString(C.agent_runtime_version_v1())
}

func Version() string {
	return C.GoString(C.agent_runtime_version_v1())
}

func (r *Runtime) invoke(
	ctx context.Context,
	commandType string,
	payload any,
) (json.RawMessage, error) {
	if !r.Enabled() {
		return nil, errors.New("runtime is disabled")
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	request, err := json.Marshal(map[string]any{
		"schema":  "agent-runtime-command/v1",
		"type":    commandType,
		"payload": payload,
	})
	if err != nil {
		return nil, err
	}
	requestC := C.CString(string(request))
	defer C.free(unsafe.Pointer(requestC))
	var responseC *C.char
	code := C.agent_runtime_invoke_v1(r.handle, requestC, &responseC)
	var envelope invokeEnvelope
	if responseC != nil {
		defer C.agent_runtime_free_string_v1(responseC)
		if err := json.Unmarshal([]byte(C.GoString(responseC)), &envelope); err != nil {
			return nil, fmt.Errorf("decode runtime response: %w", err)
		}
	}
	if code != C.AGENT_RUNTIME_OK {
		if len(envelope.Error) > 0 {
			return nil, fmt.Errorf("%s failed (%d): %s", commandType, int(code), envelope.Error)
		}
		return nil, fmt.Errorf("%s failed (%d): %s", commandType, int(code), LastError())
	}
	return envelope.Result, nil
}

func (r *Runtime) pumpEvents() {
	defer close(r.done)
	for {
		select {
		case <-r.stop:
			return
		default:
		}
		var eventC *C.char
		code := C.agent_runtime_next_event_v1(r.handle, 250, &eventC)
		if code == C.AGENT_RUNTIME_ERR_TIMEOUT {
			continue
		}
		if code != C.AGENT_RUNTIME_OK {
			return
		}
		event := json.RawMessage(C.GoString(eventC))
		C.agent_runtime_free_string_v1(eventC)
		if !IsPublicRuntimeEvent(event) {
			continue
		}
		r.mu.Lock()
		r.events = append(r.events, event)
		if len(r.events) > 200 {
			r.events = r.events[len(r.events)-200:]
		}
		sink := r.eventSink
		r.mu.Unlock()
		if sink != nil {
			sink(event)
		}
	}
}

func (r *Runtime) SendMessage(ctx context.Context, conversationID string, content string) error {
	_, err := r.SendMessageAdmission(ctx, conversationID, content)
	return err
}

func (r *Runtime) SendMessageAdmission(
	ctx context.Context,
	conversationID string,
	content string,
) (AdmissionResult, error) {
	raw, err := r.invoke(ctx, "conversation.send_message", map[string]any{
		"conversation_id": conversationID,
		"content":         content,
	})
	if err != nil {
		return AdmissionResult{}, err
	}
	var admission struct {
		CommandID string `json:"command_id"`
		Decision  string `json:"decision"`
		Reason    string `json:"reason"`
	}
	if err := json.Unmarshal(raw, &admission); err != nil {
		return AdmissionResult{}, err
	}
	return AdmissionResult{
		CommandID:    admission.CommandID,
		Accepted:     admission.Decision == "accepted",
		Decision:     admission.Decision,
		RejectReason: admission.Reason,
	}, nil
}

func (r *Runtime) Pause(ctx context.Context, conversationID string) error {
	_, err := r.PauseAdmission(ctx, conversationID)
	return err
}

func (r *Runtime) PauseAdmission(
	ctx context.Context,
	conversationID string,
) (AdmissionResult, error) {
	raw, err := r.invoke(ctx, "conversation.pause", map[string]any{
		"conversation_id": conversationID,
	})
	if err != nil {
		return AdmissionResult{}, err
	}
	var admission struct {
		CommandID string `json:"command_id"`
		Decision  string `json:"decision"`
		Reason    string `json:"reason"`
	}
	if err := json.Unmarshal(raw, &admission); err != nil {
		return AdmissionResult{}, err
	}
	return AdmissionResult{
		CommandID:    admission.CommandID,
		Accepted:     admission.Decision == "accepted",
		Decision:     admission.Decision,
		RejectReason: admission.Reason,
	}, nil
}

func (r *Runtime) SendCommand(context.Context, json.RawMessage) error {
	return errors.New("legacy SendCommand is not supported by ABI 1; use a typed runtime command")
}

func (r *Runtime) Snapshot(ctx context.Context) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.export_snapshot", map[string]any{})
}

func (r *Runtime) ConversationSnapshot(
	ctx context.Context,
	conversationID string,
	options json.RawMessage,
) (json.RawMessage, error) {
	var decoded any = map[string]any{}
	if len(options) > 0 {
		if err := json.Unmarshal(options, &decoded); err != nil {
			return nil, err
		}
	}
	return r.invoke(ctx, "conversation.export_snapshot", map[string]any{
		"conversation_id": conversationID,
		"options":         decoded,
	})
}

func (r *Runtime) Events() []json.RawMessage {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]json.RawMessage, len(r.events))
	copy(out, r.events)
	return out
}

func (r *Runtime) Close() {
	if !r.Enabled() {
		return
	}
	if code := C.agent_runtime_shutdown_v1(r.handle, 10000); code == C.AGENT_RUNTIME_OK {
		select {
		case <-r.done:
		case <-time.After(time.Second):
			select {
			case <-r.stop:
			default:
				close(r.stop)
			}
			select {
			case <-r.done:
			case <-time.After(time.Second):
			}
		}
		C.agent_runtime_destroy_v1(r.handle)
	} else {
		select {
		case <-r.stop:
		default:
			close(r.stop)
		}
	}
	r.handle = C.AgentRuntimeHandle(C.AGENT_RUNTIME_INVALID_HANDLE)
}

func (r *Runtime) CreateConversation(
	ctx context.Context,
	options json.RawMessage,
) (json.RawMessage, error) {
	var payload any = map[string]any{}
	if len(options) > 0 {
		if err := json.Unmarshal(options, &payload); err != nil {
			return nil, err
		}
	}
	return r.invoke(ctx, "conversation.spawn", payload)
}

type ToolPermissionPolicy struct {
	ReadOnly         string `json:"read_only,omitempty"`
	ControlledChange string `json:"controlled_change,omitempty"`
	Destructive      string `json:"destructive,omitempty"`
}

type ConversationSpawnOptions struct {
	Schema          string                `json:"schema"`
	ClusterID       string                `json:"cluster_id"`
	ToolHostContext map[string]any        `json:"tool_host_context,omitempty"`
	Permissions     *ToolPermissionPolicy `json:"permissions,omitempty"`
}

func (r *Runtime) SpawnConversation(
	ctx context.Context,
	options ConversationSpawnOptions,
) (json.RawMessage, error) {
	if options.Schema == "" {
		options.Schema = "agent-runtime-conversation-spawn/v1"
	}
	return r.invoke(ctx, "conversation.spawn", options)
}

func (r *Runtime) SpawnConversationFromSnapshot(
	ctx context.Context,
	spawn json.RawMessage,
	snapshot json.RawMessage,
) (json.RawMessage, error) {
	var spawnValue any
	var snapshotValue any
	if err := json.Unmarshal(spawn, &spawnValue); err != nil {
		return nil, err
	}
	if err := json.Unmarshal(snapshot, &snapshotValue); err != nil {
		return nil, err
	}
	return r.invoke(ctx, "conversation.spawn_from_snapshot", map[string]any{
		"spawn":    spawnValue,
		"snapshot": snapshotValue,
	})
}

func (r *Runtime) MaterializeConversation(
	ctx context.Context,
	conversationID string,
	options json.RawMessage,
) (json.RawMessage, error) {
	var optionsValue any = map[string]any{}
	if len(options) > 0 {
		if err := json.Unmarshal(options, &optionsValue); err != nil {
			return nil, err
		}
	}
	return r.invoke(ctx, "conversation.materialize", map[string]any{
		"conversation_id": conversationID,
		"options":         optionsValue,
	})
}

func (r *Runtime) ImportConversationSnapshot(
	ctx context.Context,
	snapshot json.RawMessage,
	options json.RawMessage,
) error {
	var snapshotValue any
	var optionsValue any = map[string]any{}
	if err := json.Unmarshal(snapshot, &snapshotValue); err != nil {
		return err
	}
	if len(options) > 0 {
		if err := json.Unmarshal(options, &optionsValue); err != nil {
			return err
		}
	}
	_, err := r.invoke(ctx, "conversation.import_snapshot", map[string]any{
		"snapshot": snapshotValue,
		"options":  optionsValue,
	})
	return err
}

func (r *Runtime) CloseConversation(ctx context.Context, conversationID string) error {
	_, err := r.invoke(ctx, "conversation.close", map[string]any{
		"conversation_id": conversationID,
	})
	return err
}

func (r *Runtime) SetDynamicSnapshot(
	ctx context.Context,
	conversationID string,
	agentID string,
	fieldName string,
	text string,
) error {
	_, err := r.invoke(ctx, "conversation.set_dynamic_snapshot", map[string]any{
		"conversation_id": conversationID,
		"agent_id":        agentID,
		"field_name":      fieldName,
		"text":            text,
	})
	return err
}

func (r *Runtime) ResolveToolPermission(
	ctx context.Context,
	conversationID string,
	toolCallID string,
	decision string,
) (bool, error) {
	raw, err := r.invoke(ctx, "conversation.resolve_tool_permission", map[string]any{
		"conversation_id": conversationID,
		"tool_call_id":    toolCallID,
		"decision":        decision,
	})
	if err != nil {
		return false, err
	}
	var result struct {
		Resolved bool `json:"resolved"`
	}
	if err := json.Unmarshal(raw, &result); err != nil {
		return false, err
	}
	return result.Resolved, nil
}

func (r *Runtime) AgentTasks(ctx context.Context, conversationID string) (json.RawMessage, error) {
	return r.invoke(ctx, "conversation.agent_tasks", map[string]any{
		"conversation_id": conversationID,
	})
}

func (r *Runtime) SetSummaryModel(
	ctx context.Context,
	conversationID string,
	modelName string,
) (AdmissionResult, error) {
	raw, err := r.invoke(ctx, "conversation.set_summary_model", map[string]any{
		"conversation_id": conversationID,
		"model_name":      modelName,
	})
	if err != nil {
		return AdmissionResult{}, err
	}
	var admission struct {
		CommandID string `json:"command_id"`
		Decision  string `json:"decision"`
		Reason    string `json:"reason"`
	}
	if err := json.Unmarshal(raw, &admission); err != nil {
		return AdmissionResult{}, err
	}
	return AdmissionResult{
		CommandID:    admission.CommandID,
		Accepted:     admission.Decision == "accepted",
		Decision:     admission.Decision,
		RejectReason: admission.Reason,
	}, nil
}

func (r *Runtime) CompactHistory(
	ctx context.Context,
	conversationID string,
	agentIDs []string,
) (json.RawMessage, error) {
	return r.invoke(ctx, "conversation.compact_history", map[string]any{
		"conversation_id": conversationID,
		"agent_ids":       agentIDs,
	})
}

func (r *Runtime) ReloadLlmFile(ctx context.Context, path string) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.reload_llm", map[string]any{
		"input": path,
	})
}

func (r *Runtime) SetCurrentModel(ctx context.Context, modelUID uint32) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.set_current_model", map[string]any{
		"model_uid": modelUID,
	})
}

func (r *Runtime) ProviderDefinitions(ctx context.Context) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.get_provider_definitions", map[string]any{})
}

func (r *Runtime) ToolDefinitions(ctx context.Context) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.get_tool_definitions", map[string]any{})
}

func (r *Runtime) WorkflowNodeDefinitions(ctx context.Context) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.get_workflow_node_definitions", map[string]any{})
}

func (r *Runtime) AgentClusterDefinitions(ctx context.Context) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.get_agent_cluster_definitions", map[string]any{})
}

func (r *Runtime) RPCEndpointDefinitions(ctx context.Context) (json.RawMessage, error) {
	return r.invoke(ctx, "runtime.get_rpc_endpoint_definitions", map[string]any{})
}

func LastError() string {
	ptr := C.agent_runtime_last_error_json_v1()
	if ptr == nil {
		return ""
	}
	return C.GoString(ptr)
}
