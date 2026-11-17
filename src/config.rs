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

/// 一个可启动的 Agent（`id` 只是运行时用它区分选项，不对外暴露）。
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub command: String,
}

/// 整份配置：一份路径模板 + 一组 Agent。
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub path_template: String,
    pub agents: Vec<Agent>,
}

/// 设置表格里的一行（只有名称与启动命令两列）。
#[derive(Deserialize)]
struct Row {
    #[serde(default)]
    name: String,
    #[serde(default)]
    command: String,
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
    let agents =
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
    let path_template = host::setting_string("pathTemplate")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TEMPLATE.to_owned());
    Config {
        path_template,
        agents,
    }
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
