package tech.xonion.corework.agenttool;

import java.util.List;

public record ToolDescriptor(
    String name,
    String description,
    String sideEffect,
    boolean workflowEnabled,
    List<String> requiredCapabilities
) {
    public ToolDescriptor(
        String name,
        String description,
        String sideEffect,
        List<String> requiredCapabilities
    ) {
        this(name, description, sideEffect, true, requiredCapabilities);
    }
}
