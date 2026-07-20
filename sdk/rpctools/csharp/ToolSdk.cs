using System.Collections.Concurrent;
using System.Net;
using System.Text.Json;
using System.Threading.Channels;
using Grpc.Core;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.Extensions.DependencyInjection;
using Proto = Corework.AgentTool.V1;

namespace Corework.AgentTool;

public static class Constants
{
    public const string Schema = "corework-agent-tool/v1";
}

public enum ToolErrorCode
{
    Unspecified = 0,
    Ok = 1,
    InvalidArgument = 100,
    MissingArgument = 101,
    PermissionDenied = 102,
    NotFound = 103,
    Conflict = 104,
    Internal = 200,
    Timeout = 201,
    Cancelled = 202,
    Unavailable = 203,
    HostCapabilityDenied = 300,
    HostCapabilityUnsupported = 301,
    HostCallFailed = 302,
    ProtocolError = 400,
    InvalidOutput = 401,
    SchemaMismatch = 402,
}

public sealed class AIOutput
{
    public object? Result { get; set; }
    public string ToAI { get; set; } = string.Empty;
    public ToolErrorCode ErrorCode { get; set; } = ToolErrorCode.Ok;
}

public sealed class ToolParameter
{
    public string Name { get; set; } = string.Empty;
    public string ParamType { get; set; } = string.Empty;
    public bool Required { get; set; }
    public string? DefaultValue { get; set; }
    public string Description { get; set; } = string.Empty;
}

public sealed class ToolOutputField
{
    public string Name { get; set; } = string.Empty;
    public string FieldType { get; set; } = string.Empty;
    public string Description { get; set; } = string.Empty;
}

public sealed class ToolDescriptor
{
    public string Name { get; set; } = string.Empty;
    public string Description { get; set; } = string.Empty;
    public List<ToolParameter> Parameters { get; set; } = [];
    public List<ToolOutputField> Outputs { get; set; } = [];
    public bool Readonly { get; set; }
    public bool Destructive { get; set; }
    public bool Idempotent { get; set; }
    public bool OpenWorld { get; set; }
    public bool Secret { get; set; }
    public string Category { get; set; } = string.Empty;
    public string DisplayName { get; set; } = string.Empty;
    public List<string> RequiredCapabilities { get; set; } = [];
}

public sealed class ToolContext
{
    private readonly string _callId;
    private readonly Func<string, object?, Task<object?>> _hostCall;

    internal ToolContext(string callId, Proto.ExecuteRequest executeRequest, Func<string, object?, Task<object?>> hostCall)
    {
        _callId = callId;
        _hostCall = hostCall;
        ToolCallId = executeRequest.ToolCallId;
        IdempotencyKey = executeRequest.IdempotencyKey;
        SessionId = executeRequest.SessionId;
        ProviderId = executeRequest.ProviderId;
        ClusterId = executeRequest.ClusterId;
        RuntimeInstanceId = executeRequest.RuntimeInstanceId;
        ConversationId = executeRequest.ConversationId;
        AgentId = executeRequest.AgentId;
        TurnId = executeRequest.TurnId;
        Permissions = executeRequest.Permissions.ToArray();
        HostContextJson = executeRequest.HostContextJson;
    }

    public string CallId => _callId;
    public string ToolCallId { get; }
    public string IdempotencyKey { get; }
    public string SessionId { get; }
    public string ProviderId { get; }
    public string ClusterId { get; }
    public string RuntimeInstanceId { get; }
    public string ConversationId { get; }
    public string AgentId { get; }
    public string TurnId { get; }
    public IReadOnlyList<string> Permissions { get; }
    public string HostContextJson { get; }

    public Task<object?> WorkspaceResolvePath(string path) => _hostCall("workspace.resolve_path", new { path });

    public Task<object?> WorkspaceResolveWorkingPath(string path) => _hostCall("workspace.resolve_working_path", new { path });

    public Task<object?> WorkspaceCreatePath(string path) => _hostCall("workspace.create_path", new { path });

    public Task<object?> WorkspaceCreateWorkingPath(string path) => _hostCall("workspace.create_working_path", new { path });

    public Task<object?> WorkspaceSaveAsEdited(string sourcePath, string suffix) =>
        _hostCall("workspace.save_as_edited", new { source_path = sourcePath, suffix });
}

public sealed class ToolArgs(JsonElement root)
{
    public JsonElement Root { get; } = root;

    public string? GetString(string name) =>
        Root.ValueKind == JsonValueKind.Object && Root.TryGetProperty(name, out var value)
            ? value.GetString()
            : null;

    public bool TryGetProperty(string name, out JsonElement value)
    {
        if (Root.ValueKind == JsonValueKind.Object && Root.TryGetProperty(name, out value))
        {
            return true;
        }
        value = default;
        return false;
    }
}

public sealed class ToolApp
{
    private readonly ConcurrentDictionary<string, RegisteredTool> _tools = new();

    public void RegisterTool(ToolDescriptor descriptor, Func<ToolContext, ToolArgs, Task<AIOutput>> handler)
    {
        if (string.IsNullOrWhiteSpace(descriptor.Name))
        {
            throw new ArgumentException("tool descriptor requires a non-empty name", nameof(descriptor));
        }
        _tools[descriptor.Name] = new RegisteredTool(descriptor, handler);
    }

    public async Task Serve(string host, int port, CancellationToken cancellationToken = default)
    {
        var builder = WebApplication.CreateBuilder();
        builder.WebHost.ConfigureKestrel(options =>
        {
            var address = IPAddress.TryParse(host, out var parsed) ? parsed : IPAddress.Loopback;
            options.Listen(address, port, listenOptions => listenOptions.Protocols = Microsoft.AspNetCore.Server.Kestrel.Core.HttpProtocols.Http2);
        });
        builder.Services.AddGrpc();
        builder.Services.AddSingleton(new AgentToolGrpcService(_tools));

        var app = builder.Build();
        app.MapGrpcService<AgentToolGrpcService>();
        await app.RunAsync(cancellationToken);
    }

    private sealed record RegisteredTool(ToolDescriptor Descriptor, Func<ToolContext, ToolArgs, Task<AIOutput>> Handler);

    private sealed record HostRequest(string Id, string Op, object? Args, TaskCompletionSource<object?> Result);

    private sealed class AgentToolGrpcService(ConcurrentDictionary<string, RegisteredTool> tools)
        : Proto.AgentToolService.AgentToolServiceBase
    {
        public override Task<Proto.ListToolsResponse> ListTools(Proto.ListToolsRequest request, ServerCallContext context)
        {
            if (request.AcceptedSchema.Count > 0 && !request.AcceptedSchema.Contains(Constants.Schema))
            {
                throw new RpcException(new Status(StatusCode.FailedPrecondition, $"unsupported schema; expected {Constants.Schema}"));
            }

            var response = new Proto.ListToolsResponse { Schema = Constants.Schema };
            response.Tools.AddRange(tools.Values.Select(tool => ToProto(tool.Descriptor)));
            return Task.FromResult(response);
        }

        public override async Task Execute(
            IAsyncStreamReader<Proto.ToolStreamMessage> requestStream,
            IServerStreamWriter<Proto.ToolStreamMessage> responseStream,
            ServerCallContext context)
        {
            if (!await requestStream.MoveNext(context.CancellationToken))
            {
                await responseStream.WriteAsync(ToolError("", "missing ExecuteRequest", ToolErrorCode.ProtocolError));
                return;
            }

            var first = requestStream.Current;
            if (first.MessageCase != Proto.ToolStreamMessage.MessageOneofCase.ExecuteRequest ||
                string.IsNullOrWhiteSpace(first.ExecuteRequest.ToolName))
            {
                await responseStream.WriteAsync(ToolError(first.CallId, "first stream message must be ExecuteRequest", ToolErrorCode.ProtocolError));
                return;
            }

            if (!tools.TryGetValue(first.ExecuteRequest.ToolName, out var tool))
            {
                await responseStream.WriteAsync(ToolError(first.CallId, $"unknown tool {first.ExecuteRequest.ToolName}", ToolErrorCode.NotFound));
                return;
            }

            JsonElement argsRoot;
            try
            {
                argsRoot = JsonDocument.Parse(string.IsNullOrWhiteSpace(first.ExecuteRequest.ArgsJson) ? "{}" : first.ExecuteRequest.ArgsJson).RootElement.Clone();
                if (argsRoot.ValueKind != JsonValueKind.Object)
                {
                    throw new JsonException("args_json must encode an object");
                }
            }
            catch (Exception ex)
            {
                await responseStream.WriteAsync(ToolError(first.CallId, $"invalid args_json: {ex.Message}", ToolErrorCode.InvalidArgument));
                return;
            }

            var hostRequests = Channel.CreateUnbounded<HostRequest>();
            var toolContext = new ToolContext(first.CallId, first.ExecuteRequest, async (op, hostArgs) =>
            {
                var pending = new HostRequest(
                    $"{first.CallId}-{Guid.NewGuid():N}",
                    op,
                    hostArgs,
                    new TaskCompletionSource<object?>(TaskCreationOptions.RunContinuationsAsynchronously));
                await hostRequests.Writer.WriteAsync(pending, context.CancellationToken);
                return await pending.Result.Task;
            });

            var handlerTask = Task.Run(() => tool.Handler(toolContext, new ToolArgs(argsRoot)), context.CancellationToken);

            while (!handlerTask.IsCompleted)
            {
                var readTask = hostRequests.Reader.ReadAsync(context.CancellationToken).AsTask();
                var completed = await Task.WhenAny(handlerTask, readTask);
                if (completed == handlerTask)
                {
                    break;
                }
                await HandleHostRequest(first.CallId, readTask.Result, requestStream, responseStream, context.CancellationToken);
            }

            try
            {
                var output = await handlerTask;
                if (string.IsNullOrWhiteSpace(output.ToAI))
                {
                    await responseStream.WriteAsync(ToolError(first.CallId, "AIOutput.to_ai must be non-empty", ToolErrorCode.InvalidOutput));
                    return;
                }

                await responseStream.WriteAsync(new Proto.ToolStreamMessage
                {
                    CallId = first.CallId,
                    AiOutput = new Proto.AIOutput
                    {
                        ResultJson = JsonSerializer.Serialize(output.Result),
                        ToAi = output.ToAI,
                        ErrorCode = (Proto.ToolErrorCode)output.ErrorCode,
                    },
                });
            }
            catch (Exception ex)
            {
                await responseStream.WriteAsync(ToolError(first.CallId, ex.Message, ToolErrorCode.Internal));
            }
        }

        private static async Task HandleHostRequest(
            string callId,
            HostRequest hostRequest,
            IAsyncStreamReader<Proto.ToolStreamMessage> requestStream,
            IServerStreamWriter<Proto.ToolStreamMessage> responseStream,
            CancellationToken cancellationToken)
        {
            await responseStream.WriteAsync(new Proto.ToolStreamMessage
            {
                CallId = callId,
                HostCall = new Proto.HostCall
                {
                    Id = hostRequest.Id,
                    Op = hostRequest.Op,
                    ArgsJson = JsonSerializer.Serialize(hostRequest.Args),
                },
            }, cancellationToken);

            if (!await requestStream.MoveNext(cancellationToken))
            {
                hostRequest.Result.TrySetException(new InvalidOperationException("stream closed before HostResult"));
                return;
            }

            var message = requestStream.Current;
            if (message.CallId != callId ||
                message.MessageCase != Proto.ToolStreamMessage.MessageOneofCase.HostResult ||
                message.HostResult.Id != hostRequest.Id)
            {
                hostRequest.Result.TrySetException(new InvalidOperationException("invalid HostResult message"));
                return;
            }

            var result = message.HostResult;
            object? value = null;
            if (!string.IsNullOrWhiteSpace(result.ValueJson))
            {
                value = JsonSerializer.Deserialize<JsonElement>(result.ValueJson).Clone();
            }

            if (result.Ok)
            {
                hostRequest.Result.TrySetResult(value);
            }
            else
            {
                hostRequest.Result.TrySetException(new InvalidOperationException(
                    $"host call {hostRequest.Op} failed with code {result.Code}: {result.ValueJson}"));
            }
        }

        private static Proto.ToolDescriptor ToProto(ToolDescriptor descriptor)
        {
            var proto = new Proto.ToolDescriptor
            {
                Name = descriptor.Name,
                Description = descriptor.Description,
                Readonly = descriptor.Readonly,
                Destructive = descriptor.Destructive,
                Idempotent = descriptor.Idempotent,
                OpenWorld = descriptor.OpenWorld,
                Secret = descriptor.Secret,
                Category = descriptor.Category,
                DisplayName = descriptor.DisplayName,
            };
            proto.Parameters.AddRange(descriptor.Parameters.Select(parameter =>
            {
                var protoParameter = new Proto.ToolParameter
                {
                    Name = parameter.Name,
                    ParamType = parameter.ParamType,
                    Required = parameter.Required,
                    Description = parameter.Description,
                };
                if (parameter.DefaultValue is not null)
                {
                    protoParameter.DefaultValue = parameter.DefaultValue;
                }
                return protoParameter;
            }));
            proto.Outputs.AddRange(descriptor.Outputs.Select(output => new Proto.ToolOutputField
            {
                Name = output.Name,
                FieldType = output.FieldType,
                Description = output.Description,
            }));
            proto.RequiredCapabilities.AddRange(descriptor.RequiredCapabilities);
            return proto;
        }

        private static Proto.ToolStreamMessage ToolError(string callId, string message, ToolErrorCode code) => new()
        {
            CallId = callId,
            Error = new Proto.ToolError
            {
                Message = message,
                Code = (Proto.ToolErrorCode)code,
            },
        };
    }
}
