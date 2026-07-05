The following Skills can be loaded. Skills are runtime behavior guidelines and operating instructions; they are **not tool commands**.
- Skill names such as fileops or audioconv must not be called directly as tools.
- When a Skill is needed, activate it with UpdateSkills --skills skill1,skill2 (comma-separated, for example UpdateSkills --skills fileops,audioconv).
- After activation, you will receive the detailed Skill instructions; then call the concrete tool commands according to those instructions.
- The tools array may only contain tool names listed in the "Available Tools" section, such as ListDir or AudioConvert.
- Note: UpdateSkills uses full replacement semantics. Each call must include all Skills that should remain active, including previously activated Skills.
