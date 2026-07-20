//! 运行时解析器验证——跑一批 LLM 文本，打印 is_tool_call + parse_tool_calls 结果
//!
//! cargo run -p ai-assistant --example runtime_parser_demo

use ai_assistant::runtime::parser::{is_tool_call, parse_tool_calls};

fn main() {
    let cases: Vec<(&str, &str)> = vec![
        // ---- 合法：单行 EXEC ----
        ("单行 EXEC + 参数",
         "EXEC BrowserOpen --url https://example.com --title \"Hello World\""),

        // ---- 合法：多个并发 EXEC ----
        ("多 EXEC 并发",
         "EXEC GetSnapshot\nEXEC GetScriptDocs --page_id main"),

        // ---- 合法：变量 + 引用 ----
        ("变量声明 + 引用",
         "$url = \"https://example.com\"\nEXEC BrowserOpen --url $url"),

        // ---- 合法：heredoc 变量 ----
        ("heredoc 多行变量",
         "$script = \"\nwhen flag clicked\nmove 10 steps\nsay hello\n\"\nEXEC ScratchInject --script $script --target sprite1"),

        // ---- 错误：旧块式 EXEC 已禁止 ----
        ("旧块式 EXEC 被拒绝",
         "EXEC FooTool\n--script\nline a\nline b\n--target sprite1"),

        // ---- 合法：带 think 块 ----
        ("带 <think> 块",
         "<think>\n让我分析一下需求\n</think>\nEXEC GetSnapshot"),

        // ---- 合法：无参数 EXEC ----
        ("无参数 EXEC",
         "EXEC GetSnapshot"),

        // ---- 合法：混合 think + 变量 + 多 EXEC ----
        ("复杂组合",
         "<think>\n需要先截图再注入\n</think>\n$code = \"\nwhen flag clicked\nforever\nmove 10 steps\nend\n\"\nEXEC GetSnapshot\nEXEC ScratchInject --script $code --target sprite1"),

        // ---- 非工具调用：纯对话 ----
        ("纯对话",
         "我觉得你应该用 ffmpeg 来转换视频格式。"),

        // ---- 非工具调用：ASK ----
        ("ASK 决策",
         "ASK\n你想用什么颜色？红色还是蓝色？"),

        // ---- 非工具调用：RESULT ----
        ("RESULT 决策",
         "RESULT\n任务完成！共注入 4 个积木到 sprite1。"),

        // ---- 错误：空输入 ----
        ("空输入",
         ""),

        // ---- 错误：EXEC 无工具名 ----
        ("EXEC 无工具名",
         "EXEC "),

        // ---- 错误：未定义变量 ----
        ("引用未定义变量",
         "EXEC Foo --bar $undefined_var"),

        // ---- 错误：heredoc 未闭合 ----
        ("heredoc 未闭合",
         "$code = \"\nunclosed content\nEXEC Foo --x 1"),

        // ---- 错误：运行时用了 $() ----
        ("运行时 $() inline pure",
         "EXEC Foo --value $(Add --A 1 --B 2)"),

        // ---- 不规范：大小写混用 ----
        ("大小写混用 exec",
         "exec BrowserOpen --url https://example.com"),

        // ---- 不规范：带 markdown 装饰 ----
        ("markdown 装饰",
         "**EXEC BrowserOpen** --url https://example.com"),

        // ---- 不规范：EXEC 前有废话 ----
        ("EXEC 前有前导文本",
         "好的，我来帮你处理\nEXEC GetSnapshot"),

        // ---- 不规范：EXEC 行尾冒号 ----
        ("EXEC 行尾冒号",
         "EXEC: BrowserOpen --url https://example.com"),
    ];

    println!("\n{}", "=".repeat(80));
    println!("  运行时解析器验证 — is_tool_call + parse_tool_calls");
    println!("{}\n", "=".repeat(80));

    let mut pass = 0;
    let mut fail = 0;

    for (i, (label, text)) in cases.iter().enumerate() {
        println!("━━━ Case {} ━━━  {}", i + 1, label);
        println!("输入文本:");
        for line in text.lines() {
            println!("  │ {}", line);
        }
        if text.is_empty() {
            println!("  │ (空)");
        }

        let is_tc = is_tool_call(text);
        println!("is_tool_call: {}", if is_tc { "✓ true" } else { "✗ false" });

        match parse_tool_calls(text) {
            Ok(calls) => {
                println!("parse_tool_calls: ✓ Ok({} 个调用)", calls.len());
                for (j, call) in calls.iter().enumerate() {
                    println!("  [{}] {} ", j, call.name);
                    for (k, v) in &call.params {
                        let display_v = if v.len() > 60 {
                            format!("{}…({} chars)", &v[..57], v.len())
                        } else {
                            v.clone()
                        };
                        println!("      --{} = {:?}", k, display_v);
                    }
                    println!("  legacy_cmd: {:?}", call.to_legacy_command());
                }
                pass += 1;
            }
            Err(e) => {
                println!("parse_tool_calls: ✗ Err({:?})", e);
                if is_tc {
                    // is_tool_call=true 但 parse 失败？不应该发生
                    println!("  ⚠️  is_tool_call=true 但 parse_tool_calls 失败！");
                    fail += 1;
                } else {
                    pass += 1;
                }
            }
        }
        println!();
    }

    println!("{}", "=".repeat(80));
    println!(
        "  结果：{} 项一致 / {} 项异常（共 {} 项）",
        pass,
        fail,
        cases.len()
    );
    println!("{}", "=".repeat(80));
}
