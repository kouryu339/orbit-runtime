package tech.xonion.corework.agenttool;

import java.util.Map;

@FunctionalInterface
public interface ToolHandler {
    AIOutput execute(ToolContext context, Map<String, Object> arguments) throws Exception;
}
