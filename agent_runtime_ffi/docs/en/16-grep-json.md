# GrepJson: structured reads using the existing Grep authorization

Grep and GrepJson are separate AI tools. They share resources.json grep configuration, root capabilities, GrepService and its concurrency semaphore. There is no separate grep_json registration. Built-in thinking/thinking-pro expose neither; hosts explicitly select one or both through their own Skill tools. Neither registers a Workflow node.

## Configuration

Use existing grep.roots and limits. max_file_bytes bounds the source file (default 2 MiB; hosts can increase it for larger documents). max_entries counts actual JSON values once, including object and array containers. max_output_bytes includes the entire result envelope. max_results caps page size; max_pattern_bytes bounds request strings. timeout_ms and max_concurrent_searches apply. max_line_bytes does not truncate JSON; regular expressions are not used.

Hidden/ignored paths, links and Windows reparse points remain excluded. Root-relative capability opens enforce file authority. GrepJson cannot increase host limits.

## Operations

- outline: immediate child descriptors with exact JSON Pointers, types and counts.
- search: AND together an exact field name, typed value_json equality, and/or literal contains on string values. Returns match Pointer, parent_pointer and at most three ancestor descriptors. Ancestor id/name/type fields are explicitly marked complete values or previews.
- read: return the exact subtree if it fits; otherwise return child descriptors/pages with value_complete=false.
- page: paginate array or object children, or return a chunk of a long string. String offsets count Unicode scalar values, not bytes or UTF-16 code units.

pointer defaults to the root (empty string); escape ~ as ~0 and / as ~1. path names one relative JSON file. Omit root_id to list authorized root IDs.

offset defaults to zero; limit defaults to 50 and is capped by host max_results. Nonzero offsets require expected_revision. Continue with identical filters/pointer, returned next_offset and revision. Objects enumerate in stable parsed-key order rather than source formatting order. If a small complete item cannot fit the remaining page space, resume it on the next page instead of degrading it to a descriptor and requiring an extra read.

complete describes enumeration/search completion. value_complete describes whether a returned value is exact. A complete enumeration of descriptors is not a complete subtree value. Never use preview as a replacement value. Results are always valid JSON. An item that cannot fit even as a descriptor causes an explicit budget error rather than an endless zero-progress continuation.

Strict JSON only, with at most 128 nested containers. Arbitrary-precision numbers retain their value. Every legal object key stays a field, including names matching serialization-library private markers. Container validation and tree construction share one traversal; the library parses strings and numbers. Search value_json uses the same lossless parser. Duplicate keys, malformed JSON, invalid/missing pointers and exceeded file/node budgets fail explicitly. Incomplete reads must not be interpreted as proof of absence.

## Read then modify

Every file result includes revision = sha256: followed by lowercase SHA-256 of the exact source bytes. Whitespace changes also change revision. Pass expected_revision even for offset-zero follow-up reads. A mismatch returns error code 409 with revision_conflict. No long-lived server snapshot is retained; calls read and parse again.

Example search:
```json
{"root_id":"project","path":"scene.json","action":"search","field":"id","value_json":"\"camera-01\""}
```

Read the returned parent_pointer with action=read and expected_revision. Expand descriptors or continue pages until all values needed for an edit have value_complete=true.

GrepJson is read-only. The host existing editing tool should accept the file reference, expected_revision, JSON Pointer and replacement/patch data, optionally an expected old value. Under one write lock shared by all writers, read exact bytes and compare revision, apply edits, validate the business Schema, atomically persist, and return the new revision. On conflict do not write; require the agent to read again. Checking a hash without protecting the check-to-write interval is not sufficient.

Rust hosts may reuse ai_assistant::systems::grep_json::document_revision. Other hosts must hash the original bytes before parsing/formatting. Array indices are valid only against that revision.

File length and modification time are checked around reads to catch common concurrent modifications. This is not a snapshot guarantee against uncoordinated in-place writers. The revision identifies bytes actually read, and later validation remains mandatory.

Each call uses bounded input, parser nodes and output, with no persistent document cache. Cancellation/timeouts signal the worker; blocked OS IO may finish later and retains its shared permit until it exits. Results use the normal Runtime tool trace and go directly to to_ai without RPC summary truncation.

Example host-activated capability Skill:

```markdown
---
name: structured-file-reading
description: Read authorized text and structured JSON files.
tools: ["Grep", "GrepJson"]
---
Use Grep for text and GrepJson for JSON. Follow next_offset with expected_revision.
Read complete values before passing edits and the revision to the host editing tool.
```


## Acceptance evidence

19 focused cases and two execution-context acceptance cases cover special legal keys, exact numbers, syntax/depth limits, pagination and revision conflicts. A 2,360,802-byte scene is searched by camera ID, then all 256 keyframes across four tracks are recovered and compared field by field. Pagination-related calls decrease from 12 to 8. Reordering bindings rejects the previous revision with 409; a new search finds the moved camera.

These are Runtime read-path checks. Host write-lock, business Schema validation and concurrent-edit integration still require host acceptance.
