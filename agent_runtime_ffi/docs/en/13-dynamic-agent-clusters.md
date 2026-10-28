# Dynamic agent clusters

For direct Role/Feature candidate creation and updates, use [Skill transactions](14-skill-candidate-transactions.md). Transaction-managed clusters restore automatically and must be updated through that API. reload_skills remains a compatibility path for unmanaged files only.

Conversations are created from clusters. `agents` defines initial members; `focus_agent_id` selects the primary member. The primary member must reference a real, non-empty `kind: role` Skill. Feature Skills provide additional capabilities. Existing profile inheritance remains supported.

After Runtime start, invoke `runtime.add_agent_cluster` with an object-valued `payload.registration` using the existing `agent-runtime-agent-cluster-registration/v1` format. Role and Feature references must already exist in the Skill catalog loaded at startup; these commands do not register newly written Skill files. This adds configuration without creating a conversation. Use `conversation.spawn` with the cluster ID to instantiate all initial members. Adding a cluster does not change existing conversations.

To replace configuration, invoke `runtime.update_agent_cluster` with the complete `registration` and `expected_version`. Read the current version from `runtime.get_agent_cluster_definitions` or the successful change result. Results use `agent-runtime-agent-cluster-change/v1`, with `action` and the effective `cluster` including `version`.

Any live conversation of the cluster blocks updates, including waiting, paused, approval-blocked and background work. Close all conversations before updating. Errors use `cluster_in_use` with `details.cluster_id` and `details.conversation_ids`, or `cluster_version_conflict` with expected/actual versions. Failed requests preserve configuration, files and running caches. Admission and commit share the FFI handle write lock with spawn/close.

Versions are SHA-256 digests of effective cluster configuration, not counters. Newly exported conversation snapshots carry `cluster_definition` (cluster ID and version). Restore/import checks it before allocating a conversation or replacing its ledger. Dynamically added/updated clusters reject snapshots without a version. The pre-start registration path retains legacy snapshot compatibility; reload dynamic configurations through the post-start add command after a restart.

Hosts own configuration file persistence. Runtime does not rewrite cluster or resource files. Equal effective configuration produces equal versions after restart. Skill content is outside this configuration digest. Retrieval glue must already be installed at startup, while model configuration may remain deferred under existing rules.

`runtime.update_agent_cluster` optionally accepts `reload_skills: ["role-name", "feature-name"]`. The host saves registered Skill files first, preferably by atomic file replacement. Names must be unique and referenced by the submitted cluster. Reload preserves name/kind, requires non-empty instructions, and rejects system/embedded Skills. Runtime stages all files and atomically accepts cluster configuration, metadata and body caches. Failure keeps the previously accepted configuration/cache; host files are neither written nor rolled back.

Reload checks actual references in every live Agent in the process, including other clusters, children, background tasks and dynamically imported Features. Idle, paused and approval-blocked instances still count. Configuration-only references do not count. `skill_in_use` errors include `details.dependencies` with `conversation_id`, `agent_id` and `skill_names`. Reference admission and updates share a read/write gate, released before driving an Agent or calling an LLM. Whole-catalog replacement by another handle is also rejected while Agents depend on it. External editors do not participate in this in-process lock.

Success adds `reloaded_skills`, a map of Skill content versions. Snapshots also record actual `skill_versions`; restore/import rejects changed bodies with `skill_version_conflict`. Dynamic clusters reject older snapshots missing these versions. Body-only reload does not change `cluster.version`. SDKs expose optional `reload_skills` (Python/C++), or `update_agent_cluster_with_skills` / `UpdateAgentClusterWithSkills` (Rust/Go), preserving existing calls.

Pre-start `runtime.register_agent_cluster` remains frozen after start. Built-in clusters cannot be replaced. Resources, tool endpoints, execution engines and AI tools are unchanged. The Rust/Python SDK methods are `add_agent_cluster` and `update_agent_cluster`; Go uses `AddAgentCluster`/`UpdateAgentCluster`, and C++ uses `add_agent_cluster_json`/`update_agent_cluster_json`. Both commands use the existing `agent_runtime_invoke_v1` symbol.

See the [complete Chinese contract and example](../zh/13-dynamic-agent-clusters.md).
