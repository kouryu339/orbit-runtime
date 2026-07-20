import assert from "node:assert/strict";
import test from "node:test";

import { requestArgs } from "../src/index.js";

test("prefers structured JSON over lossy CLI reparse", () => {
  const args = requestArgs(
    { parameters: [] },
    {
      tool_name: "ListDirectoryFiles",
      args_cli: String.raw`--path "C:\workspace\samples\nnnc"`,
      args_json: JSON.stringify({
        path: String.raw`C:\workspace\samples\nnnc`,
      }),
    },
  );

  assert.equal(args.path, String.raw`C:\workspace\samples\nnnc`);
});

test("falls back to CLI for legacy clients", () => {
  const args = requestArgs(
    {
      parameters: [
        {
          name: "path",
          required: true,
          default_value: null,
        },
      ],
    },
    {
      tool_name: "ListDirectoryFiles",
      args_cli: String.raw`--path "C:\workspace\audio"`,
      args_json: "",
    },
  );

  assert.equal(args.path, String.raw`C:\workspace\audio`);
});

test("legacy CLI preserves tab and carriage-return-like path segments", () => {
  const args = requestArgs(
    {
      parameters: [
        {
          name: "path",
          required: true,
          default_value: null,
        },
      ],
    },
    {
      tool_name: "ListDirectoryFiles",
      args_cli: String.raw`--path "C:\workspace\src-tauri\target\release\bundle\nsis"`,
      args_json: "",
    },
  );

  assert.equal(
    args.path,
    String.raw`C:\workspace\src-tauri\target\release\bundle\nsis`,
  );
});
