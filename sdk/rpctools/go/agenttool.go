package agenttool

import "context"

type ToolErrorCode int32

const (
	ToolErrorCodeUnspecified               ToolErrorCode = 0
	ToolErrorCodeOK                        ToolErrorCode = 1
	ToolErrorCodeInvalidArgument           ToolErrorCode = 100
	ToolErrorCodeMissingArgument           ToolErrorCode = 101
	ToolErrorCodePermissionDenied          ToolErrorCode = 102
	ToolErrorCodeNotFound                  ToolErrorCode = 103
	ToolErrorCodeConflict                  ToolErrorCode = 104
	ToolErrorCodeInternal                  ToolErrorCode = 200
	ToolErrorCodeTimeout                   ToolErrorCode = 201
	ToolErrorCodeCancelled                 ToolErrorCode = 202
	ToolErrorCodeUnavailable               ToolErrorCode = 203
	ToolErrorCodeHostCapabilityDenied      ToolErrorCode = 300
	ToolErrorCodeHostCapabilityUnsupported ToolErrorCode = 301
	ToolErrorCodeHostCallFailed            ToolErrorCode = 302
	ToolErrorCodeProtocolError             ToolErrorCode = 400
	ToolErrorCodeInvalidOutput             ToolErrorCode = 401
	ToolErrorCodeSchemaMismatch            ToolErrorCode = 402
)

type ToolParameter struct {
	Name         string `json:"name"`
	ParamType    string `json:"param_type"`
	Required     bool   `json:"required"`
	DefaultValue string `json:"default_value,omitempty"`
	Description  string `json:"description,omitempty"`
}

type ToolOutputField struct {
	Name        string `json:"name"`
	FieldType   string `json:"field_type"`
	Description string `json:"description,omitempty"`
}

type ToolDescriptor struct {
	Name                 string            `json:"name"`
	Description          string            `json:"description,omitempty"`
	Parameters           []ToolParameter   `json:"parameters,omitempty"`
	Outputs              []ToolOutputField `json:"outputs,omitempty"`
	Readonly             bool              `json:"readonly"`
	Destructive          bool              `json:"destructive"`
	Idempotent           bool              `json:"idempotent"`
	OpenWorld            bool              `json:"open_world"`
	Secret               bool              `json:"secret"`
	Category             string            `json:"category,omitempty"`
	DisplayName          string            `json:"display_name,omitempty"`
	RequiredCapabilities []string          `json:"required_capabilities,omitempty"`
}

type AIOutput struct {
	Result    any
	ToAI      string
	ErrorCode ToolErrorCode
}

type Context struct {
	CallID            string
	ToolCallID        string
	IdempotencyKey    string
	SessionID         string
	ProviderID        string
	ClusterID         string
	RuntimeInstanceID string
	ConversationID    string
	AgentID           string
	TurnID            string
	Permissions       []string
	HostContext       any
}

func (Context) WorkspaceResolvePath(ctx context.Context, path string) (map[string]any, error) {
	return nil, nil
}

func (Context) WorkspaceResolveWorkingPath(ctx context.Context, path string) (map[string]any, error) {
	return nil, nil
}

func (Context) WorkspaceCreatePath(ctx context.Context, path string) (map[string]any, error) {
	return nil, nil
}

func (Context) WorkspaceCreateWorkingPath(ctx context.Context, path string) (map[string]any, error) {
	return nil, nil
}

func (Context) WorkspaceSaveAsEdited(ctx context.Context, sourcePath string, suffix string) (map[string]any, error) {
	return nil, nil
}
