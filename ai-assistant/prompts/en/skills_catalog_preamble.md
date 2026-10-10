The following Skills can be loaded. Skills are runtime behavior guidelines and operating instructions; they are **not tool commands**.
- Skill names such as fileops or audioconv must not be called directly as tools.
- The host controls Skill activation. Only when UpdateSkills is explicitly available may you activate Skills with UpdateSkills --skills skill1,skill2; otherwise use host-activated Skills. A catalog entry does not grant activation permission.
- After activation, you will receive the detailed Skill instructions; then call the concrete tool commands according to those instructions.
- The tools array may only contain tool names listed in the "Available Tools" section, such as ListDir or AudioConvert.
- Note: UpdateSkills uses full replacement semantics. Each call must include all Skills that should remain active, including previously activated Skills.
