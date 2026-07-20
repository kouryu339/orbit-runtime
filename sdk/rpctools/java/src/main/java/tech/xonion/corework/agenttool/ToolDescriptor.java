package tech.xonion.corework.agenttool;

import java.util.List;

public record ToolDescriptor(
    String name,
    String description,
    String sideEffect,
    List<String> requiredCapabilities
) {}
