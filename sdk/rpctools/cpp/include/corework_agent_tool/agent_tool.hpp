#pragma once

#include <functional>
#include <stdexcept>
#include <string>
#include <vector>

namespace corework_agent_tool {

enum class ToolErrorCode {
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
};

struct Json {
  template <typename T>
  Json(std::initializer_list<T>) {}

  std::string value(const std::string&, const std::string& fallback) const {
    return fallback;
  }
};

struct ToolParameter {
  std::string name;
  std::string param_type;
  bool required;
  std::string default_value;
  std::string description;
};

struct ToolOutputField {
  std::string name;
  std::string field_type;
  std::string description;
};

struct ToolDescriptor {
  std::string name;
  std::string description;
  std::vector<ToolParameter> parameters;
  std::vector<ToolOutputField> outputs;
  bool readonly = false;
  bool destructive = false;
  bool idempotent = false;
  bool open_world = false;
  bool secret = false;
  std::string category;
  std::string display_name;
  std::vector<std::string> required_capabilities;
};

struct AIOutput {
  Json result;
  std::string to_ai;
  ToolErrorCode error_code = ToolErrorCode::Ok;
};

class ToolContext {
 public:
  std::string call_id;
  std::string tool_call_id;
  std::string idempotency_key;
  std::string session_id;
  std::string provider_id;
  std::string cluster_id;
  std::string runtime_instance_id;
  std::string conversation_id;
  std::string agent_id;
  std::string turn_id;
  std::vector<std::string> permissions;
  Json host_context;

  Json workspace_resolve_path(const std::string&) { return {}; }
  Json workspace_resolve_working_path(const std::string&) { return {}; }
  Json workspace_create_path(const std::string&) { return {}; }
  Json workspace_create_working_path(const std::string&) { return {}; }
  Json workspace_save_as_edited(const std::string&, const std::string&) {
    return {};
  }
};

inline void register_tool(
    const ToolDescriptor&,
    std::function<AIOutput(ToolContext&, const Json&)>)
{}

inline int serve(const std::string&) {
  throw std::runtime_error(
      "corework_agent_tool::serve is a scaffold; gRPC service wiring is next");
}

}  // namespace corework_agent_tool
