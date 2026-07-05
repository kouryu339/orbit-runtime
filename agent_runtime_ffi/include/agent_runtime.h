#ifndef AGENT_RUNTIME_H
#define AGENT_RUNTIME_H

#include <stdint.h>

#ifdef _WIN32
  #ifdef AGENT_RUNTIME_BUILD
    #define AGENT_RUNTIME_EXPORT __declspec(dllexport)
  #else
    #define AGENT_RUNTIME_EXPORT __declspec(dllimport)
  #endif
#else
  #define AGENT_RUNTIME_EXPORT __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Agent Runtime ABI 1
 *
 * Compatibility:
 * - agent_runtime_abi_version_v1() must return 1.
 * - Product versions do not determine ABI compatibility.
 * - Feature compatibility is discovered through
 *   agent_runtime_capabilities_v1().
 *
 * Threading:
 * - Functions may be called from different host threads.
 * - Calls operating on one handle are internally coordinated.
 * - The host must not issue new start/invoke calls after shutdown begins.
 *   Such calls fail with AGENT_RUNTIME_ERR_BAD_STATE.
 * - The host may continue next_event calls after shutdown begins to drain
 *   shutdown-produced events such as ledger deltas and conversation closure.
 * - No ABI 1 function invokes host callbacks.
 *
 * Strings:
 * - All input strings are non-NULL, NUL-terminated UTF-8.
 * - Strings returned through char** are runtime-owned allocations transferred
 *   to the host. Release each non-NULL value exactly once with
 *   agent_runtime_free_string_v1().
 * - Strings returned directly as const char* are static and must not be freed.
 */

typedef uint64_t AgentRuntimeHandle;

#define AGENT_RUNTIME_INVALID_HANDLE       UINT64_C(0)

#define AGENT_RUNTIME_OK                    0
#define AGENT_RUNTIME_ERR_INVALID_ARGUMENT  1
#define AGENT_RUNTIME_ERR_INVALID_HANDLE    2
#define AGENT_RUNTIME_ERR_BAD_STATE         3
#define AGENT_RUNTIME_ERR_TIMEOUT           4
#define AGENT_RUNTIME_ERR_UNSUPPORTED       5
#define AGENT_RUNTIME_ERR_RUNTIME         100
#define AGENT_RUNTIME_ERR_PANIC           101

/*
 * Returns the C ABI major version implemented by this library.
 * This function is non-blocking, thread-safe, and always returns 1 for ABI 1.
 */
AGENT_RUNTIME_EXPORT uint32_t agent_runtime_abi_version_v1(void);

/*
 * Returns the product version as a static UTF-8 string.
 * The pointer is never NULL and remains valid until the library is unloaded.
 */
AGENT_RUNTIME_EXPORT const char* agent_runtime_version_v1(void);

/*
 * Returns agent-runtime-capabilities/v1 JSON as a static UTF-8 string.
 * The document lists supported command types and transport semantics.
 * The pointer is never NULL and remains valid until the library is unloaded.
 */
AGENT_RUNTIME_EXPORT const char* agent_runtime_capabilities_v1(void);

/*
 * Creates an unstarted runtime.
 *
 * create_options_json:
 *   A non-NULL UTF-8 string. Pass "" for defaults, or inline
 *   agent-runtime-create-options/v1 JSON. Config file paths are not supported.
 * out_handle:
 *   A non-NULL output pointer. Set to AGENT_RUNTIME_INVALID_HANDLE on failure.
 *
 * On success, the host owns the handle and must eventually call shutdown_v1
 * followed by destroy_v1.
 */
AGENT_RUNTIME_EXPORT int agent_runtime_create_v1(
    const char* create_options_json,
    AgentRuntimeHandle* out_handle
);

/*
 * Freezes registration and starts the runtime.
 *
 * The handle must be open. The call may block while resources are initialized.
 * Repeated successful calls are idempotent.
 */
AGENT_RUNTIME_EXPORT int agent_runtime_start_v1(AgentRuntimeHandle handle);

/*
 * Executes one agent-runtime-command/v1 JSON request.
 *
 * out_response_json:
 *   Must be non-NULL. It is set to NULL before processing. When command
 *   dispatch reaches the runtime, it receives an agent-runtime-result/v1
 *   document on both command success and command failure.
 *
 * Calls on the same handle are currently serialized. The function may block
 * for the full duration of the requested operation.
 */
AGENT_RUNTIME_EXPORT int agent_runtime_invoke_v1(
    AgentRuntimeHandle handle,
    const char* request_json,
    char** out_response_json
);

/*
 * Retrieves one runtime event from the handle's event queue.
 *
 * timeout_ms:
 *   0 performs a non-blocking poll. A positive value waits up to that many
 *   milliseconds.
 * out_event_json:
 *   Must be non-NULL and is set to NULL if no event is returned.
 *
 * Returns AGENT_RUNTIME_ERR_TIMEOUT when no event arrives before the deadline.
 * During and after shutdown, hosts may keep polling until the queue closes and
 * AGENT_RUNTIME_ERR_BAD_STATE is returned. There are no callback threads and
 * no user_data lifetime obligations.
 */
AGENT_RUNTIME_EXPORT int agent_runtime_next_event_v1(
    AgentRuntimeHandle handle,
    uint32_t timeout_ms,
    char** out_event_json
);

/*
 * Starts or continues orderly shutdown.
 *
 * Once called, the handle enters CLOSING and rejects new runtime calls. The
 * function waits for in-flight FFI calls, closes conversations, stops runtime
 * services, and releases event producers. AGENT_RUNTIME_ERR_TIMEOUT is
 * retryable by calling shutdown_v1 again with the same handle.
 *
 * AGENT_RUNTIME_OK guarantees shutdown completed. Only then may destroy_v1 be
 * called.
 */
AGENT_RUNTIME_EXPORT int agent_runtime_shutdown_v1(
    AgentRuntimeHandle handle,
    uint32_t timeout_ms
);

/*
 * Removes a fully shut down handle from the process handle registry.
 *
 * Returns AGENT_RUNTIME_ERR_BAD_STATE unless shutdown_v1 has completed.
 * After success, all use of the numeric handle returns INVALID_HANDLE.
 */
AGENT_RUNTIME_EXPORT int agent_runtime_destroy_v1(AgentRuntimeHandle handle);

/*
 * Returns thread-local agent-runtime-error/v1 JSON for the most recent failed
 * ABI call on the current host thread. The pointer is borrowed, may be NULL,
 * and remains valid only until the next ABI call on the same thread.
 */
AGENT_RUNTIME_EXPORT const char* agent_runtime_last_error_json_v1(void);

/*
 * Releases a string returned through an ABI 1 char** output parameter.
 * NULL is accepted. Passing any other pointer is undefined behavior.
 */
AGENT_RUNTIME_EXPORT void agent_runtime_free_string_v1(char* value);

#ifdef __cplusplus
}
#endif

#endif
