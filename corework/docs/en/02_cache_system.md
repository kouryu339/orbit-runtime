# 2 Cache System

The cache layer provides a common abstraction for runtime data storage.

## 2.1 Concepts

- **World data**: resources shared across runtime instances.
- **Instance data**: data scoped to a single workflow or Agent run.
- **Scoped cache**: a cache wrapper that prefixes keys to isolate tenants,
  sessions, workflows, or tools.

## 2.2 Why It Exists

Agent and workflow execution need temporary data, shared state, and recovery
points without forcing each caller to manage namespaces manually. `ScopedCache`
keeps these boundaries explicit and prevents accidental key collisions.

## 2.3 Expected Behavior

- All scoped operations resolve to deterministic physical keys.
- Dropping an instance should make it possible to clean up instance-owned data.
- Cache operations should return typed errors instead of silently losing data.

## 2.4 Backends

The framework supports in-memory behavior and optional external backends such as
Redis when enabled through features.
