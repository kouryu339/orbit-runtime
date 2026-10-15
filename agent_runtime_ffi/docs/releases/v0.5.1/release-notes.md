# Orbit Runtime v0.5.1

Fix the hidden post-response auto-continue limit while preserving the configured
pre-request thinking round budget. Native ABI remains 1.3.

## Fixed

- Remove the implicit 20-tool-batch limit from native function calling and legacy
  EXEC response handling. A valid model response is no longer discarded because
  an independent post-response step counter is exhausted.
- Remove the obsolete step counters, reset operations, stop branch, and localized
  stop prompts. Old snapshot values for those counters are ignored.
- Keep `max_thinking_rounds` unchanged: the default is `0` (no round limit), and
  positive values stop at thinking entry before another model request.
- Correct the round-budget documentation to match this behavior.

## Upgrade

Replace the native runtime and use the matching v0.5.1 SDK download metadata.
No ABI or host command schema migration is required. Hosts that need a finite
thinking budget should explicitly configure a positive `max_thinking_rounds`.
There is no replacement implicit 20-step cap when this value remains `0`.

## Validation

Regression coverage exercises 25 consecutive tool batches in both protocols,
including restored obsolete counter values, and checks that an exhausted positive
round budget returns before model configuration or a request is required.

See [the release verification report](release-report.md) for completed checks and
platform limitations.
