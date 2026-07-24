#ifndef COREWORK_RUNTIME_RUNTIME_HPP
#define COREWORK_RUNTIME_RUNTIME_HPP

#include <cstdint>
#include <initializer_list>
#include <map>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#endif

namespace corework {
namespace runtime {

constexpr uint32_t kAgentRuntimeAbiVersion = 1;

constexpr int kOk = 0;
constexpr int kErrInvalidArgument = 1;
constexpr int kErrInvalidHandle = 2;
constexpr int kErrBadState = 3;
constexpr int kErrTimeout = 4;
constexpr int kErrUnsupported = 5;
constexpr int kErrRuntime = 100;
constexpr int kErrPanic = 101;

constexpr const char* kLedgerDeltaEventType = "conversation.ledger_delta";
constexpr const char* kLedgerDeltaSchema = "agent-runtime-ledger-delta/v1";
constexpr const char* kStateDeltaEventType = "conversation.state_delta";
constexpr const char* kStateDeltaSchema = "agent-runtime-state-delta/v1";
constexpr const char* kConversationCreatedEventType = "conversation:created";
constexpr const char* kConversationClosedEventType = "conversation:closed";
constexpr const char* kFrontendStateSnapshotEventType = "frontend:state_snapshot";
constexpr const char* kWorkflowResourceChangedEventType = "workflow.resource_changed";
constexpr const char* kWorkflowExecutionCompletedEventType = "workflow.execution_completed";

using AgentRuntimeHandle = uint64_t;

namespace detail {

inline bool is_space(char ch)
{
    return ch == ' ' || ch == '\n' || ch == '\r' || ch == '\t';
}

inline std::string trim(std::string value)
{
    while (!value.empty() && is_space(value.front())) {
        value.erase(value.begin());
    }
    while (!value.empty() && is_space(value.back())) {
        value.pop_back();
    }
    return value;
}

inline std::string json_escape(const std::string& value)
{
    std::ostringstream out;
    for (unsigned char ch : value) {
        switch (ch) {
        case '\\':
            out << "\\\\";
            break;
        case '"':
            out << "\\\"";
            break;
        case '\b':
            out << "\\b";
            break;
        case '\f':
            out << "\\f";
            break;
        case '\n':
            out << "\\n";
            break;
        case '\r':
            out << "\\r";
            break;
        case '\t':
            out << "\\t";
            break;
        default:
            if (ch < 0x20) {
                static const char* digits = "0123456789abcdef";
                out << "\\u00" << digits[(ch >> 4) & 0x0f] << digits[ch & 0x0f];
            } else {
                out << static_cast<char>(ch);
            }
            break;
        }
    }
    return out.str();
}

inline std::string quote(const std::string& value)
{
    return "\"" + json_escape(value) + "\"";
}

inline bool parse_json_string_at(
    const std::string& json,
    std::size_t quote_pos,
    std::string* out,
    std::size_t* next_pos)
{
    if (quote_pos >= json.size() || json[quote_pos] != '"') {
        return false;
    }
    std::ostringstream value;
    bool escape = false;
    for (std::size_t pos = quote_pos + 1; pos < json.size(); ++pos) {
        char ch = json[pos];
        if (escape) {
            switch (ch) {
            case '"':
            case '\\':
            case '/':
                value << ch;
                break;
            case 'b':
                value << '\b';
                break;
            case 'f':
                value << '\f';
                break;
            case 'n':
                value << '\n';
                break;
            case 'r':
                value << '\r';
                break;
            case 't':
                value << '\t';
                break;
            case 'u':
                value << "\\u";
                break;
            default:
                value << ch;
                break;
            }
            escape = false;
            continue;
        }
        if (ch == '\\') {
            escape = true;
            continue;
        }
        if (ch == '"') {
            if (out) {
                *out = value.str();
            }
            if (next_pos) {
                *next_pos = pos + 1;
            }
            return true;
        }
        value << ch;
    }
    return false;
}

inline std::string object_field(const std::string& json, const std::string& field)
{
    std::string key = quote(field);
    std::size_t pos = json.find(key);
    if (pos == std::string::npos) {
        return {};
    }
    pos = json.find(':', pos + key.size());
    if (pos == std::string::npos) {
        return {};
    }
    ++pos;
    while (pos < json.size() && is_space(json[pos])) {
        ++pos;
    }
    if (pos >= json.size()) {
        return {};
    }

    std::size_t start = pos;
    int depth = 0;
    bool in_string = false;
    bool escape = false;
    for (; pos < json.size(); ++pos) {
        char ch = json[pos];
        if (in_string) {
            if (escape) {
                escape = false;
            } else if (ch == '\\') {
                escape = true;
            } else if (ch == '"') {
                in_string = false;
            }
            continue;
        }
        if (ch == '"') {
            in_string = true;
            continue;
        }
        if (ch == '{' || ch == '[') {
            ++depth;
            continue;
        }
        if (ch == '}' || ch == ']') {
            if (depth == 0) {
                break;
            }
            --depth;
            continue;
        }
        if (depth == 0 && ch == ',') {
            break;
        }
    }
    return trim(json.substr(start, pos - start));
}

inline std::string find_string_field(const std::string& json, const std::string& field)
{
    std::string key = quote(field);
    std::size_t pos = json.find(key);
    if (pos == std::string::npos) {
        return {};
    }
    pos = json.find(':', pos + key.size());
    if (pos == std::string::npos) {
        return {};
    }
    ++pos;
    while (pos < json.size() && is_space(json[pos])) {
        ++pos;
    }
    std::string value;
    return parse_json_string_at(json, pos, &value, nullptr) ? value : std::string();
}

inline bool json_array_contains_string(
    const std::string& json,
    const std::string& array_field,
    const std::string& needle)
{
    std::string array = object_field(json, array_field);
    if (array.empty() || array.front() != '[') {
        return false;
    }
    for (std::size_t pos = 1; pos < array.size();) {
        while (pos < array.size() && (is_space(array[pos]) || array[pos] == ',')) {
            ++pos;
        }
        if (pos >= array.size() || array[pos] == ']') {
            break;
        }
        if (array[pos] != '"') {
            ++pos;
            continue;
        }
        std::string value;
        std::size_t next = pos + 1;
        if (parse_json_string_at(array, pos, &value, &next) && value == needle) {
            return true;
        }
        pos = next;
    }
    return false;
}

inline std::string borrowed_string(const char* pointer)
{
    return pointer ? std::string(pointer) : std::string();
}

} // namespace detail

struct RuntimeCreateOptions {
    std::string schema = "agent-runtime-create-options/v1";
    std::string log_level = "info";
    std::string language = "zh-CN";
    std::string restore_policy = "strict";
    std::optional<std::string> data_dir;

    std::string to_json() const
    {
        std::string json = std::string("{\"schema\":") + detail::quote(schema)
            + ",\"log_level\":" + detail::quote(log_level)
            + ",\"language\":" + detail::quote(language)
            + ",\"restore_policy\":" + detail::quote(restore_policy);
        if (data_dir.has_value()) {
            json += ",\"data_dir\":" + detail::quote(*data_dir);
        }
        json += "}";
        return json;
    }
};

struct RuntimeCommandOptions {
    std::optional<std::string> request_id;
    std::optional<std::string> command_id;
};

struct ConversationSpawnOptions {
    std::string schema = "agent-runtime-conversation-spawn/v1";
    std::string cluster_id;
    std::string tool_host_context_json;
    std::string permissions_json;
};

struct ConversationInfo {
    std::string json;
    std::string conversation_id;
    std::string scope_id;
    std::string tenant_id;
    std::string user_id;
    std::string created_at;
};

struct AdmissionResult {
    std::string json;
    std::string command_id;
    std::string decision;
    std::string reason;

    bool accepted() const
    {
        return decision == "accepted";
    }
};

struct RuntimeEvent {
    std::string json;
    std::string type;
    std::string event_line;
    std::string conversation_id;
    std::string payload_json;
};

inline bool is_public_runtime_event(const RuntimeEvent& event)
{
    return event.type == kConversationCreatedEventType
        || event.type == kConversationClosedEventType
        || event.type == kLedgerDeltaEventType
        || event.type == kStateDeltaEventType
        || event.type == kFrontendStateSnapshotEventType
        || event.type == kWorkflowResourceChangedEventType
        || event.type == kWorkflowExecutionCompletedEventType;
}

inline std::string conversation_id_from_event(const RuntimeEvent& event)
{
    if (!event.conversation_id.empty()) {
        return event.conversation_id;
    }
    return detail::find_string_field(event.payload_json, "conversation_id");
}

inline bool is_workflow_event(const RuntimeEvent& event)
{
    return event.event_line == "workflow"
        || detail::find_string_field(event.payload_json, "event_line") == "workflow";
}

inline std::string workflow_id_from_event(const RuntimeEvent& event)
{
    return is_workflow_event(event)
        ? detail::find_string_field(event.payload_json, "workflow_id")
        : std::string();
}

enum class ConversationPosition {
    Current,
    Background,
    DataOnly,
};

struct ConversationRegistryEntry {
    std::string conversation_id;
    ConversationPosition position = ConversationPosition::Background;
    bool waiting = false;
};

struct ConversationRegistryAction {
    std::string kind;
    std::string conversation_id;
};

class ConversationRegistry {
public:
    void track(
        const std::string& conversation_id,
        ConversationPosition position = ConversationPosition::Background)
    {
        entries_[conversation_id] = ConversationRegistryEntry { conversation_id, position, false };
        if (position == ConversationPosition::Current) {
            current_id_ = conversation_id;
        }
    }

    std::vector<ConversationRegistryAction> set_current(const std::string& conversation_id)
    {
        std::vector<ConversationRegistryAction> actions;
        if (!current_id_.empty()) {
            auto found = entries_.find(current_id_);
            if (found != entries_.end()) {
                found->second.position = ConversationPosition::Background;
                if (found->second.waiting) {
                    actions.push_back({ "close_background", found->second.conversation_id });
                }
            }
        }
        current_id_ = conversation_id;
        if (!conversation_id.empty()) {
            track(conversation_id, ConversationPosition::Current);
        }
        return actions;
    }

    std::vector<ConversationRegistryAction> observe_event(const RuntimeEvent& event)
    {
        std::string conversation_id = conversation_id_from_event(event);
        if (conversation_id.empty()) {
            return {};
        }
        if (event.type == kConversationCreatedEventType) {
            if (entries_.find(conversation_id) == entries_.end()) {
                track(conversation_id);
            }
            return {};
        }
        if (event.type == kConversationClosedEventType) {
            entries_.erase(conversation_id);
            if (current_id_ == conversation_id) {
                current_id_.clear();
            }
            return {};
        }
        if (event.type == kStateDeltaEventType) {
            auto found = entries_.find(conversation_id);
            if (found == entries_.end()) {
                return {};
            }
            found->second.waiting = event_marks_waiting(event);
            if (found->second.waiting
                && found->second.position == ConversationPosition::Background) {
                return { { "close_background", conversation_id } };
            }
        }
        return {};
    }

private:
    static bool event_marks_waiting(const RuntimeEvent& event)
    {
        return detail::find_string_field(event.payload_json, "status") == "waiting"
            || detail::find_string_field(event.payload_json, "state") == "waiting";
    }

    std::string current_id_;
    std::map<std::string, ConversationRegistryEntry> entries_;
};

class RuntimeError : public std::runtime_error {
public:
    RuntimeError(int code, std::string message, std::string error_json = {})
        : std::runtime_error(std::move(message))
        , code_(code)
        , error_json_(std::move(error_json))
    {
    }

    int code() const noexcept
    {
        return code_;
    }

    const std::string& error_json() const noexcept
    {
        return error_json_;
    }

private:
    int code_;
    std::string error_json_;
};

class RuntimeTimeout : public RuntimeError {
public:
    RuntimeTimeout(int code, std::string message, std::string error_json = {})
        : RuntimeError(code, std::move(message), std::move(error_json))
    {
    }
};

class RuntimeStateError : public RuntimeError {
public:
    RuntimeStateError(int code, std::string message, std::string error_json = {})
        : RuntimeError(code, std::move(message), std::move(error_json))
    {
    }
};

class UnsupportedCommand : public RuntimeError {
public:
    UnsupportedCommand(int code, std::string message, std::string error_json = {})
        : RuntimeError(code, std::move(message), std::move(error_json))
    {
    }
};

class Runtime {
public:
    Runtime() = default;

    static Runtime load(
        const std::string& library_path,
        const RuntimeCreateOptions& options = RuntimeCreateOptions(),
        std::initializer_list<std::string> required_commands = {})
    {
        Runtime runtime;
        runtime.load_library(library_path);
        runtime.require_commands(required_commands);
        runtime.create(options.to_json());
        return runtime;
    }

    static Runtime load_json(
        const std::string& library_path,
        const std::string& create_options_json,
        std::initializer_list<std::string> required_commands = {})
    {
        Runtime runtime;
        runtime.load_library(library_path);
        runtime.require_commands(required_commands);
        runtime.create(create_options_json);
        return runtime;
    }

    Runtime(Runtime&& other) noexcept
    {
        move_from(other);
    }

    Runtime& operator=(Runtime&& other) noexcept
    {
        if (this != &other) {
            if (close_noexcept()) {
                unload_library();
            } else {
                library_ = nullptr;
            }
            move_from(other);
        }
        return *this;
    }

    Runtime(const Runtime&) = delete;
    Runtime& operator=(const Runtime&) = delete;

    ~Runtime()
    {
        if (close_noexcept()) {
            unload_library();
        }
    }

    const std::string& version() const
    {
        return version_;
    }

    const std::string& capabilities_json() const
    {
        return capabilities_json_;
    }

    AgentRuntimeHandle handle() const
    {
        return handle_;
    }

    bool is_open() const
    {
        return handle_ != 0;
    }

    bool supports_command(const std::string& command_type) const
    {
        return detail::json_array_contains_string(capabilities_json_, "commands", command_type);
    }

    void require_command(const std::string& command_type) const
    {
        if (!supports_command(command_type)) {
            throw UnsupportedCommand(
                kErrUnsupported,
                "runtime does not support required command: " + command_type,
                capabilities_json_);
        }
    }

    void require_commands(std::initializer_list<std::string> command_types) const
    {
        for (const auto& command_type : command_types) {
            require_command(command_type);
        }
    }

    void register_resources_path(const std::string& path)
    {
        invoke("runtime.register_resources", input_payload(path));
    }

    void register_resources_json(const std::string& registration_json)
    {
        invoke("runtime.register_resources", registration_payload(registration_json));
    }

    std::string create_workflow_draft(const std::string& resource_json)
    {
        return invoke("workflow.create", std::string("{\"resource\":") + object_or_empty(resource_json) + "}");
    }

    std::string read_workflow(const std::string& id)
    {
        return invoke("workflow.read", std::string("{\"id\":") + detail::quote(id) + "}");
    }

    std::string register_workflow_draft(
        const std::string& id,
        std::optional<uint64_t> expected_revision = std::nullopt,
        const std::string& name = "")
    {
        auto payload = std::string("{\"id\":") + detail::quote(id);
        if (expected_revision) payload += ",\"expected_revision\":" + std::to_string(*expected_revision);
        if (!name.empty()) payload += ",\"name\":" + detail::quote(name);
        return invoke("workflow.register", payload + "}");
    }

    std::string update_workflow(
        const std::string& resource_json,
        std::optional<uint64_t> expected_revision = std::nullopt)
    {
        auto payload = std::string("{\"resource\":") + object_or_empty(resource_json);
        if (expected_revision) payload += ",\"expected_revision\":" + std::to_string(*expected_revision);
        return invoke("workflow.update", payload + "}");
    }

    std::string compile_workflow_draft(const std::string& id)
    {
        return invoke("workflow.compile", std::string("{\"id\":") + detail::quote(id) + "}");
    }

    std::string workflow_script_to_blueprint(const std::string& script)
    {
        return invoke("workflow.convert.script_to_blueprint", std::string("{\"script\":") + detail::quote(script) + "}");
    }

    std::string workflow_blueprint_to_script(const std::string& blueprint_json)
    {
        return invoke("workflow.convert.blueprint_to_script", std::string("{\"blueprint\":") + object_or_empty(blueprint_json) + "}");
    }

    std::string delete_workflow(
        const std::string& id,
        std::optional<uint64_t> expected_revision = std::nullopt)
    {
        auto payload = std::string("{\"id\":") + detail::quote(id);
        if (expected_revision) payload += ",\"expected_revision\":" + std::to_string(*expected_revision);
        return invoke("workflow.delete", payload + "}");
    }

    std::string list_workflows(const std::string& kind = "")
    {
        return invoke("workflow.list", kind.empty() ? "{}" : std::string("{\"kind\":") + detail::quote(kind) + "}");
    }

    std::string execute_workflow(
        const std::string& id,
        const std::string& inputs_json = "{}",
        bool trace = false)
    {
        return invoke(
            "workflow.execute",
            std::string("{\"id\":") + detail::quote(id)
                + ",\"inputs\":" + object_or_empty(inputs_json)
                + ",\"trace\":" + (trace ? "true" : "false") + "}");
    }

    std::string execute_workflow_in_context(
        const std::string& id,
        const std::string& conversation_id,
        const std::string& agent_id,
        const std::string& inputs_json = "{}",
        bool trace = false)
    {
        return invoke(
            "workflow.execute",
            std::string("{\"id\":") + detail::quote(id)
                + ",\"inputs\":" + object_or_empty(inputs_json)
                + ",\"trace\":" + (trace ? "true" : "false")
                + ",\"conversation_id\":" + detail::quote(conversation_id)
                + ",\"agent_id\":" + detail::quote(agent_id) + "}");
    }

    std::string test_workflow_draft(
        const std::string& id,
        const std::string& inputs_json = "{}",
        bool trace = false)
    {
        return invoke(
            "workflow.execute",
            std::string("{\"id\":") + detail::quote(id)
                + ",\"mode\":\"test\",\"inputs\":" + object_or_empty(inputs_json)
                + ",\"trace\":" + (trace ? "true" : "false") + "}");
    }

    std::string test_workflow_draft_in_context(
        const std::string& id,
        const std::string& conversation_id,
        const std::string& agent_id,
        const std::string& inputs_json = "{}",
        bool trace = false)
    {
        return invoke(
            "workflow.execute",
            std::string("{\"id\":") + detail::quote(id)
                + ",\"mode\":\"test\",\"inputs\":" + object_or_empty(inputs_json)
                + ",\"trace\":" + (trace ? "true" : "false")
                + ",\"conversation_id\":" + detail::quote(conversation_id)
                + ",\"agent_id\":" + detail::quote(agent_id) + "}");
    }

    std::string execute_workflow_script(
        const std::string& script,
        const std::string& inputs_json = "{}",
        bool trace = false)
    {
        return invoke(
            "workflow.execute_script",
            std::string("{\"script\":") + detail::quote(script)
                + ",\"inputs\":" + object_or_empty(inputs_json)
                + ",\"trace\":" + (trace ? "true" : "false") + "}");
    }

    std::string execute_workflow_script_in_context(
        const std::string& script,
        const std::string& conversation_id,
        const std::string& agent_id,
        const std::string& inputs_json = "{}",
        bool trace = false)
    {
        return invoke(
            "workflow.execute_script",
            std::string("{\"script\":") + detail::quote(script)
                + ",\"inputs\":" + object_or_empty(inputs_json)
                + ",\"trace\":" + (trace ? "true" : "false")
                + ",\"conversation_id\":" + detail::quote(conversation_id)
                + ",\"agent_id\":" + detail::quote(agent_id) + "}");
    }

    void register_llm_path(const std::string& path)
    {
        invoke("runtime.register_llm", input_payload(path));
    }

    void reload_llm_path(const std::string& path)
    {
        invoke("runtime.reload_llm", input_payload(path));
    }

    void register_llm_json(const std::string& registration_json)
    {
        invoke("runtime.register_llm", registration_payload(registration_json));
    }

    void reload_llm_json(const std::string& registration_json)
    {
        invoke("runtime.reload_llm", registration_payload(registration_json));
    }

    void register_agent_cluster_path(const std::string& path)
    {
        invoke("runtime.register_agent_cluster", input_payload(path));
    }

    void register_agent_cluster_json(const std::string& registration_json)
    {
        invoke("runtime.register_agent_cluster", registration_payload(registration_json));
    }

    void set_auth_context_json(const std::string& context_json)
    {
        invoke("runtime.set_auth_context", std::string("{\"context\":") + context_json + "}");
    }

    void configure_providers_path(const std::string& path)
    {
        invoke("runtime.configure_providers", input_payload(path));
    }

    void configure_providers_json(const std::string& registration_json)
    {
        invoke("runtime.configure_providers", registration_payload(registration_json));
    }

    std::string provider_definitions()
    {
        return invoke("runtime.get_provider_definitions");
    }

    std::string tool_definitions()
    {
        return invoke("runtime.get_tool_definitions");
    }

    std::string workflow_node_definitions()
    {
        return invoke("runtime.get_workflow_node_definitions");
    }

    std::string agent_cluster_definitions()
    {
        return invoke("runtime.get_agent_cluster_definitions");
    }

    std::string rpc_endpoint_definitions()
    {
        return invoke("runtime.get_rpc_endpoint_definitions");
    }

    void set_current_model(uint32_t model_uid)
    {
        invoke("runtime.set_current_model", std::string("{\"model_uid\":") + std::to_string(model_uid) + "}");
    }

    void set_language(const std::string& language)
    {
        invoke("runtime.set_language", std::string("{\"language\":") + detail::quote(language) + "}");
    }

    void start()
    {
        ensure_open();
        int code = start_(handle_);
        raise_for_code(code, "runtime start");
    }

    ConversationInfo spawn_conversation(const ConversationSpawnOptions& options)
    {
        return conversation_info_from_result(invoke("conversation.spawn", spawn_payload(options)));
    }

    ConversationInfo spawn_conversation_json(const std::string& spawn_json)
    {
        return conversation_info_from_result(invoke("conversation.spawn", spawn_json));
    }

    ConversationInfo spawn_conversation_from_snapshot(
        const std::string& spawn_json,
        const std::string& snapshot_json)
    {
        return conversation_info_from_result(
            invoke(
                "conversation.spawn_from_snapshot",
                std::string("{\"spawn\":") + spawn_json + ",\"snapshot\":" + snapshot_json + "}"));
    }

    ConversationInfo materialize_conversation(
        const std::string& conversation_id,
        const std::string& options_json = "{}")
    {
        return conversation_info_from_result(
            invoke(
                "conversation.materialize",
                std::string("{\"conversation_id\":") + detail::quote(conversation_id)
                    + ",\"options\":" + object_or_empty(options_json) + "}"));
    }

    AdmissionResult send_message_admission(
        const std::string& conversation_id,
        const std::string& content,
        RuntimeCommandOptions options = {})
    {
        std::string result = invoke(
            "conversation.send_message",
            std::string("{\"conversation_id\":") + detail::quote(conversation_id)
                + ",\"content\":" + detail::quote(content) + "}",
            std::move(options));
        return admission_from_result(result);
    }

    std::string send_message(const std::string& conversation_id, const std::string& content)
    {
        return send_message_admission(conversation_id, content).json;
    }

    AdmissionResult pause_admission(
        const std::string& conversation_id,
        RuntimeCommandOptions options = {})
    {
        std::string result = invoke(
            "conversation.pause",
            std::string("{\"conversation_id\":") + detail::quote(conversation_id) + "}",
            std::move(options));
        return admission_from_result(result);
    }

    std::string pause(const std::string& conversation_id)
    {
        return pause_admission(conversation_id).json;
    }

    void close_conversation(const std::string& conversation_id)
    {
        invoke("conversation.close", std::string("{\"conversation_id\":") + detail::quote(conversation_id) + "}");
    }

    std::string export_snapshot()
    {
        return invoke("runtime.export_snapshot");
    }

    std::string export_conversation_snapshot(
        const std::string& conversation_id,
        const std::string& options_json = "{}")
    {
        return invoke(
            "conversation.export_snapshot",
            std::string("{\"conversation_id\":") + detail::quote(conversation_id)
                + ",\"options\":" + object_or_empty(options_json) + "}");
    }

    std::string agent_tasks(const std::string& conversation_id)
    {
        return invoke("conversation.agent_tasks", std::string("{\"conversation_id\":") + detail::quote(conversation_id) + "}");
    }

    void import_conversation_snapshot(
        const std::string& snapshot_json,
        const std::string& options_json = "{}")
    {
        invoke(
            "conversation.import_snapshot",
            std::string("{\"snapshot\":") + snapshot_json + ",\"options\":" + object_or_empty(options_json) + "}");
    }

    void set_dynamic_snapshot(
        const std::string& conversation_id,
        const std::string& agent_id,
        const std::string& field_name,
        const std::string& text)
    {
        invoke(
            "conversation.set_dynamic_snapshot",
            std::string("{\"conversation_id\":") + detail::quote(conversation_id)
                + ",\"agent_id\":" + detail::quote(agent_id)
                + ",\"field_name\":" + detail::quote(field_name)
                + ",\"text\":" + detail::quote(text) + "}");
    }

    bool resolve_tool_permission(
        const std::string& conversation_id,
        const std::string& tool_call_id,
        const std::string& decision)
    {
        std::string result = invoke(
            "conversation.resolve_tool_permission",
            std::string("{\"conversation_id\":") + detail::quote(conversation_id)
                + ",\"tool_call_id\":" + detail::quote(tool_call_id)
                + ",\"decision\":" + detail::quote(decision) + "}");
        return detail::object_field(result, "resolved") == "true";
    }

    AdmissionResult set_summary_model_admission(
        const std::string& conversation_id,
        const std::string& model_name,
        RuntimeCommandOptions options = {})
    {
        std::string result = invoke(
            "conversation.set_summary_model",
            std::string("{\"conversation_id\":") + detail::quote(conversation_id)
                + ",\"model_name\":" + detail::quote(model_name) + "}",
            std::move(options));
        return admission_from_result(result);
    }

    std::string compact_history(
        const std::string& conversation_id,
        const std::string& agent_ids_json = "[]",
        RuntimeCommandOptions options = {})
    {
        return invoke(
            "conversation.compact_history",
            std::string("{\"conversation_id\":") + detail::quote(conversation_id)
                + ",\"agent_ids\":" + agent_ids_json + "}",
            std::move(options));
    }

    std::string open_workflow_studio(const std::string& options_json = "{}")
    {
        return invoke("studio.open_workflow", std::string("{\"options\":") + object_or_empty(options_json) + "}");
    }

    std::string open_agent_test_studio(const std::string& options_json = "{}")
    {
        return invoke("studio.open_agent_test", std::string("{\"options\":") + object_or_empty(options_json) + "}");
    }

    std::string invoke(
        const std::string& command_type,
        const std::string& payload_json = "{}",
        RuntimeCommandOptions options = {})
    {
        ensure_open();
        std::string request = command_envelope(command_type, payload_json, options);
        char* output = nullptr;
        int code = invoke_(handle_, request.c_str(), &output);
        std::string response = take_runtime_string(output);
        if (code != kOk) {
            std::string error = detail::object_field(response, "error");
            raise_for_code(code, "runtime invoke " + command_type, error.empty() ? response : error);
        }
        return result_from_envelope(response);
    }

    std::optional<RuntimeEvent> next_event(uint32_t timeout_ms = 0)
    {
        ensure_open();
        char* output = nullptr;
        int code = next_event_(handle_, timeout_ms, &output);
        if (code == kErrTimeout) {
            return std::nullopt;
        }
        if (code != kOk) {
            raise_for_code(code, "runtime next_event");
        }
        return runtime_event_from_json(take_runtime_string(output));
    }

    std::optional<RuntimeEvent> next_public_event(uint32_t timeout_ms = 0)
    {
        uint32_t current_timeout = timeout_ms;
        while (auto event = next_event(current_timeout)) {
            if (is_public_runtime_event(*event)) {
                return event;
            }
            current_timeout = 0;
        }
        return std::nullopt;
    }

    void shutdown(uint32_t timeout_ms = 10000)
    {
        if (handle_ == 0 || shutdown_complete_) {
            return;
        }
        int code = shutdown_(handle_, timeout_ms);
        if (code == kOk) {
            shutdown_complete_ = true;
            return;
        }
        raise_for_code(code, "runtime shutdown");
    }

    void destroy()
    {
        if (handle_ == 0) {
            return;
        }
        if (!shutdown_complete_) {
            shutdown();
        }
        int code = destroy_(handle_);
        raise_for_code(code, "runtime destroy");
        handle_ = 0;
        shutdown_complete_ = true;
    }

    void close(uint32_t timeout_ms = 10000)
    {
        shutdown(timeout_ms);
        destroy();
    }

private:
    using AbiVersionFn = uint32_t (*)();
    using StaticStringFn = const char* (*)();
    using CreateFn = int (*)(const char*, AgentRuntimeHandle*);
    using StartFn = int (*)(AgentRuntimeHandle);
    using InvokeFn = int (*)(AgentRuntimeHandle, const char*, char**);
    using NextEventFn = int (*)(AgentRuntimeHandle, uint32_t, char**);
    using ShutdownFn = int (*)(AgentRuntimeHandle, uint32_t);
    using DestroyFn = int (*)(AgentRuntimeHandle);
    using FreeStringFn = void (*)(char*);

#ifdef _WIN32
    HMODULE library_ = nullptr;
#else
    void* library_ = nullptr;
#endif
    AgentRuntimeHandle handle_ = 0;
    bool shutdown_complete_ = false;
    std::string version_;
    std::string capabilities_json_;

    AbiVersionFn abi_version_ = nullptr;
    StaticStringFn version_fn_ = nullptr;
    StaticStringFn capabilities_ = nullptr;
    CreateFn create_ = nullptr;
    StartFn start_ = nullptr;
    InvokeFn invoke_ = nullptr;
    NextEventFn next_event_ = nullptr;
    ShutdownFn shutdown_ = nullptr;
    DestroyFn destroy_ = nullptr;
    StaticStringFn last_error_ = nullptr;
    FreeStringFn free_string_ = nullptr;

    void move_from(Runtime& other) noexcept
    {
        library_ = other.library_;
        handle_ = other.handle_;
        shutdown_complete_ = other.shutdown_complete_;
        version_ = std::move(other.version_);
        capabilities_json_ = std::move(other.capabilities_json_);
        abi_version_ = other.abi_version_;
        version_fn_ = other.version_fn_;
        capabilities_ = other.capabilities_;
        create_ = other.create_;
        start_ = other.start_;
        invoke_ = other.invoke_;
        next_event_ = other.next_event_;
        shutdown_ = other.shutdown_;
        destroy_ = other.destroy_;
        last_error_ = other.last_error_;
        free_string_ = other.free_string_;

        other.library_ = nullptr;
        other.handle_ = 0;
        other.shutdown_complete_ = false;
    }

    void load_library(const std::string& path)
    {
#ifdef _WIN32
        library_ = LoadLibraryA(path.c_str());
#else
        library_ = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
#endif
        if (!library_) {
            throw RuntimeError(0, "failed to load agent runtime library: " + path);
        }

        abi_version_ = symbol<AbiVersionFn>("agent_runtime_abi_version_v1");
        version_fn_ = symbol<StaticStringFn>("agent_runtime_version_v1");
        capabilities_ = symbol<StaticStringFn>("agent_runtime_capabilities_v1");
        create_ = symbol<CreateFn>("agent_runtime_create_v1");
        start_ = symbol<StartFn>("agent_runtime_start_v1");
        invoke_ = symbol<InvokeFn>("agent_runtime_invoke_v1");
        next_event_ = symbol<NextEventFn>("agent_runtime_next_event_v1");
        shutdown_ = symbol<ShutdownFn>("agent_runtime_shutdown_v1");
        destroy_ = symbol<DestroyFn>("agent_runtime_destroy_v1");
        last_error_ = symbol<StaticStringFn>("agent_runtime_last_error_json_v1");
        free_string_ = symbol<FreeStringFn>("agent_runtime_free_string_v1");

        uint32_t actual = abi_version_();
        if (actual != kAgentRuntimeAbiVersion) {
            throw RuntimeError(
                0,
                "incompatible agent runtime ABI: expected 1, got " + std::to_string(actual));
        }
        version_ = detail::borrowed_string(version_fn_());
        capabilities_json_ = detail::borrowed_string(capabilities_());
    }

    template <typename T>
    T symbol(const char* name)
    {
#ifdef _WIN32
        void* value = reinterpret_cast<void*>(GetProcAddress(library_, name));
#else
        void* value = dlsym(library_, name);
#endif
        if (!value) {
            throw RuntimeError(0, std::string("agent runtime missing symbol: ") + name);
        }
        return reinterpret_cast<T>(value);
    }

    void create(const std::string& create_options_json)
    {
        AgentRuntimeHandle handle = 0;
        int code = create_(create_options_json.c_str(), &handle);
        if (code != kOk) {
            raise_for_code(code, "runtime create");
        }
        handle_ = handle;
        shutdown_complete_ = false;
    }

    bool close_noexcept() noexcept
    {
        if (handle_ == 0) {
            return true;
        }
        if (!shutdown_complete_) {
            int shutdown_code = shutdown_(handle_, 10000);
            if (shutdown_code != kOk) {
                return false;
            }
            shutdown_complete_ = true;
        }
        int destroy_code = destroy_(handle_);
        if (destroy_code != kOk) {
            return false;
        }
        handle_ = 0;
        return true;
    }

    void unload_library() noexcept
    {
        if (!library_) {
            return;
        }
#ifdef _WIN32
        FreeLibrary(library_);
#else
        dlclose(library_);
#endif
        library_ = nullptr;
    }

    void ensure_open() const
    {
        if (handle_ == 0) {
            throw RuntimeStateError(kErrBadState, "agent runtime is not open");
        }
    }

    void raise_for_code(int code, const std::string& context, const std::string& response = {}) const
    {
        if (code == kOk) {
            return;
        }
        std::string detail_text = response.empty() ? detail::borrowed_string(last_error_()) : response;
        std::string message = context + " failed (" + std::to_string(code) + "): " + detail_text;
        if (code == kErrTimeout) {
            throw RuntimeTimeout(code, message, detail_text);
        }
        if (code == kErrBadState || code == kErrInvalidHandle) {
            throw RuntimeStateError(code, message, detail_text);
        }
        if (code == kErrUnsupported) {
            throw UnsupportedCommand(code, message, detail_text);
        }
        throw RuntimeError(code, message, detail_text);
    }

    std::string take_runtime_string(char* pointer) const
    {
        if (!pointer) {
            return "{}";
        }
        std::string value(pointer);
        free_string_(pointer);
        return value;
    }

    static std::string input_payload(const std::string& path)
    {
        return std::string("{\"input\":") + detail::quote(path) + "}";
    }

    static std::string registration_payload(const std::string& registration_json)
    {
        return std::string("{\"registration\":") + registration_json + "}";
    }

    static std::string object_or_empty(const std::string& json)
    {
        return json.empty() ? "{}" : json;
    }

    static std::string spawn_payload(const ConversationSpawnOptions& options)
    {
        std::string payload = std::string("{\"schema\":") + detail::quote(options.schema)
            + ",\"cluster_id\":" + detail::quote(options.cluster_id);
        if (!options.tool_host_context_json.empty()) {
            payload += ",\"tool_host_context\":" + options.tool_host_context_json;
        }
        if (!options.permissions_json.empty()) {
            payload += ",\"permissions\":" + options.permissions_json;
        }
        payload += "}";
        return payload;
    }

    static std::string command_envelope(
        const std::string& command_type,
        const std::string& payload_json,
        const RuntimeCommandOptions& options)
    {
        std::string request = "{\"schema\":\"agent-runtime-command/v1\"";
        if (options.request_id.has_value()) {
            request += ",\"id\":" + detail::quote(*options.request_id);
        }
        if (options.command_id.has_value()) {
            request += ",\"command_id\":" + detail::quote(*options.command_id);
        }
        request += ",\"type\":" + detail::quote(command_type)
            + ",\"payload\":" + (payload_json.empty() ? "{}" : payload_json) + "}";
        return request;
    }

    static std::string result_from_envelope(const std::string& envelope)
    {
        std::string result = detail::object_field(envelope, "result");
        return result.empty() ? envelope : result;
    }

    static ConversationInfo conversation_info_from_result(const std::string& result)
    {
        ConversationInfo info;
        info.json = result;
        info.conversation_id = detail::find_string_field(result, "conversation_id");
        info.scope_id = detail::find_string_field(result, "scope_id");
        info.tenant_id = detail::find_string_field(result, "tenant_id");
        info.user_id = detail::find_string_field(result, "user_id");
        info.created_at = detail::find_string_field(result, "created_at");
        if (info.conversation_id.empty()) {
            throw RuntimeError(kOk, "conversation result did not include conversation_id", result);
        }
        return info;
    }

    static AdmissionResult admission_from_result(const std::string& result)
    {
        AdmissionResult admission;
        admission.json = result;
        admission.command_id = detail::find_string_field(result, "command_id");
        admission.decision = detail::find_string_field(result, "decision");
        admission.reason = detail::find_string_field(result, "reason");
        return admission;
    }

    static RuntimeEvent runtime_event_from_json(std::string json)
    {
        RuntimeEvent event;
        event.json = std::move(json);
        event.type = detail::find_string_field(event.json, "type");
        event.event_line = detail::find_string_field(event.json, "event_line");
        event.conversation_id = detail::find_string_field(event.json, "conversation_id");
        event.payload_json = detail::object_field(event.json, "payload");
        return event;
    }
};

inline bool is_ledger_delta_event(const RuntimeEvent& event)
{
    return event.type == kLedgerDeltaEventType
        && detail::find_string_field(event.payload_json, "schema") == kLedgerDeltaSchema;
}

inline std::optional<std::string> ledger_delta_from_event(const RuntimeEvent& event)
{
    if (!is_ledger_delta_event(event)) {
        return std::nullopt;
    }
    return event.payload_json;
}

inline bool is_state_delta_event(const RuntimeEvent& event)
{
    return event.type == kStateDeltaEventType
        && detail::find_string_field(event.payload_json, "schema") == kStateDeltaSchema;
}

inline std::optional<std::string> state_delta_from_event(const RuntimeEvent& event)
{
    if (!is_state_delta_event(event)) {
        return std::nullopt;
    }
    return event.payload_json;
}

} // namespace runtime
} // namespace corework

#endif // COREWORK_RUNTIME_RUNTIME_HPP
