//! Framework-level response guards.
//! These guards keep user-visible text consistent with the runtime action
//! state. Text without a subsequent tool call must not promise that an action
//! is about to run or claim that it is currently running.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedResponse {
    pub text: String,
    pub rewritten: bool,
}

pub const NO_ACTION_FALLBACK_ZH: &str = "我可以帮你执行这个操作。是否现在开始？";

pub fn guard_plain_response_without_tool_call(text: &str) -> GuardedResponse {
    if contains_strong_action_claim(text) {
        GuardedResponse {
            text: NO_ACTION_FALLBACK_ZH.to_string(),
            rewritten: true,
        }
    } else {
        GuardedResponse {
            text: text.to_string(),
            rewritten: false,
        }
    }
}

pub fn contains_strong_action_claim(text: &str) -> bool {
    let normalized = normalize_text(text);
    if normalized.is_empty() {
        return false;
    }

    contains_any(&normalized, STRONG_PROGRESS_CLAIMS)
        || (!contains_confirmation_question(&normalized)
            && (contains_any(&normalized, STRONG_TOOL_COMMITMENTS)
                || contains_immediate_action_claim(&normalized)))
}

pub fn contains_completion_claim(text: &str) -> bool {
    let normalized = normalize_text(text);
    contains_any(&normalized, COMPLETION_SUBJECTS) && contains_any(&normalized, COMPLETION_ACTIONS)
}

fn normalize_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

fn contains_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}

fn contains_confirmation_question(text: &str) -> bool {
    contains_any(text, CONFIRMATION_MARKERS)
        || (ends_with_question(text) && contains_any(text, CONFIRMATION_QUESTION_MARKERS))
}

fn contains_immediate_action_claim(text: &str) -> bool {
    contains_any(text, IMMEDIATE_ACTION_PREFIXES)
        && (contains_any(text, EXTERNAL_ACTION_VERBS) || contains_external_inspection_claim(text))
}

fn contains_external_inspection_claim(text: &str) -> bool {
    contains_any(text, INSPECTION_VERBS) && contains_any(text, EXTERNAL_OBJECT_MARKERS)
}

fn ends_with_question(text: &str) -> bool {
    text.ends_with('？') || text.ends_with('?') || text.ends_with('吗')
}

const STRONG_TOOL_COMMITMENTS: &[&str] = &[
    "我马上帮你调用",
    "马上帮你调用",
    "我现在帮你调用",
    "现在帮你调用",
    "我这就帮你调用",
    "这就帮你调用",
    "我马上调用",
    "马上调用工具",
    "我现在调用",
    "现在调用工具",
    "我这就调用",
    "这就调用工具",
    "我来调用工具",
    "我帮你调用工具",
    "我将调用工具",
    "我会调用工具",
    "我将帮你执行",
    "我会帮你执行",
    "我将为你执行",
    "我会为你执行",
    "我来帮你执行",
    "我来为你执行",
    "我马上执行",
    "我现在执行",
    "我这就执行",
    "马上帮你执行",
    "现在帮你执行",
    "这就帮你执行",
    "我马上帮你打开",
    "我现在帮你打开",
    "我这就帮你打开",
    "我马上帮你转换",
    "我现在帮你转换",
    "我这就帮你转换",
    "我马上帮你添加",
    "我现在帮你添加",
    "我马上帮你设置",
    "我现在帮你设置",
    "我马上帮你下载",
    "我现在帮你下载",
    "稍等我调用",
    "稍等我执行",
];

const STRONG_PROGRESS_CLAIMS: &[&str] = &[
    "正在调用工具",
    "正在执行工具",
    "正在帮你调用",
    "正在帮你执行",
    "正在帮你打开",
    "正在帮你转换",
    "正在帮你添加",
    "正在帮你设置",
    "正在帮你下载",
    "已开始调用",
    "已经开始调用",
    "已开始执行",
    "已经开始执行",
    "已向软件发送",
    "已经向软件发送",
];

const CONFIRMATION_MARKERS: &[&str] = &[
    "需要我",
    "是否",
    "要不要",
    "要我",
    "可以吗",
    "确定吗",
    "确认吗",
    "你确定",
    "请确认",
    "是否确认",
];

const CONFIRMATION_QUESTION_MARKERS: &[&str] = &[
    "要我",
    "需要我",
    "是否现在",
    "是否开始",
    "现在开始",
    "开始转换",
    "开始执行",
];

const IMMEDIATE_ACTION_PREFIXES: &[&str] = &[
    "我马上",
    "我现在",
    "我这就",
    "我先",
    "让我先",
    "接下来我会",
    "然后我马上",
    "然后我现在",
    "然后我会",
];

const EXTERNAL_ACTION_VERBS: &[&str] = &[
    "调用", "执行", "打开", "转换", "添加", "设置", "下载", "修改", "创建", "删除", "保存", "提交",
    "发送",
];

const INSPECTION_VERBS: &[&str] = &["查询", "检查", "读取", "查看"];

const EXTERNAL_OBJECT_MARKERS: &[&str] = &[
    "文件",
    "目录",
    "文件夹",
    "路径",
    "页面",
    "界面",
    "按钮",
    "工具",
    "接口",
    "系统",
    "软件",
    "日志",
    "配置",
    "数据库",
    "记录",
    "列表",
    "队列",
    "状态",
];

const COMPLETION_SUBJECTS: &[&str] = &[
    "我已经",
    "我已",
    "已为你",
    "已经为你",
    "帮你",
    "已向软件",
    "已经向软件",
];

const COMPLETION_ACTIONS: &[&str] = &[
    "调用了",
    "执行了",
    "打开了",
    "打开会员中心",
    "打开会员",
    "打开入口",
    "转换完成",
    "已转换",
    "转换好了",
    "添加完成",
    "已添加",
    "添加好了",
    "设置完成",
    "已设置",
    "设置好了",
    "下载完成",
    "已下载",
    "查询完成",
    "已查询",
    "读取完成",
    "已读取",
    "检查完成",
    "已检查",
    "分析完成",
    "已分析",
    "处理完成",
    "已处理",
    "处理好了",
    "发送请求",
    "发起了",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_strong_tool_commitment_without_action() {
        let guarded = guard_plain_response_without_tool_call("我马上帮你调用工具。");
        assert!(guarded.rewritten);
        assert_eq!(guarded.text, NO_ACTION_FALLBACK_ZH);
    }

    #[test]
    fn rewrites_progress_claim_without_action() {
        let guarded = guard_plain_response_without_tool_call("正在帮你转换，请稍等。");
        assert!(guarded.rewritten);
    }

    #[test]
    fn keeps_plain_reasoning_without_action() {
        for text in [
            "我现在帮你分析一下原因。",
            "稍等我梳理一下这个逻辑。",
            "我这就解释一下为什么会这样。",
            "我先看看这个推理哪里不对。",
            "让我先想一下这个问题。",
            "让我先解释一下这个现象。",
        ] {
            let guarded = guard_plain_response_without_tool_call(text);
            assert!(!guarded.rewritten, "{text}");
        }
    }

    #[test]
    fn rewrites_external_inspection_commitment_without_action() {
        for text in [
            "我现在帮你读取文件。",
            "我马上帮你检查页面状态。",
            "我这就查询待转列表。",
            "我先读取这个文件。",
            "让我先检查页面状态。",
            "让我先查看待转列表。",
        ] {
            let guarded = guard_plain_response_without_tool_call(text);
            assert!(guarded.rewritten, "{text}");
        }
    }

    #[test]
    fn keeps_completion_claim_for_confirmed_prior_results() {
        let guarded = guard_plain_response_without_tool_call("我已经为你打开会员中心。");
        assert!(!guarded.rewritten);
        assert!(contains_completion_claim("我已经为你打开会员中心。"));
    }

    #[test]
    fn keeps_capability_or_confirmation_question() {
        let guarded = guard_plain_response_without_tool_call("我可以帮你转换，是否现在开始？");
        assert!(!guarded.rewritten);
    }

    #[test]
    fn keeps_user_requested_immediate_action_question() {
        let guarded = guard_plain_response_without_tool_call("你要我现在开始转换吗？");
        assert!(!guarded.rewritten);
        assert!(!contains_strong_action_claim("你要我现在开始转换吗？"));
    }

    #[test]
    fn keeps_future_action_when_it_requests_confirmation() {
        let guarded = guard_plain_response_without_tool_call("我将为你执行转换，你确定吗？");
        assert!(!guarded.rewritten);
    }

    #[test]
    fn rewrites_future_action_without_confirmation() {
        let guarded = guard_plain_response_without_tool_call("我将为你执行转换。");
        assert!(guarded.rewritten);
    }

    #[test]
    fn rewrites_unbacked_follow_up_action() {
        let guarded = guard_plain_response_without_tool_call("然后我马上修改它。");
        assert!(guarded.rewritten);
    }
}
