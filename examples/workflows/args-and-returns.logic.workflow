{
  "schema": "orbit.workflow.logic/v1",
  "inputs": {
    "names": {
      "type": "String",
      "disablename": "姓名列表",
      "description": "待拆分的姓名；支持中英文逗号。"
    }
  },
  "variables": {
    "count": {
      "type": "i64",
      "initial": 0,
      "disablename": "计数"
    }
  },
  "steps": [
    {
      "id": 1,
      "call": "StringAppendNode",
      "args": {
        "A": input.names,
        "B": ""
      }
    },
    {
      "id": 2,
      "call": "DebugPrintNode",
      "args": {
        "Value": {
          "names": split(1.Joined, ["，", ","]),
          "next": add(var.count, 1),
          "text": "add(var.count, 1)",
          "items": [input.names, "input.names", 123, null]
        }
      }
    }
  ],
  "returns": {
    "result": {
      "value": {
        "names": split(1.Joined, ["，", ","]),
        "next": add(var.count, 1),
        "first": item(split(input.names, ["，", ","]), 0),
        "text": "1.Joined"
      },
      "disablename": "处理结果",
      "description": "返回拆分列表和下次计数；不修改计数变量。"
    }
  }
}
