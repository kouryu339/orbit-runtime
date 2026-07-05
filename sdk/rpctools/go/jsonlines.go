package agenttool

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"reflect"
	"strings"
)

const Schema = "corework-agent-tool/v1"

type registeredTool struct {
	Descriptor ToolDescriptor
	Handler    any
}

var toolRegistry []registeredTool

type wireMessage struct {
	Type     string           `json:"type"`
	Schema   string           `json:"schema,omitempty"`
	Request  *executeRequest  `json:"request,omitempty"`
	Metadata *ToolDescriptor  `json:"metadata,omitempty"`
	Tools    []ToolDescriptor `json:"tools,omitempty"`
	ID       string           `json:"id,omitempty"`
	Op       string           `json:"op,omitempty"`
	Args     json.RawMessage  `json:"args,omitempty"`
	OK       bool             `json:"ok,omitempty"`
	Value    any              `json:"value,omitempty"`
	Output   *remoteAIOutput  `json:"output,omitempty"`
	Message  string           `json:"message,omitempty"`
}

type executeRequest struct {
	ToolName          string          `json:"tool_name"`
	ArgsCLI           string          `json:"args_cli"`
	ArgsJSON          json.RawMessage `json:"args_json"`
	CallID            string          `json:"call_id"`
	ToolCallID        string          `json:"tool_call_id"`
	IdempotencyKey    string          `json:"idempotency_key"`
	SessionID         string          `json:"session_id"`
	ProviderID        string          `json:"provider_id"`
	ClusterID         string          `json:"cluster_id"`
	RuntimeInstanceID string          `json:"runtime_instance_id"`
	ConversationID    string          `json:"conversation_id"`
	AgentID           string          `json:"agent_id"`
	TurnID            string          `json:"turn_id"`
	Permissions       []string        `json:"permissions"`
	HostContext       any             `json:"host_context"`
}

type remoteAIOutput struct {
	Result    any           `json:"result"`
	ToAI      string        `json:"to_ai"`
	ErrorCode ToolErrorCode `json:"error_code"`
}

func RegisterTool[T any](descriptor ToolDescriptor, handler func(context.Context, Context, T) AIOutput) {
	toolRegistry = append(toolRegistry, registeredTool{Descriptor: descriptor, Handler: handler})
}

func Serve(address string) error {
	listener, err := net.Listen("tcp", address)
	if err != nil {
		return err
	}
	defer listener.Close()
	for {
		conn, err := listener.Accept()
		if err != nil {
			return err
		}
		go func() {
			_ = serveConn(conn)
		}()
	}
}

func serveConn(conn net.Conn) error {
	defer conn.Close()
	reader := bufio.NewReader(conn)
	writer := bufio.NewWriter(conn)
	line, err := reader.ReadBytes('\n')
	if err != nil {
		return err
	}
	var msg wireMessage
	if err := json.Unmarshal(line, &msg); err != nil {
		return writeWire(writer, wireMessage{Type: "error", Message: err.Error()})
	}
	if msg.Type == "list_tools" {
		return writeWire(writer, wireMessage{
			Type:   "tools",
			Schema: Schema,
			Tools:  registeredToolDescriptors(),
		})
	}
	if msg.Type != "execute" || msg.Request == nil {
		return writeErrorOutput(writer, "", "first message must be list_tools or execute", ToolErrorCodeProtocolError)
	}
	tool, ok := findTool(msg.Request.ToolName)
	if !ok {
		return writeErrorOutput(writer, msg.Request.ToolName, "unknown tool "+msg.Request.ToolName, ToolErrorCodeNotFound)
	}
	output, err := callTool(tool, msg.Request)
	if err != nil {
		return writeErrorOutput(writer, msg.Request.ToolName, err.Error(), ToolErrorCodeInternal)
	}
	return writeWire(writer, wireMessage{
		Type: "ai_output",
		Output: &remoteAIOutput{
			Result:    output.Result,
			ToAI:      output.ToAI,
			ErrorCode: output.ErrorCode,
		},
	})
}

func registeredToolDescriptors() []ToolDescriptor {
	descriptors := make([]ToolDescriptor, 0, len(toolRegistry))
	for _, tool := range toolRegistry {
		descriptors = append(descriptors, tool.Descriptor)
	}
	return descriptors
}

func writeErrorOutput(writer *bufio.Writer, toolName string, message string, code ToolErrorCode) error {
	toAI := "RPC tool call failed: " + message
	if strings.TrimSpace(toolName) != "" {
		toAI = "RPC tool " + toolName + " failed: " + message
	}
	return writeWire(writer, wireMessage{
		Type: "ai_output",
		Output: &remoteAIOutput{
			Result:    map[string]any{"error": message},
			ToAI:      toAI,
			ErrorCode: code,
		},
	})
}

func writeWire(writer *bufio.Writer, msg wireMessage) error {
	payload, err := json.Marshal(msg)
	if err != nil {
		return err
	}
	payload = append(payload, '\n')
	if _, err := writer.Write(payload); err != nil {
		return err
	}
	return writer.Flush()
}

func findTool(name string) (registeredTool, bool) {
	for _, tool := range toolRegistry {
		if tool.Descriptor.Name == name {
			return tool, true
		}
	}
	return registeredTool{}, false
}

func callTool(tool registeredTool, request *executeRequest) (AIOutput, error) {
	args, err := decodeArgs(request.ArgsJSON)
	if err != nil {
		return AIOutput{}, err
	}
	handlerValue := reflect.ValueOf(tool.Handler)
	if handlerValue.Kind() != reflect.Func || handlerValue.Type().NumIn() != 3 {
		return AIOutput{}, errors.New("tool handler must be func(context.Context, agenttool.Context, T) agenttool.AIOutput")
	}
	inputType := handlerValue.Type().In(2)
	input := reflect.New(inputType)
	if len(args) > 0 {
		payload, _ := json.Marshal(args)
		if err := json.Unmarshal(payload, input.Interface()); err != nil {
			return AIOutput{}, fmt.Errorf("decode tool args: %w", err)
		}
	}
	toolCtx := Context{
		CallID:            request.CallID,
		ToolCallID:        request.ToolCallID,
		IdempotencyKey:    request.IdempotencyKey,
		SessionID:         request.SessionID,
		ProviderID:        request.ProviderID,
		ClusterID:         request.ClusterID,
		RuntimeInstanceID: request.RuntimeInstanceID,
		ConversationID:    request.ConversationID,
		AgentID:           request.AgentID,
		TurnID:            request.TurnID,
		Permissions:       append([]string(nil), request.Permissions...),
		HostContext:       request.HostContext,
	}
	result := handlerValue.Call([]reflect.Value{
		reflect.ValueOf(context.Background()),
		reflect.ValueOf(toolCtx),
		input.Elem(),
	})
	if len(result) != 1 {
		return AIOutput{}, errors.New("tool handler must return one value")
	}
	output, ok := result[0].Interface().(AIOutput)
	if !ok {
		return AIOutput{}, errors.New("tool handler must return agenttool.AIOutput")
	}
	if strings.TrimSpace(output.ToAI) == "" {
		return AIOutput{
			Result:    map[string]any{"error": "AIOutput.ToAI must not be empty"},
			ToAI:      "RPC tool " + request.ToolName + " returned an empty to_ai field.",
			ErrorCode: ToolErrorCodeInvalidOutput,
		}, nil
	}
	if output.ErrorCode == ToolErrorCodeUnspecified {
		output.ErrorCode = ToolErrorCodeOK
	}
	return output, nil
}

func decodeArgs(raw json.RawMessage) (map[string]any, error) {
	if len(raw) == 0 {
		return map[string]any{}, nil
	}
	var args map[string]any
	if err := json.Unmarshal(raw, &args); err != nil {
		return nil, err
	}
	return args, nil
}
