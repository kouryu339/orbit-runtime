{
  "schema": "orbit.workflow.logic/v1",
  "inputs": {
    "path": {
      "type": "String",
      "disablename": "Excel 文件",
      "description": "待处理的 Excel 文件路径；文件必须存在。"
    },
    "sheet": {
      "type": "String",
      "default": "申请表",
      "disablename": "工作表",
      "description": "目标工作表名称；必须存在于文件中。"
    },
    "column": {
      "type": "String",
      "default": "姓名",
      "disablename": "读取列",
      "description": "选择读取列；仅支持姓名或手机号。"
    }
  },
  "variables": {
    "processed_count": {
      "type": "i64",
      "initial": 0,
      "disablename": "已遍历行数"
    }
  },
  "steps": [
    {
      "id": 1,
      "call": "ExcelOpenSession",
      "args": {
        "path": input.path
      }
    },
    {
      "id": 2,
      "call": "ExcelQuerySessionRows",
      "args": {
        "session_id": 1.session_id,
        "sheet": input.sheet,
        "query": "[是否执行] IS_EMPTY",
        "parameters": []
      }
    },
    {
      "id": 3,
      "for": {
        "step": 2,
        "output": "row_numbers"
      },
      "body": [
        {
          "id": 4,
          "switch": {
            "input": "column"
          },
          "cases": [
            {
              "value": "姓名",
              "body": [
                {
                  "id": 5,
                  "call": "ExcelReadSessionValueAtRow",
                  "args": {
                    "session_id": 1.session_id,
                    "sheet": input.sheet,
                    "row_number": var.item,
                    "column": "姓名"
                  }
                },
                {
                  "id": 6,
                  "if": {
                    "fn": "eq",
                    "args": [
                      {
                        "step": 5,
                        "output": "value"
                      },
                      ""
                    ]
                  },
                  "then": [
                    {
                      "id": 7,
                      "call": "DebugPrintNode",
                      "args": {
                        "Value": "姓名为空"
                      }
                    }
                  ],
                  "otherwise": [
                    {
                      "id": 8,
                      "call": "DebugPrintNode",
                      "args": {
                        "Value": split(5.value, ["，", ","])
                      }
                    }
                  ]
                }
              ]
            },
            {
              "value": "手机号",
              "body": [
                {
                  "id": 9,
                  "call": "ExcelReadSessionValueAtRow",
                  "args": {
                    "session_id": 1.session_id,
                    "sheet": input.sheet,
                    "row_number": var.item,
                    "column": "手机号"
                  }
                },
                {
                  "id": 10,
                  "call": "DebugPrintNode",
                  "args": {
                    "Value": 9.value
                  }
                }
              ]
            }
          ],
          "default": [
            {
              "id": 11,
              "call": "DebugPrintNode",
              "args": {
                "Value": {
                  "code": "unsupported_column",
                  "message": "仅支持姓名和手机号"
                }
              }
            }
          ]
        },
        {
          "id": 12,
          "set": "processed_count",
          "value": {
            "fn": "add",
            "args": [
              {
                "var": "processed_count"
              },
              1
            ]
          }
        }
      ]
    }
  ],
  "returns": {
    "session_id": {
      "value": 1.session_id,
      "disablename": "Excel 会话",
      "description": "本次打开的会话标识；后续操作需使用同一会话。"
    },
    "row_numbers": {
      "value": 2.row_numbers,
      "disablename": "待处理行号",
      "description": "查询命中的行号；无匹配时为空数组。"
    },
    "processed_count": {
      "value": var.processed_count,
      "disablename": "已遍历行数",
      "description": "本次遍历的行数；包含未匹配列名的情况。"
    }
  }
}
