以下是你可加载的技能。技能是运行时的行为准则和操作指导，**不是工具命令**。
- 技能名（如 fileops、audioconv）不能直接放入 tools 数组调用。
- 需要某个技能时，使用工具 UpdateSkills --skills 技能名1,技能名2 来激活它（多个用逗号分隔，如 UpdateSkills --skills fileops,audioconv）。
- 激活后你会获得该技能的详细操作指导，再根据指导调用具体的工具命令。
- tools 数组中只能写「可用工具」章节里列出的工具名（如 ListDir、AudioConvert 等）。
- 注意：UpdateSkills 是全量替换语义，每次调用需传入所有期望激活的技能（包括之前已激活的）。
