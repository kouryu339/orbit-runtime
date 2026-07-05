/**
 * 节点图标映射 — 按节点类型 / 节点种类返回 lucide 图标。
 *
 * 优先级：
 *   1. 按 FlowchartNode.kind（注册表节点类型，如 OpenBrowserNode）匹配特定图标
 *   2. 退化到按 node_type（action/decision/loop/...）匹配通用图标
 *
 * 普通用户的视觉锚点：图标比文字快 2-3 倍
 */
import React from 'react';
import {
  Play, StopCircle, Box, GitBranch, Repeat, ArrowRightCircle, Variable, GitMerge,
  Globe, MousePointer2, Type, Eye, Download, Upload,
  FileText, Folder, FileSpreadsheet, FileImage, Trash2, Copy,
  Cpu, Brain, Sparkles,
  Wifi, Database, Send,
  Plus, Minus, X, Divide, Hash, ToggleLeft,
  Filter, ListOrdered, ArrowUpDown,
  Clock, Calendar,
  type LucideIcon,
} from 'lucide-react';

import type { FlowchartNodeType } from '../types';

/** 通用类型 → 图标 */
const TYPE_ICONS: Record<FlowchartNodeType, LucideIcon> = {
  start:       Play,
  end:         StopCircle,
  action:      Box,
  decision:    GitBranch,
  loop:        Repeat,
  break:       ArrowRightCircle,
  variable:    Variable,
  sub_process: GitMerge,
};

/**
 * 注册表节点种类 → 图标（按前缀匹配，覆盖最常用的几类）
 *
 * 不需要每个节点都列 — 没匹配上就走 TYPE_ICONS 的兜底图标。
 */
const KIND_ICONS: Array<[RegExp, LucideIcon]> = [
  // ── 浏览器系列 ──
  [/^OpenBrowser/i,  Globe],
  [/Browser/i,       Globe],
  [/^Navigate/i,     Globe],
  [/^ReloadPage/i,   Globe],
  [/^ClosePage/i,    Globe],
  [/^Click/i,        MousePointer2],
  [/^Hover/i,        MousePointer2],
  [/^FillInput/i,    Type],
  [/^TypeInput/i,    Type],
  [/^GetText/i,      Eye],
  [/^GetCurrentUrl/i,Eye],
  [/^WaitForElement/i,Clock],
  [/^UploadFile/i,   Upload],
  [/^Download/i,     Download],

  // ── 文件系列 ──
  [/^CreateFolder/i, Folder],
  [/Folder/i,        Folder],
  [/^Excel|Spreadsheet/i, FileSpreadsheet],
  [/^Image|Png|Jpg/i,FileImage],
  [/^Delete|Remove(?!Pin)/i, Trash2],
  [/^Copy(?!Workflow)/i, Copy],
  [/File|Path|Rename/i,FileText],

  // ── AI / LLM ──
  [/^(Llm|Ai|Chat|Prompt|Embed)/i, Sparkles],
  [/Brain/i,         Brain],
  [/Cpu|Compute/i,   Cpu],

  // ── HTTP / 网络 ──
  [/Http|Request|Fetch|Api/i, Wifi],
  [/Database|Sql|Db/i, Database],
  [/Send|Post|Email/i, Send],

  // ── 数学 ──
  [/^Add/i,          Plus],
  [/^Subtract/i,     Minus],
  [/^Multiply/i,     X],
  [/^Divide/i,       Divide],
  [/^Modulo/i,       Hash],
  [/^Compare|Equal|Greater|Less/i, ToggleLeft],

  // ── 集合 ──
  [/^Filter/i,       Filter],
  [/^Sort/i,         ArrowUpDown],
  [/^Length|Count/i, ListOrdered],

  // ── 时间 ──
  [/^DateTime|Date|Time(?!out)/i, Calendar],
];

export function getNodeIcon(nodeType: FlowchartNodeType, kind?: string): LucideIcon {
  if (kind) {
    for (const [re, icon] of KIND_ICONS) {
      if (re.test(kind)) return icon;
    }
  }
  return TYPE_ICONS[nodeType] ?? Box;
}
