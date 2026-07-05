//go:build !cgo

package runtimehost

import (
	"context"
	"encoding/json"
	"errors"
)

type Runtime struct{}

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
}

const LedgerDeltaEventType = "conversation.ledger_delta"
const StateDeltaEventType = "conversation.state_delta"

type RuntimeEvent struct {
	Type                 string          `json:"type"`
	ConversationID       string          `json:"conversation_id,omitempty"`
	ConversationEventSeq uint64          `json:"conversation_event_seq,omitempty"`
	Payload              json.RawMessage `json:"payload"`
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
	Payload         json.RawMessage `json:"-"`
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

func Start(Options) (*Runtime, error) {
	return nil, errors.New("runtimehost requires cgo")
}

func (*Runtime) Enabled() bool { return false }

func (*Runtime) ConversationID() string { return "" }

func Version() string { return "unsupported" }

func (*Runtime) SendMessage(context.Context, string, string) error {
	return errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) SendMessageAdmission(context.Context, string, string) (AdmissionResult, error) {
	return AdmissionResult{}, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) Pause(context.Context, string) error {
	return errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) PauseAdmission(context.Context, string) (AdmissionResult, error) {
	return AdmissionResult{}, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) SendCommand(context.Context, json.RawMessage) error {
	return errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) Snapshot(context.Context) (json.RawMessage, error) {
	return json.RawMessage(`{"enabled":false}`), nil
}

func (*Runtime) ConversationSnapshot(context.Context, string, json.RawMessage) (json.RawMessage, error) {
	return json.RawMessage(`{"enabled":false}`), nil
}

func (*Runtime) Events() []json.RawMessage { return nil }

func (*Runtime) Close() {}

func (*Runtime) RegisterResourcesFile(context.Context, string) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) RegisterAgentClusterFile(context.Context, string) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) CreateConversation(context.Context, json.RawMessage) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) SpawnConversationFromSnapshot(context.Context, json.RawMessage, json.RawMessage) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) MaterializeConversation(context.Context, string, json.RawMessage) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) ImportConversationSnapshot(context.Context, json.RawMessage, json.RawMessage) error {
	return errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) CloseConversation(context.Context, string) error {
	return errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) SetDynamicSnapshot(context.Context, string, string, string, string) error {
	return errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) ResolveToolPermission(context.Context, string, string, string) (bool, error) {
	return false, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) AgentTasks(context.Context, string) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) SetSummaryModel(context.Context, string, string) (AdmissionResult, error) {
	return AdmissionResult{}, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) CompactHistory(context.Context, string, []string) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) ReloadLlmFile(context.Context, string) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) SetCurrentModel(context.Context, uint32) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func (*Runtime) ProviderDefinitions(context.Context) (json.RawMessage, error) {
	return nil, errors.New("runtimehost requires linux and cgo")
}

func LastError() string { return "runtimehost requires linux and cgo" }
