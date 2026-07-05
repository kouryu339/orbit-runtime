from types import SimpleNamespace
import unittest

from corework_agent_tool import _request_args


class RequestArgsTests(unittest.TestCase):
    def test_prefers_structured_json_over_lossy_cli_reparse(self):
        request = SimpleNamespace(
            tool_name="ListDirectoryFiles",
            args_cli=r'--path "C:\workspace\samples\nnnc"',
            args_json=r'{"path":"C:\\Users\\buzim\\Desktop\\nnnc"}',
        )

        args = _request_args({"parameters": []}, request)

        self.assertEqual(args["path"], r"C:\Users\buzim\Desktop\nnnc")

    def test_falls_back_to_cli_for_legacy_clients(self):
        request = SimpleNamespace(
            tool_name="ListDirectoryFiles",
            args_cli=r'--path "C:\workspace\audio"',
            args_json="",
        )
        metadata = {
            "parameters": [
                {
                    "name": "path",
                    "required": True,
                    "default_value": None,
                }
            ]
        }

        args = _request_args(metadata, request)

        self.assertEqual(args["path"], r"C:\workspace\audio")

    def test_legacy_cli_preserves_tab_and_carriage_return_like_path_segments(self):
        request = SimpleNamespace(
            tool_name="ListDirectoryFiles",
            args_cli=(
                r'--path "C:\workspace\src-tauri\target\release\bundle\nsis"'
            ),
            args_json="",
        )
        metadata = {
            "parameters": [
                {
                    "name": "path",
                    "required": True,
                    "default_value": None,
                }
            ]
        }

        args = _request_args(metadata, request)

        self.assertEqual(
            args["path"],
            r"C:\workspace\src-tauri\target\release\bundle\nsis",
        )


if __name__ == "__main__":
    unittest.main()
