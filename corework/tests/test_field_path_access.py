#!/usr/bin/env python3
"""
生成测试字段路径访问功能的蓝图 JSON

此脚本生成一个蓝图，展示如何使用字段路径从复杂结构体中提取字段
"""

import json
from pathlib import Path


def create_field_path_test_blueprint():
    """创建测试字段路径访问的蓝图"""
    
    blueprint = {
        "metadata": {
            "name": "FieldPathAccessTest",
            "description": "测试字段路径访问功能 - 从 SubQuestion 提取字段",
            "version": "1.0.0"
        },
        "nodes": [
            # Start 节点
            {
                "id": "start_1",
                "node_type": "StartNode",
                "display_name": "Start",
                "position": {"x": 100, "y": 200},
                "outputs": [
                    {
                        "pin_name": "reference",
                        "data_type": "ReferenceAnswer",
                        "description": "参考答案（包含 sub_questions）"
                    }
                ]
            },
            
            # 字段提取节点 - 提取 sub_id
            {
                "id": "extract_sub_id",
                "node_type": "ExtractFieldNode",
                "display_name": "提取题目ID",
                "position": {"x": 400, "y": 150},
                "inputs": [
                    {
                        "pin_name": "reference",
                        "data_type": "ReferenceAnswer",
                        "description": "输入的参考答案对象",
                        # 使用字段路径访问
                        "field_path": "sub_questions.0.sub_id"
                    }
                ],
                "outputs": [
                    {
                        "pin_name": "sub_id",
                        "data_type": "String",
                        "description": "提取的题目ID"
                    }
                ]
            },
            
            # 字段提取节点 - 提取 question_type
            {
                "id": "extract_question_type",
                "node_type": "ExtractFieldNode",
                "display_name": "提取题目类型",
                "position": {"x": 400, "y": 250},
                "inputs": [
                    {
                        "pin_name": "reference",
                        "data_type": "ReferenceAnswer",
                        "field_path": "sub_questions.0.question_type"
                    }
                ],
                "outputs": [
                    {
                        "pin_name": "question_type",
                        "data_type": "String"
                    }
                ]
            },
            
            # 字段提取节点 - 提取 full_score
            {
                "id": "extract_full_score",
                "node_type": "ExtractFieldNode",
                "display_name": "提取满分值",
                "position": {"x": 400, "y": 350},
                "inputs": [
                    {
                        "pin_name": "reference",
                        "data_type": "ReferenceAnswer",
                        "field_path": "sub_questions.0.full_score"
                    }
                ],
                "outputs": [
                    {
                        "pin_name": "full_score",
                        "data_type": "Float"
                    }
                ]
            },
            
            # 打印节点 - 显示提取的字段
            {
                "id": "print_fields",
                "node_type": "PrintNode",
                "display_name": "打印字段",
                "position": {"x": 700, "y": 200},
                "inputs": [
                    {
                        "pin_name": "sub_id",
                        "data_type": "String"
                    },
                    {
                        "pin_name": "question_type",
                        "data_type": "String"
                    },
                    {
                        "pin_name": "full_score",
                        "data_type": "Float"
                    }
                ]
            },
            
            # End 节点
            {
                "id": "end_1",
                "node_type": "EndNode",
                "display_name": "End",
                "position": {"x": 1000, "y": 200},
                "inputs": [
                    {
                        "pin_name": "sub_id",
                        "data_type": "String"
                    },
                    {
                        "pin_name": "question_type",
                        "data_type": "String"
                    },
                    {
                        "pin_name": "full_score",
                        "data_type": "Float"
                    }
                ]
            }
        ],
        
        "connections": [
            # 执行流
            {"from_node": "start_1", "from_pin": "Out", "to_node": "extract_sub_id", "to_pin": "In"},
            {"from_node": "extract_sub_id", "from_pin": "Out", "to_node": "extract_question_type", "to_pin": "In"},
            {"from_node": "extract_question_type", "from_pin": "Out", "to_node": "extract_full_score", "to_pin": "In"},
            {"from_node": "extract_full_score", "from_pin": "Out", "to_node": "print_fields", "to_pin": "In"},
            {"from_node": "print_fields", "from_pin": "Out", "to_node": "end_1", "to_pin": "In"},
            
            # 数据流 - reference 分发到三个提取节点
            {"from_node": "start_1", "from_pin": "reference", "to_node": "extract_sub_id", "to_pin": "reference"},
            {"from_node": "start_1", "from_pin": "reference", "to_node": "extract_question_type", "to_pin": "reference"},
            {"from_node": "start_1", "from_pin": "reference", "to_node": "extract_full_score", "to_pin": "reference"},
            
            # 提取的字段传递到打印节点
            {"from_node": "extract_sub_id", "from_pin": "sub_id", "to_node": "print_fields", "to_pin": "sub_id"},
            {"from_node": "extract_question_type", "from_pin": "question_type", "to_node": "print_fields", "to_pin": "question_type"},
            {"from_node": "extract_full_score", "from_pin": "full_score", "to_node": "print_fields", "to_pin": "full_score"},
            
            # 传递到 End
            {"from_node": "extract_sub_id", "from_pin": "sub_id", "to_node": "end_1", "to_pin": "sub_id"},
            {"from_node": "extract_question_type", "from_pin": "question_type", "to_node": "end_1", "to_pin": "question_type"},
            {"from_node": "extract_full_score", "from_pin": "full_score", "to_node": "end_1", "to_pin": "full_score"}
        ]
    }
    
    return blueprint


def create_simple_print_test():
    """创建简化版本 - 只使用内置节点测试字段路径"""
    
    blueprint = {
        "metadata": {
            "name": "SimpleFieldPathTest",
            "description": "简化版字段路径测试 - 使用 StringNode 验证",
            "version": "1.0.0"
        },
        "nodes": [
            {
                "id": "start_1",
                "node_type": "StartNode",
                "display_name": "Start",
                "position": {"x": 100, "y": 200},
                "outputs": [
                    {
                        "pin_name": "test_data",
                        "data_type": "Object",
                        "description": "测试对象"
                    }
                ]
            },
            
            # 使用 String 常量节点模拟字段路径访问
            {
                "id": "field_path_1",
                "node_type": "StringNode",
                "display_name": "字段路径: name",
                "position": {"x": 400, "y": 150},
                "properties": {
                    "value": "name",
                    "description": "要访问的字段路径"
                }
            },
            
            {
                "id": "end_1",
                "node_type": "EndNode",
                "display_name": "End",
                "position": {"x": 700, "y": 200},
                "inputs": [
                    {
                        "pin_name": "field_path",
                        "data_type": "String"
                    }
                ]
            }
        ],
        
        "connections": [
            {"from_node": "start_1", "from_pin": "Out", "to_node": "field_path_1", "to_pin": "In"},
            {"from_node": "field_path_1", "from_pin": "Out", "to_node": "end_1", "to_pin": "In"},
            {"from_node": "field_path_1", "from_pin": "Value", "to_node": "end_1", "to_pin": "field_path"}
        ]
    }
    
    return blueprint


def main():
    """主函数"""
    
    # 获取输出目录
    output_dir = Path(__file__).parent
    
    # 生成完整测试蓝图
    blueprint = create_field_path_test_blueprint()
    output_file = output_dir / "test_field_path_access.json"
    
    with open(output_file, 'w', encoding='utf-8') as f:
        json.dump(blueprint, f, indent=2, ensure_ascii=False)
    
    print(f"✅ 已生成测试蓝图: {output_file}")
    print(f"   节点数: {len(blueprint['nodes'])}")
    print(f"   连接数: {len(blueprint['connections'])}")
    
    # 生成简化版测试蓝图
    simple_blueprint = create_simple_print_test()
    simple_output = output_dir / "test_field_path_simple.json"
    
    with open(simple_output, 'w', encoding='utf-8') as f:
        json.dump(simple_blueprint, f, indent=2, ensure_ascii=False)
    
    print(f"✅ 已生成简化测试蓝图: {simple_output}")


if __name__ == "__main__":
    main()
