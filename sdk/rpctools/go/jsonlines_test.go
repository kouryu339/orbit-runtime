package agenttool

import (
	"bufio"
	"context"
	"encoding/json"
	"net"
	"testing"
	"time"
)

type testEchoRequest struct {
	Text string `json:"text"`
}

func TestServeExecuteReturnsAIOutput(t *testing.T) {
	toolRegistry = nil
	RegisterTool(ToolDescriptor{Name: "Echo", Description: "Echo text."}, func(ctx context.Context, toolCtx Context, req testEchoRequest) AIOutput {
		if toolCtx.ConversationID != "conv-test" {
			t.Fatalf("toolCtx.ConversationID = %q, want conv-test", toolCtx.ConversationID)
		}
		if toolCtx.AgentID != "agent-test" {
			t.Fatalf("toolCtx.AgentID = %q, want agent-test", toolCtx.AgentID)
		}
		return AIOutput{Result: map[string]any{"text": req.Text}, ToAI: "echo: " + req.Text}
	})

	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := listener.Addr().String()
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}

	errCh := make(chan error, 1)
	go func() {
		errCh <- Serve(address)
	}()

	deadline := time.Now().Add(2 * time.Second)
	var conn net.Conn
	for time.Now().Before(deadline) {
		conn, err = net.Dial("tcp", address)
		if err == nil {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err != nil {
		t.Fatalf("dial tool server: %v", err)
	}
	defer conn.Close()

	request := wireMessage{
		Type: "execute",
		Request: &executeRequest{
			ToolName:       "Echo",
			ArgsJSON:       json.RawMessage(`{"text":"hello"}`),
			ConversationID: "conv-test",
			AgentID:        "agent-test",
		},
	}
	payload, err := json.Marshal(request)
	if err != nil {
		t.Fatal(err)
	}
	payload = append(payload, '\n')
	if _, err := conn.Write(payload); err != nil {
		t.Fatal(err)
	}

	line, err := bufio.NewReader(conn).ReadBytes('\n')
	if err != nil {
		t.Fatal(err)
	}
	var response wireMessage
	if err := json.Unmarshal(line, &response); err != nil {
		t.Fatal(err)
	}
	if response.Type != "ai_output" {
		t.Fatalf("response type = %q, want ai_output", response.Type)
	}
	if response.Output == nil {
		t.Fatal("response output is nil")
	}
	if response.Output.ToAI != "echo: hello" {
		t.Fatalf("to_ai = %q", response.Output.ToAI)
	}
	if response.Output.ErrorCode != ToolErrorCodeOK {
		t.Fatalf("error_code = %d", response.Output.ErrorCode)
	}

	select {
	case err := <-errCh:
		t.Fatalf("server exited unexpectedly: %v", err)
	default:
	}
}

func TestServeListToolsReturnsRegisteredDescriptors(t *testing.T) {
	toolRegistry = nil
	RegisterTool(ToolDescriptor{
		Name:        "FrontendNavigate",
		DisplayName: "Navigate Frontend",
		Description: "Publish a frontend navigation intent.",
		Readonly:    true,
		Idempotent:  true,
		Parameters: []ToolParameter{
			{Name: "view", ParamType: "String", Required: true},
		},
	}, func(ctx context.Context, toolCtx Context, req testEchoRequest) AIOutput {
		return AIOutput{ToAI: "ok"}
	})

	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := listener.Addr().String()
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	go func() { _ = Serve(address) }()

	deadline := time.Now().Add(2 * time.Second)
	var conn net.Conn
	for time.Now().Before(deadline) {
		conn, err = net.Dial("tcp", address)
		if err == nil {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err != nil {
		t.Fatalf("dial tool server: %v", err)
	}
	defer conn.Close()

	payload, err := json.Marshal(wireMessage{Type: "list_tools"})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := conn.Write(append(payload, '\n')); err != nil {
		t.Fatal(err)
	}

	line, err := bufio.NewReader(conn).ReadBytes('\n')
	if err != nil {
		t.Fatal(err)
	}
	var response wireMessage
	if err := json.Unmarshal(line, &response); err != nil {
		t.Fatal(err)
	}
	if response.Type != "tools" || response.Schema != Schema {
		t.Fatalf("unexpected list_tools response: %#v", response)
	}
	if len(response.Tools) != 1 || response.Tools[0].Name != "FrontendNavigate" {
		t.Fatalf("tools = %#v", response.Tools)
	}
	if len(response.Tools[0].Parameters) != 1 || response.Tools[0].Parameters[0].Name != "view" {
		t.Fatalf("parameters = %#v", response.Tools[0].Parameters)
	}
}
