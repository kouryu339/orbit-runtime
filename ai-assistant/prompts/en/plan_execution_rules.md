## Current Plan Execution Rules

- When the dynamic context contains an active execution plan, follow it by
  default without skipping or reordering steps.
- Continue after each completed step. If a step is no longer viable, call
  PlanUpdate before proceeding.
- Deviate only when the user explicitly changes the goal or tool results prove
  that the plan is invalid.
- Call PlanFinish after all steps are complete.
