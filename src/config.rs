//! 配置数据：Agent 列表与路径模板。
//!
//! 配置不再由插件自己存储，而是放在 IDE 原生「设置 › Quick Path」里（`plugin.toml` 的
//! `contributes.settings.pages`）。读取时直接向宿主要设置值；没配置过就回落到内置默认。

use openidea_plugin::host;
use serde::Deserialize;

/// Agent 数量上限，避免一张表无限膨胀。
pub const MAX_AGENTS: usize = 50;
/// 默认路径模板：Claude Code 的 `@路径#L行` 写法。
pub const DEFAULT_TEMPLATE: &str = "@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]";

/// 内置模板预设：(下拉框显示名, 模板内容)。
pub const PRESETS: &[(&str, &str)] = &[
    (
        "Claude Code",
        "@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]",
    ),
    ("Codex", "@{path}[ONE::{start}][MANY::{start}-{end}]"),
    ("纯路径", "{path}"),
];

/// 一个可启动的 Agent（`id` 只是运行时用它区分选项，不对外暴露）。
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub command: String,
    /// 是否为「默认启动」的那个；正常只有一项为 true。
    pub is_default: bool,
}

/// 整份配置：一份路径模板 + 一组 Agent。
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub path_template: String,
    pub agents: Vec<Agent>,
}

impl Config {
    /// 取默认 Agent：勾了「默认」的第一项；都没勾就退回第一项。
    pub fn default_agent(&self) -> Option<&Agent> {
        self.agents
            .iter()
            .find(|a| a.is_default)
            .or_else(|| self.agents.first())
    }
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

/// 内置默认 Agent：六个常见 CLI。
fn builtin_agents() -> Vec<Agent> {
    [
        ("Claude Code", "claude"),
        ("Codex", "codex"),
        ("OpenCode", "opencode"),
        ("Pi", "pi"),
        ("CodeBuddy", "codebuddy"),
        ("Qoder", "qoder"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (name, command))| Agent {
        id: i.to_string(),
        name: (*name).to_owned(),
        command: (*command).to_owned(),
        is_default: i == 0,
    })
    .collect()
}

impl Default for Config {
    /// 首次使用（设置里还没有值）时的回退值。
    fn default() -> Self {
        Self {
            path_template: DEFAULT_TEMPLATE.to_owned(),
            agents: builtin_agents(),
        }
    }
}

/// 从 IDE 设置读取配置；缺失时用内置默认值。
pub fn load() -> Config {
    // 设置里的 `agents` 是对象数组；解析失败或键不存在都回落到内置默认。
    let mut agents =
        match host::setting("agents").and_then(|s| serde_json::from_str::<Vec<Row>>(&s).ok()) {
            Some(rows) => rows
                .into_iter()
                .filter_map(|r| {
                    let name = r.name.trim();
                    let command = r.command.trim();
                    // 名称或命令为空的行直接忽略，避免启动空命令。
                    (!name.is_empty() && !command.is_empty()).then(|| Agent {
                        id: String::new(),
                        name: name.to_owned(),
                        command: command.to_owned(),
                        is_default: r.is_default,
                    })
                })
                .take(MAX_AGENTS)
                .enumerate()
                .map(|(i, mut a)| {
                    a.id = i.to_string();
                    a
                })
                .collect(),
            None => builtin_agents(),
        };
    // 「默认」只能有一个：都没勾就选第一项，勾了多个就只认第一个勾的。
    match agents.iter().position(|a| a.is_default) {
        Some(keep) => {
            for (i, a) in agents.iter_mut().enumerate() {
                a.is_default = i == keep;
            }
        }
        None => {
            if let Some(first) = agents.first_mut() {
                first.is_default = true;
            }
        }
    }
    // 以「自定义模板」文本框为准：填了就用它；留空才用上面的「模板预设」下拉。
    let path_template = match custom_template() {
        Some(t) => t,
        None => preset_template(),
    };
    Config {
        path_template,
        agents,
    }
}

/// 读「自定义模板」文本框；空字符串视为没填。
fn custom_template() -> Option<String> {
    host::setting_string("pathTemplate").filter(|s| !s.trim().is_empty())
}

/// 取下拉选中预设对应的模板；未知值回落到默认模板。
fn preset_template() -> String {
    let preset = host::setting_string("templatePreset").unwrap_or_else(|| PRESETS[0].0.to_owned());
    PRESETS
        .iter()
        .find(|(name, _)| *name == preset)
        .map(|(_, t)| (*t).to_owned())
        .unwrap_or_else(|| DEFAULT_TEMPLATE.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_six_agents_and_claude_template() {
        let cfg = Config::default();
        assert_eq!(cfg.agents.len(), 6);
        assert_eq!(cfg.path_template, DEFAULT_TEMPLATE);
        assert_eq!(cfg.agents[0].name, "Claude Code");
        assert_eq!(cfg.agents[0].command, "claude");
    }
}
