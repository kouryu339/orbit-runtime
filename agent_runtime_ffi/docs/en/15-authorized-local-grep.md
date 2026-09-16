# Authorized local Grep (AI only)

Register `grep` in the host's resources JSON before starting the Runtime:

```json
{
  "schema": "agent-runtime-resource-registration/v1",
  "id": "local-resources",
  "skills": { "root_dir": "./skills", "builtin_system": true },
  "grep": {
    "roots": [{ "id": "project", "path": "E:/open-source/ai-framework" }],
    "timeout_ms": 10000,
    "max_results": 200,
    "max_output_bytes": 65536,
    "max_file_bytes": 2097152,
    "max_pattern_bytes": 16384,
    "max_regex_bytes": 8388608,
    "max_line_bytes": 4096,
    "max_entries": 100000,
    "max_concurrent_searches": 2
  }
}
```

All shown limits are configurable defaults. Limits must be positive, with at
least 1024 output bytes. Root IDs are unique 1–64 character ASCII identifiers
(letters, digits, `_`, `-`). Missing configuration grants no filesystem access.
Root directories must exist at registration. Relative root paths require a
resources-file base directory; JSON-only registration should use absolute paths.
Resource registration is frozen after Runtime start. Changing authority requires
restarting the Runtime with new resources and recreating conversations. Capabilities are supplied by the
host, not restored from model-writable cache or conversation snapshots.

The built-in thinking and thinking-pro skills expose `Grep`. Custom skills can
include `Grep` in their tools list. Normal tool permission policy still applies.
It is system-only: no Workflow node or script function is registered.

## AI calls

Call `Grep` with no `root_id` to list authorized IDs (not absolute paths). Search:

```json
{
  "root_id": "project",
  "path": "ai-assistant/src",
  "pattern": "PlanWrite",
  "literal": true,
  "ignore_case": false,
  "glob": "**/*.rs",
  "output_mode": "content",
  "limit": 50
}
```

`path` defaults to the root; use forward slashes. `literal` defaults to true;
false enables Rust regex (not PCRE, no backreferences or look-around). Matching
is line-based, not multiline. Glob patterns are relative to the authorized root.
`content` returns path, one-based line number, text and `line_truncated`.
`files_with_matches` returns one path per file; `count` returns matching **lines**
per file, not occurrences. Directory iteration order is unspecified; no offset
pagination or immutable filesystem snapshot is promised.

## Boundaries and completeness

- Absolute/parent/drive/UNC/alternate-stream paths are rejected. Traversal skips
  symlinks and Windows reparse points. File reads use directory capabilities to
  prevent outside-root access through link replacement races. This is not an OS
  sandbox: hosts should not authorize attacker-managed directories containing
  sensitive hard links, special files, or files being maliciously moved around.
- Hidden paths are excluded. Nested `.gitignore` and `.ignore` rules apply even
  without a Git repository. Global Git excludes and `.git/info/exclude` are not
  loaded. Only UTF-8 text without NUL bytes is searched; binary/non-UTF-8 files
  are counted as skipped. No AI parameter can disable these boundaries.
- `complete` refers to scanning eligible text files. IO failures and oversized
  files make it false; counters expose these omissions. Hidden/ignored/link and
  binary files are intentionally out of scope. `truncated` and `reason` report
  result, output, entry or time limits. A line can be clipped independently,
  indicated by `line_truncated`. Zero matches is distinct from invalid input.
- Output is bounded JSON directly in `to_ai`, not an RPC 600-character summary.
  The byte budget includes the response envelope; matching entries are never
  cut mid-JSON. `limit` is clamped to the host maximum. Narrow the path/glob when
  incomplete; never interpret partial results as proof that text is absent.
- Searches share the host service's concurrency limit across conversations.
  Saturation returns 429. Timeout returns 408 or a partial result with a reason.
  Cancellation signals the blocking worker; outstanding OS IO may finish later
  and keeps its concurrency permit until it exits. This is not a hard IO kill.

The implementation uses Rust `regex`, `globset`, `ignore` and `cap-std`, not a
shell or externally installed `rg`. Search results are untrusted file data, not
instructions. Standard Runtime tool tracing records calls and results; consider
host log access/retention when searching sensitive authorized directories.
