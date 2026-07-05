use corework::ai_system::{AIOutput, SimpleArgs};

#[test]
fn test_simple_args_dash_format() {
    let args = SimpleArgs::parse("--product_id ABC123 --include_reserved true").unwrap();

    assert_eq!(args.get("product_id"), Some("ABC123"));
    assert_eq!(args.get("include_reserved"), Some("true"));
    assert!(args.get_bool("include_reserved"));
}

#[test]
fn test_simple_args_equals_format_with_dash_prefix() {
    let args = SimpleArgs::parse("--product_id=ABC123 --include_reserved=true").unwrap();

    assert_eq!(args.get("product_id"), Some("ABC123"));
    assert_eq!(args.get("include_reserved"), Some("true"));
    assert!(args.get_bool("include_reserved"));
}

#[test]
fn test_simple_args_flag_without_value() {
    let args = SimpleArgs::parse("--product_id ABC123 --include_reserved").unwrap();

    assert_eq!(args.get("product_id"), Some("ABC123"));
    assert_eq!(args.get("include_reserved"), Some("true"));
    assert!(args.get_bool("include_reserved"));
}

#[test]
fn test_simple_args_missing_required() {
    let args = SimpleArgs::parse("--include_reserved true").unwrap();

    let result = args.get_required("product_id");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("缺失必填参数"));
}

#[test]
fn test_simple_args_type_conversion() {
    let args = SimpleArgs::parse("--count 42 --price 19.99 --active true").unwrap();

    assert_eq!(args.get_i64("count"), Some(42));
    assert_eq!(args.get_f64("price"), Some(19.99));
    assert!(args.get_bool("active"));
}

#[test]
fn test_ai_output_success() {
    let output = AIOutput::success(
        serde_json::json!({"msg": "数据查询成功"}),
        "✅ 数据查询成功",
    );
    assert!(output.is_ok());
    assert_eq!(output.error_code, 0);
    assert!(output.to_ai.contains("数据查询成功"));
    assert!(!output.result.is_null());
}

#[test]
fn test_ai_output_error() {
    let output = AIOutput::error(1, "❌ 参数无效");
    assert!(!output.is_ok());
    assert_eq!(output.error_code, 1);
    assert!(output.to_ai.contains("参数无效"));
    assert!(output.result.is_null());
}

#[tokio::test]
async fn test_system_registry_ai_functions() {
    use corework::system::SystemRegistry;

    let ai_systems = SystemRegistry::list_ai_systems();
    println!("发现 {} 个AI系统", ai_systems.len());

    for system in ai_systems {
        println!("  - {}: {}", system.name, system.description);
    }

    // 生成帮助文档
    let help_doc = SystemRegistry::generate_ai_help_doc();
    assert!(help_doc.contains("Available AI Systems"));
}

#[test]
fn test_parse_complex_args() {
    let args = SimpleArgs::parse("--product_id ABC123 --count 5 --price 29.99 --active").unwrap();

    assert_eq!(args.get("product_id"), Some("ABC123"));
    assert_eq!(args.get_i64("count"), Some(5));
    assert_eq!(args.get_f64("price"), Some(29.99));
    assert!(args.get_bool("active"));
}

#[test]
fn test_bool_parsing_variations() {
    let args1 = SimpleArgs::parse("--flag true").unwrap();
    assert!(args1.get_bool("flag"));

    let args2 = SimpleArgs::parse("--flag 1").unwrap();
    assert!(args2.get_bool("flag"));

    let args3 = SimpleArgs::parse("--flag yes").unwrap();
    assert!(args3.get_bool("flag"));

    let args4 = SimpleArgs::parse("--flag false").unwrap();
    assert!(!args4.get_bool("flag"));

    let args5 = SimpleArgs::parse("--flag").unwrap();
    assert!(args5.get_bool("flag"));
}

#[test]
fn test_simple_args_quoted_and_list_values() {
    let args = SimpleArgs::parse(r#"--name "Alice Smith" --tags alpha,beta,gamma"#).unwrap();

    assert_eq!(args.get("name"), Some("Alice Smith"));
    assert_eq!(args.get_list("tags"), vec!["alpha", "beta", "gamma"]);
}

#[test]
fn test_simple_args_quoted_value_can_contain_inner_exec_flags() {
    let args = SimpleArgs::parse(
        r#"--script "input text:String\n1: EXEC CallLlm --user_message input.text\nreturn reply=1.response_text" --note "llm draft""#,
    )
    .unwrap();

    let names = args.names().collect::<Vec<_>>();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"script"));
    assert!(names.contains(&"note"));
    assert_eq!(args.get("note"), Some("llm draft"));
    let script = args.get("script").unwrap();
    assert!(script.contains("EXEC CallLlm --user_message input.text"));
    assert!(!names.contains(&"user_message"));
}
