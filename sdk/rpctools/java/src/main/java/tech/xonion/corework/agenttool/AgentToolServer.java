package tech.xonion.corework.agenttool;

import java.util.LinkedHashMap;
import java.util.Map;

public final class AgentToolServer {
    private final Map<String, RegisteredTool> tools = new LinkedHashMap<>();

    public AgentToolServer register(ToolDescriptor descriptor, ToolHandler handler) {
        tools.put(descriptor.name(), new RegisteredTool(descriptor, handler));
        return this;
    }

    public Map<String, ToolDescriptor> descriptors() {
        var result = new LinkedHashMap<String, ToolDescriptor>();
        for (var entry : tools.entrySet()) {
            result.put(entry.getKey(), entry.getValue().descriptor());
        }
        return Map.copyOf(result);
    }

    public AIOutput execute(String toolName, ToolContext context, Map<String, Object> arguments) throws Exception {
        var tool = tools.get(toolName);
        if (tool == null) {
            return AIOutput.error("Tool is not registered: " + toolName, Map.of());
        }
        return tool.handler().execute(context, arguments);
    }

    private record RegisteredTool(ToolDescriptor descriptor, ToolHandler handler) {}
}
