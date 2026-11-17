//! 配置数据：Agent 列表与路径模板。
//!
//! 配置由 IDE 原生「设置 › oi-path-assistant」提供（见 `plugin.toml` 的 `contributes.settings.pages`），
//! 插件只读取、不写入；读取失败或从未配置时回落到内置默认。

use openidea_plugin::host;
use serde::Deserialize;

/// Agent 数量上限，避免一张表无限膨胀。
pub const MAX_AGENTS: usize = 50;
/// 默认路径模板（等价于第一个预设 Claude Code）。
pub const DEFAULT_TEMPLATE: &str = "@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]";

/// 内置模板预设：(下拉框显示名, 模板内容)。
pub const PRESETS: &[(&str, &str)] = &[
    ("Claude Code", DEFAULT_TEMPLATE),
    ("Codex", "@{path}[ONE::{start}][MANY::{start}-{end}]"),
    ("纯路径", "{path}"),
];

/// 内置默认 Agent：(名称, 启动命令)。首个即「默认启动」。
const BUILTIN_AGENTS: &[(&str, &str)] = &[
    ("Claude Code", "claude"),
    ("Codex", "codex"),
    ("OpenCode", "opencode"),
    ("Pi", "pi"),
    ("CodeBuddy", "codebuddy"),
    ("Qoder", "qoder"),
];

/// 设置键名集中在此，避免散落在各处。
const KEY_AGENTS: &str = "agents";
const KEY_TEMPLATE: &str = "pathTemplate";
const KEY_PRESET: &str = "templatePreset";
const KEY_AUTO_ENTER: &str = "autoEnter";
const KEY_NOTIFY_ON_COPY: &str = "notifyOnCopy";

/// 一个可启动的 Agent。
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub name: String,
    pub command: String,
    /// 是否为「默认启动」的那个；正常只有一项为 true。
    pub is_default: bool,
}

/// 设置表格里的一行（名称 / 启动命令 / 是否默认）。
#[derive(Deserialize)]
struct Row {
    #[serde(default)]
    name: String,
    #[serde(default)]
    command: String,
    #[serde(rename = "default", default)]
    is_default: bool,
}

impl Row {
    /// 转成 Agent；名称或命令为空的行忽略，避免启动空命令。
    fn into_agent(self) -> Option<Agent> {
        let name = self.name.trim();
        let command = self.command.trim();
        (!name.is_empty() && !command.is_empty()).then(|| Agent {
            name: name.to_owned(),
            command: command.to_owned(),
            is_default: self.is_default,
        })
    }
}

/// 只取路径模板（渲染引用时用，省去解析 Agent 列表）。
pub fn template() -> String {
    read_template()
}

/// 全部 Agent；没有配置过就用内置列表。
pub fn agents() -> Vec<Agent> {
    read_agents()
}

/// 「默认」Agent：勾了的第一项；都没勾退回第一项；没有 Agent 时返回 `None`。
pub fn default_agent() -> Option<Agent> {
    pick_default(&read_agents()).cloned()
}

/// 从列表里挑默认项：勾了的第一项，没有就第一项。
fn pick_default(agents: &[Agent]) -> Option<&Agent> {
    agents
        .iter()
        .find(|a| a.is_default)
        .or_else(|| agents.first())
}

/// 插入路径后是否自动回车。
pub fn auto_enter() -> bool {
    host::setting_bool(KEY_AUTO_ENTER).unwrap_or(false)
}

/// 拷贝路径后是否弹提示。
pub fn notify_on_copy() -> bool {
    host::setting_bool(KEY_NOTIFY_ON_COPY).unwrap_or(true)
}

/// 读取 Agent 表格；没有设置过就用内置列表。
fn read_agents() -> Vec<Agent> {
    let mut agents = match read_rows() {
        Some(rows) => rows
            .into_iter()
            .filter_map(Row::into_agent)
            .take(MAX_AGENTS)
            .collect(),
        None => builtin_agents(),
    };
    ensure_single_default(&mut agents);
    agents
}

/// 把设置里的 `agents` 解析成行；键不存在或格式不对都返回 `None`。
fn read_rows() -> Option<Vec<Row>> {
    host::setting(KEY_AGENTS).and_then(|s| serde_json::from_str(&s).ok())
}

/// 「默认」只能有一个：都没勾就选第一项，勾了多个只认第一个勾的。
fn ensure_single_default(agents: &mut [Agent]) {
    let keep = agents.iter().position(|a| a.is_default).unwrap_or(0);
    for (i, a) in agents.iter_mut().enumerate() {
        a.is_default = i == keep;
    }
}

/// 内置 Agent 列表（首项默认）。
fn builtin_agents() -> Vec<Agent> {
    BUILTIN_AGENTS
        .iter()
        .enumerate()
        .map(|(i, (name, command))| Agent {
            name: (*name).to_owned(),
            command: (*command).to_owned(),
            is_default: i == 0,
        })
        .collect()
}

/// 以「自定义模板」为准：填了用它，留空才用上面的「模板预设」。
fn read_template() -> String {
    custom_template().unwrap_or_else(preset_template)
}

/// 读「自定义模板」文本框；空白字符串视为没填。
fn custom_template() -> Option<String> {
    host::setting_string(KEY_TEMPLATE).filter(|s| !s.trim().is_empty())
}

/// 取下拉选中预设对应的模板；未知值回落到默认模板。
fn preset_template() -> String {
    let name = host::setting_string(KEY_PRESET).unwrap_or_else(|| PRESETS[0].0.to_owned());
    PRESETS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or_else(|| DEFAULT_TEMPLATE.to_owned(), |(_, t)| (*t).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个只有名字的 Agent，默认为 false。
    fn agent(name: &str) -> Agent {
        Agent {
            name: name.to_owned(),
            command: name.to_lowercase(),
            is_default: false,
        }
    }

    #[test]
    fn builtin_agents_have_six_and_claude_first() {
        let agents = builtin_agents();
        assert_eq!(agents.len(), 6);
        assert_eq!(agents[0].name, "Claude Code");
        assert_eq!(agents[0].command, "claude");
        assert!(agents[0].is_default);
        // 默认模板必须与第一个预设保持一致，防止两处漂移。
        assert_eq!(PRESETS[0].1, DEFAULT_TEMPLATE);
    }

    #[test]
    fn pick_default_prefers_marked_then_first() {
        let mut agents = vec![agent("a"), agent("b")];
        agents[1].is_default = true;
        assert_eq!(pick_default(&agents).map(|a| a.name.as_str()), Some("b"));

        // 没有勾选时退回第一项
        assert_eq!(
            pick_default(&[agent("a")]).map(|a| a.name.as_str()),
            Some("a")
        );
        // 空列表没有默认项
        assert!(pick_default(&[]).is_none());
    }

    #[test]
    fn keep_only_first_checked_as_default() {
        let mut agents = vec![agent("a"), agent("b"), agent("c")];
        agents[1].is_default = true;
        agents[2].is_default = true;
        ensure_single_default(&mut agents);
        let flags: Vec<bool> = agents.iter().map(|a| a.is_default).collect();
        assert_eq!(flags, [false, true, false]);
    }

    #[test]
    fn no_checked_marks_first_as_default() {
        let mut agents = vec![agent("a"), agent("b")];
        ensure_single_default(&mut agents);
        assert!(agents[0].is_default);
        assert!(!agents[1].is_default);
    }

    #[test]
    fn settings_agents_default_matches_builtin() {
        // 设置页 JSON 的默认值与代码里的内置列表必须一致，避免改了代码忘了改 JSON。
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../settings-agents.json")).unwrap();
        let rows: Vec<Row> =
            serde_json::from_value(json["properties"]["agents"]["default"].clone()).unwrap();
        let expected = builtin_agents();
        assert_eq!(rows.len(), expected.len());
        for (row, want) in rows.iter().zip(&expected) {
            assert_eq!(row.name, want.name);
            assert_eq!(row.command, want.command);
            assert_eq!(row.is_default, want.is_default);
        }
    }

    #[test]
    fn settings_preset_enum_matches_code() {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../settings-template.json")).unwrap();
        let names: Vec<&str> = json["properties"]["templatePreset"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(names, PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>());
    }
}
