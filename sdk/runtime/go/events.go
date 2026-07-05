package runtimehost

import "encoding/json"

const ConversationCreatedEventType = "conversation:created"
const ConversationClosedEventType = "conversation:closed"
const FrontendStateSnapshotEventType = "frontend:state_snapshot"

func IsPublicRuntimeEvent(event json.RawMessage) bool {
	var envelope RuntimeEvent
	if err := json.Unmarshal(event, &envelope); err != nil {
		return false
	}
	switch envelope.Type {
	case ConversationCreatedEventType,
		ConversationClosedEventType,
		LedgerDeltaEventType,
		StateDeltaEventType,
		FrontendStateSnapshotEventType:
		return true
	default:
		return false
	}
}

func ConversationIDFromEvent(event json.RawMessage) string {
	var envelope RuntimeEvent
	if err := json.Unmarshal(event, &envelope); err != nil {
		return ""
	}
	if envelope.ConversationID != "" {
		return envelope.ConversationID
	}
	var payload struct {
		ConversationID string `json:"conversation_id"`
	}
	if err := json.Unmarshal(envelope.Payload, &payload); err != nil {
		return ""
	}
	return payload.ConversationID
}

type ConversationPosition string

const (
	ConversationCurrent    ConversationPosition = "current"
	ConversationBackground ConversationPosition = "background"
	ConversationDataOnly   ConversationPosition = "data_only"
)

type ConversationRegistryAction struct {
	Kind           string `json:"kind"`
	ConversationID string `json:"conversation_id"`
}

type ConversationRegistryEntry struct {
	ConversationID string               `json:"conversation_id"`
	Position       ConversationPosition `json:"position"`
	Waiting        bool                 `json:"waiting"`
}

type ConversationRegistry struct {
	current string
	entries map[string]ConversationRegistryEntry
}

func NewConversationRegistry() *ConversationRegistry {
	return &ConversationRegistry{
		entries: make(map[string]ConversationRegistryEntry),
	}
}

func (r *ConversationRegistry) Track(conversationID string, position ConversationPosition) {
	if r.entries == nil {
		r.entries = make(map[string]ConversationRegistryEntry)
	}
	r.entries[conversationID] = ConversationRegistryEntry{
		ConversationID: conversationID,
		Position:       position,
	}
	if position == ConversationCurrent {
		r.current = conversationID
	}
}

func (r *ConversationRegistry) SetCurrent(conversationID string) []ConversationRegistryAction {
	var actions []ConversationRegistryAction
	if r.entries == nil {
		r.entries = make(map[string]ConversationRegistryEntry)
	}
	if r.current != "" {
		if entry, ok := r.entries[r.current]; ok {
			entry.Position = ConversationBackground
			r.entries[r.current] = entry
			if entry.Waiting {
				actions = append(actions, ConversationRegistryAction{
					Kind:           "close_background",
					ConversationID: entry.ConversationID,
				})
			}
		}
	}
	r.current = conversationID
	if conversationID != "" {
		r.Track(conversationID, ConversationCurrent)
	}
	return actions
}

func (r *ConversationRegistry) ObserveEvent(event json.RawMessage) []ConversationRegistryAction {
	var envelope RuntimeEvent
	if err := json.Unmarshal(event, &envelope); err != nil {
		return nil
	}
	conversationID := ConversationIDFromEvent(event)
	if conversationID == "" {
		return nil
	}
	if r.entries == nil {
		r.entries = make(map[string]ConversationRegistryEntry)
	}
	switch envelope.Type {
	case ConversationCreatedEventType:
		if _, ok := r.entries[conversationID]; !ok {
			r.Track(conversationID, ConversationBackground)
		}
	case ConversationClosedEventType:
		delete(r.entries, conversationID)
		if r.current == conversationID {
			r.current = ""
		}
	case StateDeltaEventType:
		entry, ok := r.entries[conversationID]
		if !ok {
			return nil
		}
		entry.Waiting = eventMarksWaiting(envelope.Payload)
		r.entries[conversationID] = entry
		if entry.Waiting && entry.Position == ConversationBackground {
			return []ConversationRegistryAction{{
				Kind:           "close_background",
				ConversationID: conversationID,
			}}
		}
	}
	return nil
}

func eventMarksWaiting(payload json.RawMessage) bool {
	var state struct {
		Status string `json:"status"`
		State  string `json:"state"`
	}
	if err := json.Unmarshal(payload, &state); err != nil {
		return false
	}
	return state.Status == "waiting" || state.State == "waiting"
}
