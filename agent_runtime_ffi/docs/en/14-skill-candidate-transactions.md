# Skill candidate transactions

Host commands `runtime.read_skill`, `runtime.validate_skill_transaction` and `runtime.commit_skill_transaction` use the existing invoke ABI. They are not new AI tools. The host binds candidates to the current employee project and keeps unaccepted edits separate.

Read with `{"name":"employee_role"}` to obtain accepted `source`, `metadata`, content `version`, `store_revision`, and `exists`. Missing names return `exists:false` and `version:null`. Submit the same payload to validate or commit:

```json
{
  "expected_store_revision": 0,
  "skills": [{
    "name": "employee_role", "kind": "role", "expected_version": null,
    "source": "---\nname: employee_role\ndescription: Answer assigned requests.\nkind: role\n---\n\nRequest missing information before acting.\n"
  }],
  "cluster": {
    "expected_version": null,
    "registration": {
      "schema":"agent-runtime-agent-cluster-registration/v1", "id":"employee-team", "description":"Employee execution cluster",
      "focus_agent_id":"primary", "agents":[{"id":"primary","role":"employee_role","features":[]}]
    }
  }
}
```

Null/omitted expected content versions mean create; replacements require the current digest. A provided cluster follows the same rule using cluster.version. Store revision serializes the complete managed catalog. Empty skills are allowed only with a cluster update. Candidates supply full SKILL.md text, never filesystem paths. Names match `[A-Za-z0-9_-]{1,128}`, must be unique, and must match metadata. Only non-system role/capability Skills with nonempty description/body are accepted; name/kind cannot change during replacement. Installed tools and executable workflow references are checked. Maximum 64 candidates, 1 MiB each, 4 MiB total UTF-8 bytes.

Any live dependent Agent blocks content replacement, including other clusters, dynamic imports, children, background work, idle/paused/approval-blocked instances. Cluster replacement requires no live conversations of that cluster. Reference admission and publication use the shared Skill gate. Validation performs no writes or execution. Commit returns accepted sources, metadata, content versions, store revision and optional effective cluster. Errors preserve the accepted state and never close conversations.

Errors include skill_in_use (details.dependencies with conversation_id/agent_id/skill_names), skill_version_conflict (name/expected/actual), skill_store_conflict (expected/actual), cluster_in_use and cluster_version_conflict. Snapshot content mismatches use the structured Skill conflict. Stale store instances must reopen resources rather than silently loading another version.

Configure resources.skills.managed_dir to a writable host-owned directory; default is skills.root_dir/.orbit-managed. Immutable objects/<source-sha256>/SKILL.md files are written before atomically replacing catalog.json, the sole durable commit point. The manifest contains sources, versions and complete transaction-managed clusters. Restart ignores unreferenced objects and restores either the old transaction or the complete new transaction. Corrupt accepted objects fail loading. Retained historical objects are not automatically deleted. A file lock serializes writers across processes, but live Agent occupancy is process-local. These guarantees cover process interruption; power-loss guarantees depend on the filesystem/platform.

Updates to an existing filesystem Skill create a managed source overlay; the original file and reference/scripts asset base remain untouched. New Skills use their managed object directory as asset base; this API does not upload extra assets. Original reload_skills remains for unmanaged files; managed Skills and clusters must use candidate transactions. The host persists employee UI metadata independently. Same-process Runtime handles must use a consistent resource root.

SDKs expose read_skill/validate_skill_transaction/commit_skill_transaction (Rust/Python), ReadSkill/ValidateSkillTransaction/CommitSkillTransaction (Go), and read_skill/validate_skill_transaction_json/commit_skill_transaction_json (C++). Public payload schema: schemas/skill-transaction.schema.json. Detect capabilities.features.skill_candidate_transactions == 1; local packages can share the same base runtime version.

See the [Chinese contract](../zh/14-skill-candidate-transactions.md) for persistence and error details.
