package tech.xonion.corework.agenttool;

import java.util.Map;

public record AIOutput(String toAi, Map<String, Object> data, boolean success) {
    public static AIOutput ok(String toAi, Map<String, Object> data) {
        return new AIOutput(toAi, data, true);
    }

    public static AIOutput error(String toAi, Map<String, Object> data) {
        return new AIOutput(toAi, data, false);
    }
}
