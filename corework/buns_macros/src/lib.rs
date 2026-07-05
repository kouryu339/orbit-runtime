//! Buns Framework 装饰器宏

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::Parse, parse::ParseStream, parse_macro_input, ItemEnum, ItemStruct, LitStr, Token,
};

/// #[buns_system("名字")] 装饰器 - 注册L1系统层组件
///
/// 支持2种语法形式：
/// 1. 基础形式：`#[buns_system("QueryStockSystem")]`
/// 2. 带描述：`#[buns_system("QueryStockSystem", description = "查询商品库存")]`
///    ```
#[proc_macro_attribute]
pub fn buns_system(attr: TokenStream, item: TokenStream) -> TokenStream {
    let system_attrs = parse_macro_input!(attr as SystemAttributes);
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;
    let name_str = &system_attrs.name;
    let display_name = system_attrs.display_name.as_deref().unwrap_or(name_str);
    let description = system_attrs.description.as_deref();

    // description 是 Option<&str>，需要手动生成 Some("...") / None token
    let description_tokens = match &description {
        Some(desc) => quote! { ::std::option::Option::Some(#desc) },
        None => quote! { ::std::option::Option::None },
    };

    // 生成 SystemFactory 提交
    let system_factory_submission = quote! {
        ::inventory::submit! {
            ::corework::SystemFactory {
                name: #name_str,
                description: #description_tokens,
                constructor: || ::std::sync::Arc::new(#struct_name::default()),
                dynamic_constructor: Some(|| ::std::sync::Arc::new(#struct_name::default())),
            }
        }
    };

    // 如果有params定义，则生成 AISystemFactory 提交
    let ai_factory_submission = if let Some(ref params) = system_attrs.params {
        let param_defs: Vec<_> = params
            .iter()
            .map(|param| {
                let name = &param.name;
                let param_type = &param.param_type;
                let required = param.required;
                let default_value_tokens = match &param.default_value {
                    Some(val) => quote! { ::std::option::Option::Some(#val) },
                    None => quote! { ::std::option::Option::None },
                };
                let description = &param.description;

                quote! {
                    ::corework::ai_system::AIParameter {
                        name: #name,
                        param_type: #param_type,
                        required: #required,
                        default_value: #default_value_tokens,
                        description: #description,
                    }
                }
            })
            .collect();

        let params_const_name = syn::Ident::new(
            &format!("__AI_PARAMS_{}__", struct_name.to_string().to_uppercase()),
            struct_name.span(),
        );

        // 处理 outputs
        let output_defs: Vec<_> = system_attrs
            .outputs
            .iter()
            .flatten()
            .map(|output| {
                let name = &output.name;
                let field_type = &output.field_type;
                let description = &output.description;

                quote! {
                    ::corework::ai_system::AIOutputField {
                        name: #name,
                        field_type: #field_type,
                        description: #description,
                    }
                }
            })
            .collect();

        let outputs_const_name = syn::Ident::new(
            &format!("__AI_OUTPUTS_{}__", struct_name.to_string().to_uppercase()),
            struct_name.span(),
        );

        let ai_description = description.unwrap_or(name_str);

        // 元数据字段（使用MCP的默认值）
        let destructive_val = system_attrs.destructive.unwrap_or(true); // 默认假设危险
        let readonly_val = system_attrs.readonly.unwrap_or(false); // 默认假设有写操作
        let idempotent_val = system_attrs.idempotent.unwrap_or(false); // 默认非幂等
        let open_world_val = system_attrs.open_world.unwrap_or(true); // 默认开放世界
        let secret_val = system_attrs.secret.unwrap_or(false); // 默认无敏感数据

        // 如果有 outputs，生成常量
        let outputs_const = if !output_defs.is_empty() {
            quote! {
                #[allow(non_upper_case_globals)]
                const #outputs_const_name: &[::corework::ai_system::AIOutputField] = &[
                    #(#output_defs),*
                ];
            }
        } else {
            quote! {
                #[allow(non_upper_case_globals)]
                const #outputs_const_name: &[::corework::ai_system::AIOutputField] = &[];
            }
        };

        quote! {
            #outputs_const

            #[allow(non_upper_case_globals)]
            const #params_const_name: &[::corework::ai_system::AIParameter] = &[
                #(#param_defs),*
            ];

            ::inventory::submit! {
                ::corework::ai_system::AISystemFactory {
                    metadata: ::corework::ai_system::AISystemMetadata {
                        name: #name_str,
                        display_name: #display_name,
                        description: #ai_description,
                        tool_kind: "local",
                        parameters: #params_const_name,
                        outputs: #outputs_const_name,
                        destructive: #destructive_val,
                        readonly: #readonly_val,
                        idempotent: #idempotent_val,
                        open_world: #open_world_val,
                        secret: #secret_val,
                    },
                    constructor: || ::std::sync::Arc::new(#struct_name::default()),
                }
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #input

        impl ::std::default::Default for #struct_name {
            fn default() -> Self {
                Self
            }
        }

        #[::async_trait::async_trait]
        impl ::corework::prelude::DynamicExecute for #struct_name
        where
            Self: ::corework::system::SystemOperation,
            <Self as ::corework::system::SystemOperation>::Input: ::corework::cache::CacheValue,
            <Self as ::corework::system::SystemOperation>::Output: ::corework::cache::CacheValue,
        {
            async fn execute_dynamic(
                &self,
                input: ::std::collections::HashMap<::std::string::String, ::serde_json::Value>,
                ctx: &::corework::orchestration::Context,
            ) -> ::corework::error::Result<::serde_json::Value> {
                let input_value = ::serde_json::Value::Object(
                    input.into_iter().collect::<::serde_json::Map<_, _>>()
                );
                let typed_input: <Self as ::corework::system::SystemOperation>::Input =
                    ::serde_json::from_value(input_value)
                        .map_err(|e| ::corework::error::FrameworkError::InvalidData(
                            format!("Failed to deserialize input: {}", e)
                        ))?;

                let output = <Self as ::corework::system::SystemOperation>::execute(
                    self,
                    typed_input,
                    ctx
                ).await.map_err(|e| ::corework::error::FrameworkError::SystemError(
                    format!("{:?}", e)
                ))?;

                let output_value = ::serde_json::to_value(&output)
                    .map_err(|e| ::corework::error::FrameworkError::SerializationError(e))?;

                Ok(output_value)
            }
        }

        #system_factory_submission

        #ai_factory_submission
    };

    TokenStream::from(expanded)
}

/// #[buns_model("名字", "描述", "分类")] 装饰器 - 注册模型类型
/// #[buns_model("名字", "版本", "描述", "分类", exportable = false)] - 指定不导出
///
/// 自动将结构体注册为可导出的类型，生成 TypeStructure 定义
///
/// # 参数
/// - name: 类型名称
/// - version: 版本号（可选，默认 "1.0.0"）
/// - description: 类型描述
/// - category: 类型分类
/// - exportable: 是否导出到蓝图编辑器（可选，默认 true）
///
/// # 示例
/// ```
/// // 导出到蓝图编辑器
/// #[buns_model("Point2D", "2D point with x and y coordinates", "Geometry")]
/// pub struct Point2D {
///     /// X coordinate
///     pub x: f64,
///     /// Y coordinate
///     pub y: f64,
/// }
///
/// // 仅用于代码内部和缓存，不导出
/// #[buns_model("PageInfo", "1.0.0", "Browser page info", "Browser", exportable = false)]
/// pub struct PageInfo {
///     pub page_id: String,
/// }
/// ```
#[proc_macro_attribute]
pub fn buns_model(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_struct = parse_macro_input!(item as ItemStruct);
    let struct_name = &input_struct.ident;

    // 解析参数：name, version, description, category 和 exportable
    let attr_args = parse_macro_input!(attr as ModelAttributes);
    let name_str = attr_args.name;
    let version_str = attr_args.version;
    let description_str = attr_args.description;
    let category_str = attr_args.category;
    let exportable = attr_args.exportable;

    // 解析结构体字段
    let fields = match &input_struct.fields {
        syn::Fields::Named(fields) => &fields.named,
        _ => panic!("buns_model only supports structs with named fields"),
    };

    // 生成 FieldDefinition 列表
    let mut field_defs = Vec::new();
    for field in fields {
        let field_name = field.ident.as_ref().unwrap().to_string();
        let field_type = &field.ty;

        // 提取类型名称
        let type_name = quote!(#field_type).to_string().replace(" ", ""); // 移除空格，如 "Vec < Point2D >" -> "Vec<Point2D>"

        // 检查是否有 #[optional] 属性
        let is_optional = field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("optional"));

        // 检查是否有 #[doc] 文档注释
        let description = field
            .attrs
            .iter()
            .filter_map(|attr| {
                if attr.path().is_ident("doc") {
                    if let syn::Meta::NameValue(meta) = &attr.meta {
                        if let syn::Expr::Lit(expr_lit) = &meta.value {
                            if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                return Some(lit_str.value().trim().to_string());
                            }
                        }
                    }
                }
                None
            })
            .collect::<Vec<_>>()
            .join(" ");

        field_defs.push(quote! {
            ::corework::data_type::type_structure::FieldDefinition {
                name: #field_name,
                type_name: #type_name,
                description: #description,
                optional: #is_optional,
            }
        });
    }

    // 生成唯一的常量名（与 register_node 一致的模式）
    let fields_const_name =
        quote::format_ident!("__TYPE_{}_FIELDS__", struct_name.to_string().to_uppercase());

    let expanded = quote! {
        #input_struct

        // 生成编译时常量数组
        const #fields_const_name: &[::corework::data_type::type_structure::FieldDefinition] = &[
            #(#field_defs),*
        ];

        // 直接使用 inventory::submit!（无需 ctor）
        ::inventory::submit! {
            ::corework::data_type::type_structure::TypeStructure {
                name: #name_str,
                version: #version_str,
                description: #description_str,
                category: #category_str,
                fields: #fields_const_name,
                is_primitive: false,
                exportable: #exportable,
                is_enum: false,
                enum_variants: &[],
            }
        }
    };

    TokenStream::from(expanded)
}

/// #[buns_enum("名字", "描述", "分类")] 装饰器 - 注册枚举类型
///
/// 自动将枚举注册为可导出的类型，生成 TypeStructure 定义
///
/// # 参数
/// - name: 类型名称
/// - version: 版本号（可选，默认 "1.0.0"）
/// - description: 类型描述
/// - category: 类型分类
/// - exportable: 是否导出到蓝图编辑器（可选，默认 true）
///
/// # 示例
/// ```
/// #[buns_enum("QuestionType", "Question type enumeration", "Grading")]
/// pub enum QuestionType {
///     /// Single blank question
///     SingleBlank,
///     /// Multiple blank question
///     MultipleBlank,
///     /// Short answer question
///     ShortAnswer,
/// }
/// ```
#[proc_macro_attribute]
pub fn buns_enum(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_enum = parse_macro_input!(item as ItemEnum);
    let enum_name = &input_enum.ident;

    // 解析参数
    let attr_args = parse_macro_input!(attr as ModelAttributes);
    let name_str = attr_args.name;
    let version_str = attr_args.version;
    let description_str = attr_args.description;
    let category_str = attr_args.category;
    let exportable = attr_args.exportable;

    // 提取枚举变体
    let mut variant_defs = Vec::new();
    for variant in &input_enum.variants {
        let variant_name = variant.ident.to_string();

        // 提取文档注释作为描述
        let description = variant
            .attrs
            .iter()
            .filter_map(|attr| {
                if attr.path().is_ident("doc") {
                    if let syn::Meta::NameValue(meta) = &attr.meta {
                        if let syn::Expr::Lit(expr_lit) = &meta.value {
                            if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                                return Some(lit_str.value().trim().to_string());
                            }
                        }
                    }
                }
                None
            })
            .collect::<Vec<_>>()
            .join(" ");

        let desc = if description.is_empty() {
            variant_name.clone()
        } else {
            description
        };

        variant_defs.push(quote! {
            ::corework::data_type::type_structure::EnumVariant {
                name: #variant_name,
                description: #desc,
            }
        });
    }

    // 生成变体数组常量名
    let variants_const_name = syn::Ident::new(
        &format!("__ENUM_VARIANTS_{}__", enum_name.to_string().to_uppercase()),
        enum_name.span(),
    );

    let expanded = quote! {
        #input_enum

        const #variants_const_name: &[::corework::data_type::type_structure::EnumVariant] = &[
            #(#variant_defs),*
        ];

        ::inventory::submit! {
            ::corework::data_type::type_structure::TypeStructure {
                name: #name_str,
                version: #version_str,
                description: #description_str,
                category: #category_str,
                fields: &[],
                is_primitive: false,
                exportable: #exportable,
                is_enum: true,
                enum_variants: #variants_const_name,
            }
        }
    };

    TokenStream::from(expanded)
}

/// #[buns_orchestration("名字")] 装饰器 - 注册L2编排层组件
#[proc_macro_attribute]
pub fn buns_orchestration(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name = parse_macro_input!(attr as LitStr);
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;
    let name_str = name.value();

    let expanded = quote! {
        #input

        impl ::std::default::Default for #struct_name {
            fn default() -> Self {
                Self
            }
        }

        ::inventory::submit! {
            ::corework::SystemFactory {
                name: #name_str,
                constructor: || ::std::sync::Arc::new(#struct_name::default()),
                dynamic_constructor: None,
            }
        }
    };

    TokenStream::from(expanded)
}

/// #[register_node(...)] 装饰器 - 注册蓝图节点
#[proc_macro_attribute]
pub fn register_node(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;
    let node_attr = parse_macro_input!(attr as NodeAttributes);

    let node_type_str = struct_name.to_string();
    let node_type_enum = &node_attr.node_type;
    let version = &node_attr.version;
    let category = &node_attr.category;
    let display_name = &node_attr.display_name;
    let description = &node_attr.description;
    let permissions = node_attr.permissions;

    // 解析引脚定义
    let mut pin_definitions = Vec::new();

    // 处理 exec_in（支持 "PinName@描述" 格式）
    if let Some(ref exec_in_pins) = node_attr.exec_in {
        for pin_def in exec_in_pins {
            let (name, _, description, _) = parse_pin_definition(pin_def);
            let description_lit = syn::LitStr::new(description, proc_macro2::Span::call_site());
            pin_definitions.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #name,
                    kind: ::corework::prelude::PinKind::ExecInput,
                    data_type: "",
                    description: #description_lit,
                    default_value: None,
                }
            });
        }
    }

    // 处理 exec_out（支持 "PinName@描述" 格式）
    if let Some(ref exec_out_pins) = node_attr.exec_out {
        for pin_def in exec_out_pins {
            let (name, _, description, _) = parse_pin_definition(pin_def);
            let description_lit = syn::LitStr::new(description, proc_macro2::Span::call_site());
            pin_definitions.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #name,
                    kind: ::corework::prelude::PinKind::ExecOutput,
                    data_type: "",
                    description: #description_lit,
                    default_value: None,
                }
            });
        }
    }

    // 处理 data_in（支持 "name:Type@描述#默认值" 格式）
    if let Some(ref data_in_pins) = node_attr.data_in {
        for pin_def in data_in_pins {
            let (name, type_name_str, description, default_str) = parse_pin_definition(pin_def);
            let type_name_lit = syn::LitStr::new(type_name_str, proc_macro2::Span::call_site());
            let description_lit = syn::LitStr::new(description, proc_macro2::Span::call_site());
            let default_value_tokens = if default_str.is_empty() {
                quote! { None }
            } else {
                let default_lit = syn::LitStr::new(default_str, proc_macro2::Span::call_site());
                quote! { Some(#default_lit) }
            };
            pin_definitions.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #name,
                    kind: ::corework::prelude::PinKind::DataInput,
                    data_type: #type_name_lit,
                    description: #description_lit,
                    default_value: #default_value_tokens,
                }
            });
        }
    }

    // 处理 data_out
    if let Some(ref data_out_pins) = node_attr.data_out {
        for pin_def in data_out_pins {
            let (name, type_name_str, description, _) = parse_pin_definition(pin_def);
            let type_name_lit = syn::LitStr::new(type_name_str, proc_macro2::Span::call_site());
            let description_lit = syn::LitStr::new(description, proc_macro2::Span::call_site());
            pin_definitions.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #name,
                    kind: ::corework::prelude::PinKind::DataOutput,
                    data_type: #type_name_lit,
                    description: #description_lit,
                    default_value: None,
                }
            });
        }
    }

    // 生成唯一的常量名
    let pins_const_name = syn::Ident::new(
        &format!("__NODE_{}_PINS__", struct_name),
        struct_name.span(),
    );

    let constraints_const_name = syn::Ident::new(
        &format!("__NODE_{}_CONSTRAINTS__", struct_name),
        struct_name.span(),
    );

    // 生成工厂函数标识符
    let factory_name = syn::Ident::new(
        &format!("__NODE_FACTORY_{}__", struct_name),
        struct_name.span(),
    );

    // 生成wildcard_constraints定义
    let wildcard_constraints_code = if let Some(ref constraints) = node_attr.wildcard_constraints {
        let constraint_items: Vec<_> = constraints
            .iter()
            .map(|(wildcard_id, pins)| {
                let pins_array = pins.iter().map(|p| quote! { #p }).collect::<Vec<_>>();
                quote! {
                    (#wildcard_id, &[#(#pins_array),*])
                }
            })
            .collect();

        quote! {
            #[allow(non_upper_case_globals)]
            const #constraints_const_name: &[(&str, &[&str])] = &[
                #(#constraint_items),*
            ];
        }
    } else {
        quote! {
            #[allow(non_upper_case_globals)]
            const #constraints_const_name: &[(&str, &[&str])] = &[];
        }
    };

    // 根据 node_type 生成 execute_node 的完整实现（对齐UE，无需trait分支）
    let execute_node_fn = match node_type_enum.as_str() {
        "Impure" => {
            quote! {
                // Impure节点的__execute_node_impl实现
                // 调用用户定义的execute方法（不通过ImpureNode trait）
                impl #struct_name {
                    #[doc(hidden)]
                    pub fn __execute_node_impl<'a>(
                        &'a self,
                        ctx: &'a mut ::corework::workflow::execution::ExecutionContext,
                        inputs: ::std::collections::HashMap<String, ::corework::workflow::core::DataValue>,
                    ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::corework::error::Result<::corework::workflow::core::NodeOutput>> + Send + 'a>> {
                        Box::pin(async move {
                            self.execute(ctx, inputs).await
                        })
                    }
                }
            }
        }
        "Pure" => {
            quote! {
                // Pure节点的__execute_node_impl实现
                // 调用用户定义的evaluate方法（不通过PureNode trait）
                impl #struct_name {
                    #[doc(hidden)]
                    pub fn __execute_node_impl<'a>(
                        &'a self,
                        _ctx: &'a mut ::corework::workflow::execution::ExecutionContext,
                        inputs: ::std::collections::HashMap<String, ::corework::workflow::core::DataValue>,
                    ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::corework::error::Result<::corework::workflow::core::NodeOutput>> + Send + 'a>> {
                        Box::pin(async move {
                            let outputs = self.evaluate(inputs)?;
                            Ok(::corework::workflow::core::NodeOutput::Data(outputs))
                        })
                    }
                }
            }
        }
        _ => quote! {},
    };

    let expanded = quote! {
        #input

        #execute_node_fn

        #[allow(non_upper_case_globals)]
        const #pins_const_name: &[::corework::prelude::PinMetadata] = &[
            #(#pin_definitions),*
        ];

        #wildcard_constraints_code

        // Node factory: create the default instance; the executor owns the runtime name.
        #[allow(non_snake_case)]
        fn #factory_name(_name: String) -> ::std::sync::Arc<dyn ::corework::workflow::nodes::traits::BlueprintNode + Send + Sync> {
            ::std::sync::Arc::new(#struct_name::default())
        }

        // 使用 inventory 提交节点元数据
        ::inventory::submit! {
            ::corework::prelude::NodeMetadata {
                node_type: #node_type_str,
                version: #version,
                category: #category,
                display_name: #display_name,
                description: #description,
                pins: #pins_const_name,
                permissions: ::corework::prelude::NodePermissions::new(#permissions),
                wildcard_constraints: #constraints_const_name,
            }
        }

        // 使用 inventory 提交节点工厂 - node_type -> 实例映射
        ::inventory::submit! {
            ::corework::prelude::NodeFactory {
                node_type: #node_type_str,
                factory: #factory_name,
            }
        }
    };

    TokenStream::from(expanded)
}

// 辅助函数：解析字符串数组 ["a", "b", "c"]
fn parse_string_array(input: ParseStream) -> syn::Result<Vec<String>> {
    let content;
    syn::bracketed!(content in input);
    let mut result = Vec::new();

    while !content.is_empty() {
        let lit: LitStr = content.parse()?;
        result.push(lit.value());

        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }

    Ok(result)
}

// 辅助函数：解析引脚定义 "name:Type@description" 或 "name:Type"
/// 解析引脚定义字符串。
///
/// 格式：`"name:Type@描述#默认值"` 或其子集：
/// - `"Name@描述"`              — exec 引脚
/// - `"name:Type@描述"`         — 无默认值
/// - `"name:Type@描述#default"` — 有默认值（JSON 字符串）
///
/// 返回 `(name, type_str, description, default_value)` 四元组。
fn parse_pin_definition(pin_def: &str) -> (&str, &str, &str, &str) {
    // 先从末尾找 '#' 分离默认值（避免描述中含 '#' 干扰的情况，用 rfind）
    let (main_part, default_val) = if let Some(hash_pos) = pin_def.rfind('#') {
        (&pin_def[..hash_pos], &pin_def[hash_pos + 1..])
    } else {
        (pin_def, "")
    };

    // 再分割 @description
    if let Some(at_pos) = main_part.find('@') {
        let (name_type, description) = main_part.split_at(at_pos);
        let description = &description[1..]; // 跳过 '@'

        // 再分割 name:Type
        if let Some(colon_pos) = name_type.find(':') {
            let (name, type_name) = name_type.split_at(colon_pos);
            let type_name = &type_name[1..]; // 跳过 ':'
            return (name, type_name, description, default_val);
        } else {
            // 只有 Name@description，无类型（exec 引脚）
            return (name_type, "", description, default_val);
        }
    } else {
        // 没有描述，仅 name:Type
        if let Some(colon_pos) = main_part.find(':') {
            let (name, type_name) = main_part.split_at(colon_pos);
            let type_name = &type_name[1..]; // 跳过 ':'
            return (name, type_name, "", default_val);
        }
    }

    // 格式错误，返回原字符串作为 name
    (main_part, "", "", default_val)
}

// 辅助函数：解析通配符约束 [("T", &["pin1", "pin2"]), ("U", &["pin3"])]
fn parse_wildcard_constraints(input: ParseStream) -> syn::Result<Vec<(String, Vec<String>)>> {
    let content;
    syn::bracketed!(content in input);
    let mut result = Vec::new();

    while !content.is_empty() {
        // 解析 ("T", &["pin1", "pin2"])
        let tuple_content;
        syn::parenthesized!(tuple_content in content);

        // 解析通配符ID "T"
        let wildcard_id: LitStr = tuple_content.parse()?;
        tuple_content.parse::<Token![,]>()?;

        // 解析 &["pin1", "pin2"]
        tuple_content.parse::<Token![&]>()?;
        let pins_content;
        syn::bracketed!(pins_content in tuple_content);

        let mut pins = Vec::new();
        while !pins_content.is_empty() {
            let pin: LitStr = pins_content.parse()?;
            pins.push(pin.value());

            if !pins_content.is_empty() {
                pins_content.parse::<Token![,]>()?;
            }
        }

        result.push((wildcard_id.value(), pins));

        if !content.is_empty() {
            content.parse::<Token![,]>()?;
        }
    }

    Ok(result)
}

// 节点属性解析结构
struct NodeAttributes {
    node_type: String,
    version: String,
    category: String,
    display_name: String,
    description: String,
    permissions: u8,
    exec_in: Option<Vec<String>>,
    exec_out: Option<Vec<String>>,
    data_in: Option<Vec<String>>,
    data_out: Option<Vec<String>>,
    wildcard_constraints: Option<Vec<(String, Vec<String>)>>,
}

impl Parse for NodeAttributes {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut node_type = None;
        let mut version = None;
        let mut category = None;
        let mut display_name = None;
        let mut description = None;
        let mut permissions = None;
        let mut exec_in = None;
        let mut exec_out = None;
        let mut data_in = None;
        let mut data_out = None;
        let mut wildcard_constraints = None;

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "node_type" => {
                    let value: LitStr = input.parse()?;
                    node_type = Some(value.value());
                }
                "version" => {
                    let value: LitStr = input.parse()?;
                    version = Some(value.value());
                }
                "category" => {
                    let value: LitStr = input.parse()?;
                    category = Some(value.value());
                }
                "display_name" => {
                    let value: LitStr = input.parse()?;
                    display_name = Some(value.value());
                }
                "description" => {
                    let value: LitStr = input.parse()?;
                    description = Some(value.value());
                }
                "permissions" => {
                    let value: syn::LitInt = input.parse()?;
                    permissions = Some(value.base10_parse::<u8>()?);
                }
                "exec_in" => {
                    exec_in = Some(parse_string_array(input)?);
                }
                "exec_out" => {
                    exec_out = Some(parse_string_array(input)?);
                }
                "data_in" => {
                    data_in = Some(parse_string_array(input)?);
                }
                "data_out" => {
                    data_out = Some(parse_string_array(input)?);
                }
                "wildcard_constraints" => {
                    wildcard_constraints = Some(parse_wildcard_constraints(input)?);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("Unknown attribute: {}", key),
                    ))
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(NodeAttributes {
            node_type: node_type.ok_or_else(|| input.error("Missing 'node_type' attribute"))?,
            version: version.unwrap_or_else(|| "1.0.0".to_string()), // 默认版本
            category: category.ok_or_else(|| input.error("Missing 'category' attribute"))?,
            display_name: display_name
                .ok_or_else(|| input.error("Missing 'display_name' attribute"))?,
            description: description
                .ok_or_else(|| input.error("Missing 'description' attribute"))?,
            permissions: permissions.unwrap_or(0),
            exec_in,
            exec_out,
            data_in,
            data_out,
            wildcard_constraints,
        })
    }
}

// buns_system 宏参数解析
struct SystemAttributes {
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    params: Option<Vec<ParamDefinition>>,
    outputs: Option<Vec<OutputFieldDefinition>>,
    // 行为元数据（参照MCP）
    destructive: Option<bool>, // 是否会破坏性修改环境
    readonly: Option<bool>,    // 是否只读操作
    idempotent: Option<bool>,  // 是否幂等
    open_world: Option<bool>,  // 是否与开放世界交互
    secret: Option<bool>,      // 是否处理敏感信息
}

// 参数定义结构（支持完整元数据）
struct ParamDefinition {
    name: String,
    param_type: String,            // 类型：String, i32, bool等
    required: bool,                // 是否必填
    default_value: Option<String>, // 默认值
    description: String,           // 详细描述
}

// 输出字段定义结构
struct OutputFieldDefinition {
    name: String,
    field_type: String,
    description: String,
}

impl Parse for SystemAttributes {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // 第一个参数必须是字符串（系统名称）
        let name: LitStr = input.parse()?;
        let name = name.value();

        let mut description = None;
        let mut display_name = None;
        let mut params = None;
        let mut outputs = None;
        let mut destructive = None;
        let mut readonly = None;
        let mut idempotent = None;
        let mut open_world = None;
        let mut secret = None;

        // 解析可选参数
        while !input.is_empty() {
            input.parse::<Token![,]>()?;

            if input.is_empty() {
                break;
            }

            // 检查是否是 params {...} 块
            let lookahead = input.lookahead1();
            if lookahead.peek(syn::Ident) {
                let ident: syn::Ident = input.fork().parse()?;

                if ident == "params" {
                    // 解析 params { ... }
                    input.parse::<syn::Ident>()?; // 消耗 "params"

                    let content;
                    syn::braced!(content in input);

                    let mut param_list = Vec::new();
                    while !content.is_empty() {
                        // 解析参数名
                        let param_name: syn::Ident = content.parse()?;
                        content.parse::<Token![:]>()?;

                        // 检查是否是新格式（结构体）还是旧格式（字符串）
                        let lookahead = content.lookahead1();

                        if lookahead.peek(syn::LitStr) {
                            // 旧格式：name: "description"
                            let desc: LitStr = content.parse()?;
                            param_list.push(ParamDefinition {
                                name: param_name.to_string(),
                                param_type: "String".to_string(), // 默认String
                                required: desc.value().contains("必填"), // 简单判断
                                default_value: None,
                                description: desc.value(),
                            });
                        } else {
                            // 新格式：name { type: String, required: true, ... }
                            let param_content;
                            syn::braced!(param_content in content);

                            let mut param_type = "String".to_string();
                            let mut required = false;
                            let mut default_value = None;
                            let mut description = String::new();

                            while !param_content.is_empty() {
                                let key: syn::Ident = param_content.parse()?;
                                param_content.parse::<Token![:]>()?;

                                match key.to_string().as_str() {
                                    "type" => {
                                        let ty: syn::Ident = param_content.parse()?;
                                        param_type = ty.to_string();
                                    }
                                    "required" => {
                                        let val: syn::LitBool = param_content.parse()?;
                                        required = val.value;
                                    }
                                    "default" => {
                                        // 支持多种字面量
                                        if let Ok(s) = param_content.parse::<LitStr>() {
                                            default_value = Some(s.value());
                                        } else if let Ok(i) = param_content.parse::<syn::LitInt>() {
                                            default_value = Some(i.base10_digits().to_string());
                                        } else if let Ok(b) = param_content.parse::<syn::LitBool>()
                                        {
                                            default_value = Some(b.value.to_string());
                                        } else if let Ok(f) = param_content.parse::<syn::LitFloat>()
                                        {
                                            default_value = Some(f.base10_digits().to_string());
                                        }
                                    }
                                    "description" => {
                                        let desc: LitStr = param_content.parse()?;
                                        description = desc.value();
                                    }
                                    _ => {
                                        return Err(syn::Error::new(
                                            key.span(),
                                            format!("Unknown param attribute: {}", key),
                                        ));
                                    }
                                }

                                if !param_content.is_empty() {
                                    param_content.parse::<Token![,]>()?;
                                }
                            }

                            param_list.push(ParamDefinition {
                                name: param_name.to_string(),
                                param_type,
                                required,
                                default_value,
                                description,
                            });
                        }

                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }

                    params = Some(param_list);
                } else if ident == "description" {
                    // 解析 description = "..."
                    input.parse::<syn::Ident>()?; // 消耗 "description"
                    input.parse::<Token![=]>()?;
                    let desc: LitStr = input.parse()?;
                    description = Some(desc.value());
                } else if ident == "display_name" {
                    input.parse::<syn::Ident>()?; // 消耗 "display_name"
                    input.parse::<Token![=]>()?;
                    let value: LitStr = input.parse()?;
                    display_name = Some(value.value());
                } else if ident == "outputs" {
                    // 解析 outputs { ... }
                    input.parse::<syn::Ident>()?; // 消耗 "outputs"

                    let content;
                    syn::braced!(content in input);

                    let mut output_list = Vec::new();
                    while !content.is_empty() {
                        // 解析字段名
                        let field_name: syn::Ident = content.parse()?;
                        content.parse::<Token![:]>()?;

                        // 解析类型和描述（格式：type@描述 或 type）
                        let type_and_desc: LitStr = content.parse()?;
                        let type_and_desc_str = type_and_desc.value();

                        // 分割类型和描述
                        let (field_type, description) =
                            if let Some(idx) = type_and_desc_str.find('@') {
                                (
                                    type_and_desc_str[..idx].trim().to_string(),
                                    type_and_desc_str[idx + 1..].trim().to_string(),
                                )
                            } else {
                                (type_and_desc_str.trim().to_string(), String::new())
                            };

                        output_list.push(OutputFieldDefinition {
                            name: field_name.to_string(),
                            field_type,
                            description,
                        });

                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }

                    outputs = Some(output_list);
                } else if ident == "destructive" {
                    input.parse::<syn::Ident>()?; // 消耗 "destructive"
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    destructive = Some(val.value);
                } else if ident == "readonly" {
                    input.parse::<syn::Ident>()?; // 消耗 "readonly"
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    readonly = Some(val.value);
                } else if ident == "idempotent" {
                    input.parse::<syn::Ident>()?; // 消耗 "idempotent"
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    idempotent = Some(val.value);
                } else if ident == "open_world" {
                    input.parse::<syn::Ident>()?; // 消耗 "open_world"
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    open_world = Some(val.value);
                } else if ident == "secret" {
                    input.parse::<syn::Ident>()?; // 消耗 "secret"
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    secret = Some(val.value);
                } else {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("Unknown attribute: {}", ident),
                    ));
                }
            } else {
                break;
            }
        }

        Ok(SystemAttributes {
            name,
            display_name,
            description,
            params,
            outputs,
            destructive,
            readonly,
            idempotent,
            open_world,
            secret,
        })
    }
}

// ============================================================================
// define_operation 宏 — 统一 AI 系统与节点注册
// ============================================================================

/// 从驼峰名称生成显示名称：`ClickElement` → `Click Element`
fn camel_to_display_name(name: &str) -> String {
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if i > 0 && ch.is_uppercase() {
            // 避免连续大写拆分（如 "URL" 不拆成 "U R L"）
            let prev = name.chars().nth(i - 1).unwrap_or('a');
            if prev.is_lowercase() {
                result.push(' ');
            }
        }
        result.push(ch);
    }
    result
}

// Operation 参数定义
struct OperationParam {
    name: String,
    param_type: String,
    description: String,
    required: bool,
}

// Operation 输出定义
struct OperationOutput {
    name: String,
    output_type: String,
    description: String,
}

// define_operation 宏属性
struct OperationAttributes {
    name: String,
    description: String,
    category: String,
    system_only: bool,
    display_name: Option<String>,
    params: Vec<OperationParam>,
    outputs: Vec<OperationOutput>,
    exec_in: Vec<String>,
    exec_out: Vec<String>,
    destructive: Option<bool>,
    readonly: Option<bool>,
    idempotent: Option<bool>,
    open_world: Option<bool>,
    secret: Option<bool>,
}

impl Parse for OperationAttributes {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut description = None;
        let mut category = None;
        let mut system_only = false;
        let mut display_name = None;
        let mut params = Vec::new();
        let mut outputs = Vec::new();
        let mut exec_in = Vec::new();
        let mut exec_out = Vec::new();
        let mut destructive = None;
        let mut readonly = None;
        let mut idempotent = None;
        let mut open_world = None;
        let mut secret = None;

        while !input.is_empty() {
            let key: syn::Ident = input.parse()?;

            match key.to_string().as_str() {
                "name" => {
                    input.parse::<Token![=]>()?;
                    let val: LitStr = input.parse()?;
                    name = Some(val.value());
                }
                "description" => {
                    input.parse::<Token![=]>()?;
                    let val: LitStr = input.parse()?;
                    description = Some(val.value());
                }
                "category" => {
                    input.parse::<Token![=]>()?;
                    let val: LitStr = input.parse()?;
                    category = Some(val.value());
                }
                "system_only" => {
                    system_only = true;
                    // system_only 是裸标志，无 = value
                }
                "display_name" => {
                    input.parse::<Token![=]>()?;
                    let val: LitStr = input.parse()?;
                    display_name = Some(val.value());
                }
                "params" => {
                    let content;
                    syn::braced!(content in input);
                    while !content.is_empty() {
                        let param_name: syn::Ident = content.parse()?;
                        content.parse::<Token![:]>()?;
                        let type_desc: LitStr = content.parse()?;
                        let type_desc_str = type_desc.value();

                        // 格式: "Type@Description"
                        let (param_type, desc) = if let Some(at_pos) = type_desc_str.find('@') {
                            (
                                type_desc_str[..at_pos].to_string(),
                                type_desc_str[at_pos + 1..].to_string(),
                            )
                        } else {
                            ("String".to_string(), type_desc_str)
                        };

                        let required = desc.contains("必填");

                        params.push(OperationParam {
                            name: param_name.to_string(),
                            param_type,
                            description: desc,
                            required,
                        });

                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                }
                "outputs" => {
                    let content;
                    syn::braced!(content in input);
                    while !content.is_empty() {
                        let output_name: syn::Ident = content.parse()?;
                        content.parse::<Token![:]>()?;
                        let type_desc: LitStr = content.parse()?;
                        let type_desc_str = type_desc.value();

                        let (output_type, desc) = if let Some(at_pos) = type_desc_str.find('@') {
                            (
                                type_desc_str[..at_pos].to_string(),
                                type_desc_str[at_pos + 1..].to_string(),
                            )
                        } else {
                            ("String".to_string(), type_desc_str)
                        };

                        outputs.push(OperationOutput {
                            name: output_name.to_string(),
                            output_type,
                            description: desc,
                        });

                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                }
                "exec_in" => {
                    input.parse::<Token![=]>()?;
                    exec_in = parse_string_array(input)?;
                }
                "exec_out" => {
                    input.parse::<Token![=]>()?;
                    exec_out = parse_string_array(input)?;
                }
                "destructive" => {
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    destructive = Some(val.value);
                }
                "readonly" => {
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    readonly = Some(val.value);
                }
                "idempotent" => {
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    idempotent = Some(val.value);
                }
                "open_world" => {
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    open_world = Some(val.value);
                }
                "secret" => {
                    input.parse::<Token![=]>()?;
                    let val: syn::LitBool = input.parse()?;
                    secret = Some(val.value);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("Unknown attribute: {}", key),
                    ))
                }
            }

            // 消耗可选的尾逗号
            if !input.is_empty() {
                let _ = input.parse::<Token![,]>();
            }
        }

        Ok(OperationAttributes {
            name: name.ok_or_else(|| input.error("Missing 'name'"))?,
            description: description.ok_or_else(|| input.error("Missing 'description'"))?,
            category: category.unwrap_or_default(),
            system_only,
            display_name,
            params,
            outputs,
            exec_in,
            exec_out,
            destructive,
            readonly,
            idempotent,
            open_world,
            secret,
        })
    }
}

/// 生成参数提取代码（DataValue → String，用于构建 AIInput）
fn gen_param_extraction(param: &OperationParam) -> proc_macro2::TokenStream {
    let name = &param.name;
    let name_ident = syn::Ident::new(&param.name, proc_macro2::Span::call_site());

    let extract_expr = match param.param_type.as_str() {
        "String" => quote! {
            __inputs.get(#name).and_then(|v| v.as_str()).map(|s| s.to_string())
        },
        "i64" | "Number" | "USize" => quote! {
            __inputs.get(#name).and_then(|v| v.as_i64()).map(|n| n.to_string())
        },
        "bool" => quote! {
            __inputs.get(#name).and_then(|v| v.as_bool()).map(|b| b.to_string())
        },
        _ => quote! {
            __inputs.get(#name).map(|v| ::serde_json::to_string(&v.value).unwrap_or_default())
        },
    };

    if param.required {
        quote! {
            let #name_ident = (#extract_expr)
                .ok_or_else(|| ::corework::error::FrameworkError::SystemError(
                    format!("Missing required param: {}", #name)
                ))?;
            __args.insert(#name.to_string(), #name_ident);
        }
    } else {
        quote! {
            if let Some(__v) = #extract_expr {
                if !__v.is_empty() {
                    __args.insert(#name.to_string(), __v);
                }
            }
        }
    }
}

/// 生成输出提取代码（AIOutput.result JSON → DataValue）
fn gen_output_extraction(output: &OperationOutput) -> proc_macro2::TokenStream {
    let name = &output.name;
    match output.output_type.as_str() {
        "String" => quote! {
            if let Some(v) = __result.get(#name).and_then(|v| v.as_str()) {
                __data.insert(#name.to_string(), ::corework::workflow::core::DataValue::from_string(v));
            }
        },
        "i64" | "Number" => quote! {
            if let Some(v) = __result.get(#name).and_then(|v| v.as_i64()) {
                __data.insert(#name.to_string(), ::corework::workflow::core::DataValue::from_i64(v));
            }
        },
        "bool" => quote! {
            if let Some(v) = __result.get(#name).and_then(|v| v.as_bool()) {
                __data.insert(#name.to_string(), ::corework::workflow::core::DataValue::from_bool(v));
            }
        },
        _ => quote! {
            if let Some(v) = __result.get(#name) {
                __data.insert(#name.to_string(), ::corework::workflow::core::DataValue::from_string(
                    ::serde_json::to_string(v).unwrap_or_default()
                ));
            }
        },
    }
}

/// #[define_operation(...)] — 统一 AI 系统与节点注册
///
/// 从单一定义同时生成 AI 系统注册和蓝图节点注册，消除重复，杜绝漂移。
///
/// # 始终生成（AI 系统侧）
/// 1. `impl Default`
/// 2. `impl DynamicExecute`
/// 3. `inventory::submit!(SystemFactory { ... })`
/// 4. `inventory::submit!(AISystemFactory { ... })`
///
/// # 非 system_only 时额外生成（节点侧）
/// 5. `{Name}NodeWrapper` 结构体 + Default
/// 6. `impl BlueprintNode for {Name}NodeWrapper`
/// 7. `const __NODE_{NAME}_PINS__`
/// 8. `inventory::submit!(NodeMetadata { ... })`
/// 9. `inventory::submit!(NodeFactory { ... })`
#[proc_macro_attribute]
pub fn define_operation(attr: TokenStream, item: TokenStream) -> TokenStream {
    let op = parse_macro_input!(attr as OperationAttributes);
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;
    let struct_name_str = struct_name.to_string();
    let name_str = &op.name;
    let ai_display_name = op
        .display_name
        .clone()
        .unwrap_or_else(|| camel_to_display_name(name_str));
    let description = &op.description;

    // ── AI 系统侧：Default ──
    let default_impl = quote! {
        impl ::std::default::Default for #struct_name {
            fn default() -> Self { Self }
        }
    };

    // ── AI 系统侧：DynamicExecute ──
    let dynamic_execute_impl = quote! {
        #[::async_trait::async_trait]
        impl ::corework::prelude::DynamicExecute for #struct_name
        where
            Self: ::corework::system::SystemOperation,
            <Self as ::corework::system::SystemOperation>::Input: ::corework::cache::CacheValue,
            <Self as ::corework::system::SystemOperation>::Output: ::corework::cache::CacheValue,
        {
            async fn execute_dynamic(
                &self,
                input: ::std::collections::HashMap<::std::string::String, ::serde_json::Value>,
                ctx: &::corework::orchestration::Context,
            ) -> ::corework::error::Result<::serde_json::Value> {
                let input_value = ::serde_json::Value::Object(
                    input.into_iter().collect::<::serde_json::Map<_, _>>()
                );
                let typed_input: <Self as ::corework::system::SystemOperation>::Input =
                    ::serde_json::from_value(input_value)
                        .map_err(|e| ::corework::error::FrameworkError::InvalidData(
                            format!("Failed to deserialize input: {}", e)
                        ))?;
                let output = <Self as ::corework::system::SystemOperation>::execute(
                    self, typed_input, ctx
                ).await.map_err(|e| ::corework::error::FrameworkError::SystemError(
                    format!("{:?}", e)
                ))?;
                let output_value = ::serde_json::to_value(&output)
                    .map_err(|e| ::corework::error::FrameworkError::SerializationError(e))?;
                Ok(output_value)
            }
        }
    };

    // ── AI 系统侧：SystemFactory ──
    // 注意：使用 struct 名（而非 operation name）作为 key，
    // 因为 system_by_type::<T>() 通过 Rust 类型名的最后一段查找。
    let system_factory = quote! {
        ::inventory::submit! {
            ::corework::SystemFactory {
                name: #struct_name_str,
                description: ::std::option::Option::Some(#description),
                constructor: || ::std::sync::Arc::new(#struct_name::default()),
                dynamic_constructor: Some(|| ::std::sync::Arc::new(#struct_name::default())),
            }
        }
    };

    // Also register the operation name as a dynamic system alias. AI EXEC lines
    // and AISystemFactory metadata use `name = "..."`, while system_by_type::<T>()
    // still needs the Rust struct-name key above.
    let operation_name_system_factory = if *name_str != struct_name_str {
        quote! {
            ::inventory::submit! {
                ::corework::SystemFactory {
                    name: #name_str,
                    description: ::std::option::Option::Some(#description),
                    constructor: || ::std::sync::Arc::new(#struct_name::default()),
                    dynamic_constructor: Some(|| ::std::sync::Arc::new(#struct_name::default())),
                }
            }
        }
    } else {
        quote! {}
    };

    // ── AI 系统侧：AISystemFactory + 参数/输出常量 ──
    let params_const_name = syn::Ident::new(
        &format!("__AI_PARAMS_{}__", struct_name.to_string().to_uppercase()),
        struct_name.span(),
    );
    let outputs_const_name = syn::Ident::new(
        &format!("__AI_OUTPUTS_{}__", struct_name.to_string().to_uppercase()),
        struct_name.span(),
    );

    let param_defs: Vec<_> = op
        .params
        .iter()
        .map(|p| {
            let pname = &p.name;
            let ptype = &p.param_type;
            let required = p.required;
            let pdesc = &p.description;
            quote! {
                ::corework::ai_system::AIParameter {
                    name: #pname,
                    param_type: #ptype,
                    required: #required,
                    default_value: ::std::option::Option::None,
                    description: #pdesc,
                }
            }
        })
        .collect();

    let output_field_defs: Vec<_> = op
        .outputs
        .iter()
        .map(|o| {
            let oname = &o.name;
            let otype = &o.output_type;
            let odesc = &o.description;
            quote! {
                ::corework::ai_system::AIOutputField {
                    name: #oname,
                    field_type: #otype,
                    description: #odesc,
                }
            }
        })
        .collect();

    let destructive_val = op.destructive.unwrap_or(true);
    let readonly_val = op.readonly.unwrap_or(false);
    let idempotent_val = op.idempotent.unwrap_or(false);
    let open_world_val = op.open_world.unwrap_or(true);
    let secret_val = op.secret.unwrap_or(false);

    let ai_factory = quote! {
        #[allow(non_upper_case_globals)]
        const #params_const_name: &[::corework::ai_system::AIParameter] = &[
            #(#param_defs),*
        ];

        #[allow(non_upper_case_globals)]
        const #outputs_const_name: &[::corework::ai_system::AIOutputField] = &[
            #(#output_field_defs),*
        ];

        ::inventory::submit! {
            ::corework::ai_system::AISystemFactory {
                metadata: ::corework::ai_system::AISystemMetadata {
                    name: #name_str,
                    display_name: #ai_display_name,
                    description: #description,
                    tool_kind: "local",
                    parameters: #params_const_name,
                    outputs: #outputs_const_name,
                    destructive: #destructive_val,
                    readonly: #readonly_val,
                    idempotent: #idempotent_val,
                    open_world: #open_world_val,
                    secret: #secret_val,
                },
                constructor: || ::std::sync::Arc::new(#struct_name::default()),
            }
        }
    };

    // ── 节点侧（非 system_only）──
    let node_side = if !op.system_only {
        let wrapper_name =
            syn::Ident::new(&format!("{}NodeWrapper", struct_name), struct_name.span());
        let display_name_str = op
            .display_name
            .clone()
            .unwrap_or_else(|| camel_to_display_name(name_str));
        let category_str = &op.category;

        // 引脚元数据
        let mut pin_defs = Vec::new();

        for pin_def_str in &op.exec_in {
            let (pname, _, pdesc, _) = parse_pin_definition(pin_def_str);
            let pdesc_lit = syn::LitStr::new(pdesc, proc_macro2::Span::call_site());
            pin_defs.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #pname,
                    kind: ::corework::prelude::PinKind::ExecInput,
                    data_type: "",
                    description: #pdesc_lit,
                    default_value: None,
                }
            });
        }

        // params → DataInput 引脚
        for p in &op.params {
            let pname = &p.name;
            let ptype = &p.param_type;
            let pdesc = &p.description;
            pin_defs.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #pname,
                    kind: ::corework::prelude::PinKind::DataInput,
                    data_type: #ptype,
                    description: #pdesc,
                    default_value: None,
                }
            });
        }

        // outputs → DataOutput 引脚
        for o in &op.outputs {
            let oname = &o.name;
            let otype = &o.output_type;
            let odesc = &o.description;
            pin_defs.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #oname,
                    kind: ::corework::prelude::PinKind::DataOutput,
                    data_type: #otype,
                    description: #odesc,
                    default_value: None,
                }
            });
        }

        for pin_def_str in &op.exec_out {
            let (pname, _, pdesc, _) = parse_pin_definition(pin_def_str);
            let pdesc_lit = syn::LitStr::new(pdesc, proc_macro2::Span::call_site());
            pin_defs.push(quote! {
                ::corework::prelude::PinMetadata {
                    name: #pname,
                    kind: ::corework::prelude::PinKind::ExecOutput,
                    data_type: "",
                    description: #pdesc_lit,
                    default_value: None,
                }
            });
        }

        let pins_const = syn::Ident::new(
            &format!("__NODE_{}_PINS__", name_str.to_uppercase()),
            struct_name.span(),
        );
        let constraints_const = syn::Ident::new(
            &format!("__NODE_{}_CONSTRAINTS__", name_str.to_uppercase()),
            struct_name.span(),
        );
        let factory_fn = syn::Ident::new(
            &format!("__NODE_FACTORY_{}__", name_str.to_uppercase()),
            struct_name.span(),
        );

        // 生成 pins() 的 Vec<Pin> 构造
        let mut runtime_pins = Vec::new();
        for pin_def_str in &op.exec_in {
            let (pname, _, _, _) = parse_pin_definition(pin_def_str);
            runtime_pins.push(quote! { ::corework::prelude::Pin::exec_in(#pname) });
        }
        for p in &op.params {
            let pname = &p.name;
            let ptype = &p.param_type;
            runtime_pins.push(quote! { ::corework::prelude::Pin::data_in(#pname, #ptype) });
        }
        for o in &op.outputs {
            let oname = &o.name;
            let otype = &o.output_type;
            runtime_pins.push(quote! { ::corework::prelude::Pin::data_out(#oname, #otype) });
        }
        for pin_def_str in &op.exec_out {
            let (pname, _, _, _) = parse_pin_definition(pin_def_str);
            runtime_pins.push(quote! { ::corework::prelude::Pin::exec_out(#pname) });
        }

        // 生成 execute_node 内部的参数提取代码
        let param_extractions: Vec<_> = op.params.iter().map(gen_param_extraction).collect();

        // 生成输出处理代码
        let output_handling = if op.outputs.is_empty() {
            // 无 outputs → ExecPin(第一个 exec_out)
            let first_exec_out = op
                .exec_out
                .first()
                .map(|s| parse_pin_definition(s).0.to_string())
                .unwrap_or_else(|| "Then".to_string());
            quote! {
                Ok(::corework::workflow::core::NodeOutput::ExecPin(#first_exec_out.to_string()))
            }
        } else {
            let output_extractions: Vec<_> = op.outputs.iter().map(gen_output_extraction).collect();
            quote! {
                {
                    let __result = &__output.result;
                    let mut __data = ::std::collections::HashMap::new();
                    #(#output_extractions)*
                    Ok(::corework::workflow::core::NodeOutput::Data(__data))
                }
            }
        };

        quote! {
            // ── NodeWrapper 结构体 ──
            #[derive(Debug, Clone)]
            struct #wrapper_name;

            impl ::std::default::Default for #wrapper_name {
                fn default() -> Self { Self }
            }

            // ── 引脚元数据常量 ──
            #[allow(non_upper_case_globals)]
            const #pins_const: &[::corework::prelude::PinMetadata] = &[
                #(#pin_defs),*
            ];

            #[allow(non_upper_case_globals)]
            const #constraints_const: &[(&str, &[&str])] = &[];

            // ── BlueprintNode 实现 ──
            impl ::corework::workflow::nodes::traits::BlueprintNode for #wrapper_name {
                fn name(&self) -> &str {
                    #name_str
                }

                fn node_type(&self) -> ::corework::workflow::nodes::traits::NodeType {
                    ::corework::workflow::nodes::traits::NodeType::Impure
                }

                fn pins(&self) -> Vec<::corework::prelude::Pin> {
                    vec![
                        #(#runtime_pins),*
                    ]
                }

                fn description(&self) -> Option<&str> {
                    Some(#description)
                }

                fn category(&self) -> Option<&str> {
                    Some(#category_str)
                }

                fn execute_node<'a>(
                    &'a self,
                    ctx: &'a mut ::corework::workflow::execution::ExecutionContext,
                    inputs: ::std::collections::HashMap<String, ::corework::workflow::core::DataValue>,
                ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = ::corework::error::Result<::corework::workflow::core::NodeOutput>> + Send + 'a>> {
                    Box::pin(async move {
                        let mut __inputs = inputs;
                        let mut __args = ::std::collections::HashMap::<String, String>::new();

                        #(#param_extractions)*

                        let __legacy_ctx = ctx.inner();
                        let __system = __legacy_ctx.system_by_type::<#struct_name>()?;
                        let __output = __system.execute(
                            ::corework::ai_system::AIInput::from_args(__args),
                            __legacy_ctx,
                        ).await.map_err(|e| ::corework::error::FrameworkError::SystemError(
                            format!("{} 执行失败: {}", #name_str, e)
                        ))?;

                        if __output.error_code != 0 {
                            return Err(::corework::error::FrameworkError::SystemError(
                                format!("{} 执行失败: {}", #name_str, __output.to_ai)
                            ));
                        }

                        #output_handling
                    })
                }
            }

            // ── 节点工厂函数 ──
            #[allow(non_snake_case)]
            fn #factory_fn(_name: String) -> ::std::sync::Arc<dyn ::corework::workflow::nodes::traits::BlueprintNode + Send + Sync> {
                ::std::sync::Arc::new(#wrapper_name::default())
            }

            // ── NodeMetadata 注册 ──
            ::inventory::submit! {
                ::corework::prelude::NodeMetadata {
                    node_type: #name_str,
                    version: "1.0.0",
                    category: #category_str,
                    display_name: #display_name_str,
                    description: #description,
                    pins: #pins_const,
                    permissions: ::corework::prelude::NodePermissions::new(0),
                    wildcard_constraints: #constraints_const,
                }
            }

            // ── NodeFactory 注册 ──
            ::inventory::submit! {
                ::corework::prelude::NodeFactory {
                    node_type: #name_str,
                    factory: #factory_fn,
                }
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #input

        #default_impl
        #dynamic_execute_impl
        #system_factory
        #operation_name_system_factory
        #ai_factory
        #node_side
    };

    TokenStream::from(expanded)
}

// buns_model 宏参数解析
struct ModelAttributes {
    name: String,
    version: String,
    description: String,
    category: String,
    exportable: bool,
}

impl Parse for ModelAttributes {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;

        // 尝试解析 version（可选）
        let next_token = input.fork();
        let version = if let Ok(ver) = next_token.parse::<LitStr>() {
            // 检查是否为版本号格式（x.y.z）
            if ver.value().matches('.').count() >= 1 {
                input.parse::<LitStr>()?; // 消耗
                input.parse::<Token![,]>()?;
                ver.value()
            } else {
                "1.0.0".to_string() // 默认版本
            }
        } else {
            "1.0.0".to_string()
        };

        let description: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        let category: LitStr = input.parse()?;

        // 解析可选的 exportable 参数
        let exportable = if !input.is_empty() {
            input.parse::<Token![,]>()?;

            // 解析 exportable = true/false
            let ident: syn::Ident = input.parse()?;
            if ident != "exportable" {
                return Err(syn::Error::new(ident.span(), "Expected 'exportable'"));
            }
            input.parse::<Token![=]>()?;
            let value: syn::LitBool = input.parse()?;
            value.value
        } else {
            true // 默认导出
        };

        Ok(ModelAttributes {
            name: name.value(),
            version,
            description: description.value(),
            category: category.value(),
            exportable,
        })
    }
}
