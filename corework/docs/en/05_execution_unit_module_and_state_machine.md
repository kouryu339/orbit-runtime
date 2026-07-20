# 5 Execution Unit, Module, and State Machine

This document summarizes the execution building blocks used by `corework`.

## 5.1 Execution Unit

An execution unit is a small runnable component with explicit input, output, and
error behavior. Systems, workflow nodes, and Agent runtime states all follow this
style.

## 5.2 Module

A module groups related systems, nodes, configuration, and registration logic.
Modules make capabilities discoverable without requiring the root application to
manually wire every operation.

## 5.3 State Machine

The state machine layer models long-running runtime behavior as transitions
between named states. It is used by the Agent Runtime for phases such as
thinking, executing tools, saying a response, asking for input, and suspension.

## 5.4 Engineering Guidance

- Keep state transitions explicit.
- Keep side effects in execution units, not in registration code.
- Use events to report runtime progress.
- Persist snapshots when host recovery is required.
