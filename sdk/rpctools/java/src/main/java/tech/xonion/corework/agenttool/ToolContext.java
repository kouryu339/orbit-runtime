package tech.xonion.corework.agenttool;

import java.util.List;

public record ToolContext(
    String callId,
    String toolCallId,
    String idempotencyKey,
    String sessionId,
    String providerId,
    String clusterId,
    String runtimeInstanceId,
    String conversationId,
    String agentId,
    String turnId,
    String workflowId,
    String workflowRunId,
    String jevName,
    String jevRunId,
    long jevSnapshotRevision,
    String nodeId,
    List<String> permissions,
    String hostContextJson
) {}
