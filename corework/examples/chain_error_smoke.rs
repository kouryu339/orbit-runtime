//! Phase 2 冒烟：编译器错误消息升级验证
//!
//! `cargo test -p corework` 被 for_loop_node.rs 预先错误阻塞，
//! 所以用 example 做 Phase 2 的端到端验证。
//!
//! 运行：`cargo run -p corework --example chain_error_smoke`

use corework::workflow::chain_compiler::{
    compile_chain, find_closest, levenshtein, ChainError, ChainErrorKind,
};

fn main() {
    let mut pass = 0usize;
    let mut fail = 0usize;

    macro_rules! check {
        ($name:expr, $cond:expr) => {{
            if $cond {
                pass += 1;
                println!("✅ {}", $name);
            } else {
                fail += 1;
                println!("❌ {}", $name);
            }
        }};
    }

    // ─── 单元：自动分类 ────────────────────────────────────────────

    let e = ChainError::new(3, "IF 语句缺少结尾冒号");
    check!(
        "autoclass: Syntax (缺少冒号)",
        e.kind == ChainErrorKind::Syntax
    );

    let e = ChainError::new(5, "未定义的步骤: 2");
    check!(
        "autoclass: UnknownReference (未定义步骤)",
        e.kind == ChainErrorKind::UnknownReference
    );

    let e = ChainError::new(5, "步骤 1 没有引脚 `foo`");
    check!(
        "autoclass: UnknownReference (没有引脚)",
        e.kind == ChainErrorKind::UnknownReference
    );

    let e = ChainError::new(1, "未知算子 foo");
    check!(
        "autoclass: UnknownOperation (未知算子)",
        e.kind == ChainErrorKind::UnknownOperation
    );

    // ─── 单元：链式构造 ────────────────────────────────────────────

    let e = ChainError::of_kind(1, ChainErrorKind::TypeMismatch, "whatever");
    check!("of_kind 覆盖推断", e.kind == ChainErrorKind::TypeMismatch);

    let e = ChainError::new(2, "bad").with_col(12);
    check!("with_col 设置列号", e.col == 12);
    check!("Display 含列号", format!("{}", e).contains("line 2:12"));

    let e = ChainError::new(2, "bad");
    let s = format!("{}", e);
    check!(
        "Display 无列号时省略",
        s.contains("line 2 ") && !s.contains(":0")
    );

    let e = ChainError::new(1, "bad").with_suggestion("try X");
    check!(
        "Display 渲染 suggestion",
        format!("{}", e).contains("💡 try X")
    );

    // ─── 单元：Levenshtein ──────────────────────────────────────────

    check!("levenshtein identity", levenshtein("abc", "abc") == 0);
    check!("levenshtein 1-subst", levenshtein("abc", "abd") == 1);
    check!("levenshtein 1-del", levenshtein("click", "clik") == 1);
    check!("levenshtein empty", levenshtein("", "abc") == 3);

    let cands = vec!["ClickElement", "FillInput", "OpenBrowser"];
    let hit = find_closest("ClickElment", cands);
    check!(
        "find_closest 命中近似词",
        hit.as_deref() == Some("ClickElement")
    );

    let hit = find_closest("xyz", vec!["ClickElement"]);
    check!("find_closest 拒绝过远", hit.is_none());

    let e = ChainError::new(1, "bad").with_suggest_from("Clik", vec!["Click", "Close", "Copy"]);
    check!(
        "with_suggest_from 追加提示",
        e.suggestion
            .as_deref()
            .map(|s| s.contains("Click"))
            .unwrap_or(false)
    );

    // ─── 集成：端到端编译 ───────────────────────────────────────────

    // 未知节点名 → UnknownOperation
    let res = compile_chain(
        r#"
INPUT url:String
$x = 0
1: CALL NoSuchNodeForSure123()
RETURN result=$x
"#,
    );
    match res {
        Err(e) if e.kind == ChainErrorKind::UnknownOperation => {
            pass += 1;
            println!("✅ compile unknown_op: kind = {}", e.kind.as_str());
            println!("   message: {}", e.message);
            if let Some(s) = &e.suggestion {
                println!("   suggestion: {}", s);
            }
        }
        other => {
            fail += 1;
            println!(
                "❌ compile unknown_op: expected UnknownOperation, got {:?}",
                other
            );
        }
    }

    // 不存在的步骤号 → UnknownReference
    let res = compile_chain(
        r#"
INPUT a:String
RETURN result=9.Body
"#,
    );
    match res {
        Err(e) if e.kind == ChainErrorKind::UnknownReference => {
            pass += 1;
            println!("✅ compile unknown_ref: kind = {}", e.kind.as_str());
            println!("   message: {}", e.message);
        }
        other => {
            fail += 1;
            println!(
                "❌ compile unknown_ref: expected UnknownReference, got {:?}",
                other
            );
        }
    }

    // JSON 序列化（Tauri 需要）
    let e = ChainError::of_kind(3, ChainErrorKind::UnknownOperation, "未知节点 Clik")
        .with_col(5)
        .with_suggestion("did you mean `Click`?");
    let json = serde_json::to_value(&e).unwrap();
    check!("JSON line", json["line"] == 3);
    check!("JSON col", json["col"] == 5);
    check!("JSON kind", json["kind"] == "unknown_operation");
    check!(
        "JSON suggestion",
        json["suggestion"] == "did you mean `Click`?"
    );

    println!("\n──────────────");
    println!("✅ passed: {}, ❌ failed: {}", pass, fail);
    if fail > 0 {
        std::process::exit(1);
    }
}
