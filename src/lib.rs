//! Quick Path 路径助手：插件入口。
//!
//! 负责命令分发（启动 / 拷贝 / 插入）与 `terminal-exited` 事件处理。
//! 具体逻辑拆到两个模块：`config`（读取 IDE 设置）、`template`（纯函数渲染）。

mod config;
mod template;

use std::path::{Component, Path};

use openidea_plugin::host::{self, Command};
use openidea_plugin::{
    Context, Error, Event, EventKind, Invocation, NotificationLevel, PickItem, Plugin,
    register_plugin,
};

use config::Agent;
use template::{render, selection_lines};

/// storage 中记录「最近一次启动的终端 key」的键。
const KEY_LAST_TERMINAL: &str = "lastTerminalKey";
/// storage 中记录终端序号计数器的键（保证跨会话 key 不重复）。
const KEY_TERM_SEQ: &str = "terminalSeq";

/// 插件本体（无状态：配置每次从 IDE 设置读取）。
#[derive(Default)]
pub struct QuickPath;

impl Plugin for QuickPath {
    /// 激活时订阅终端退出事件，并按上次留下的终端 key 恢复「可插入」上下文。
    fn activate(&mut self, _ctx: &Context) -> Result<(), Error> {
        // 订阅失败也不该阻止插件加载，故忽略错误。
        let _ = host::subscribe(EventKind::TerminalExited);
        let has = host::storage::get(KEY_LAST_TERMINAL)
            .ok()
            .flatten()
            .is_some();
        let _ = host::set_context("hasTerminal", has);
        Ok(())
    }

    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        _arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            "copy-path" => self.copy_path(ctx),
            "insert-path" => self.insert_path(ctx),
            "launch-agent" => self.launch_agent(ctx),
            other => Err(Error::NotFound(format!("command {other}"))),
        }
    }

    /// 终端退出/关闭后，如果正好是最近那个，清掉记录并让「插入路径」重新隐藏。
    fn handle_events(&mut self, events: &[Event], _ctx: &Invocation) {
        for event in events {
            let Some(exited) = event.terminal_exited() else {
                continue;
            };
            let last = host::storage::get(KEY_LAST_TERMINAL).ok().flatten();
            if last.as_deref() == Some(exited.key.as_str()) {
                clear_last_terminal();
            }
        }
    }
}

impl QuickPath {
    /// 拷贝：把当前文件 + 选区的引用写入剪贴板。
    fn copy_path(&self, ctx: &Invocation) -> Result<Option<String>, Error> {
        let Some(text) = self.current_ref(ctx) else {
            return Ok(None);
        };
        host::copy_to_clipboard(&text)?;
        if host::setting_bool("notifyOnCopy").unwrap_or(true) {
            notify(NotificationLevel::Info, &format!("已复制：{text}"));
        }
        Ok(None)
    }

    /// 插入：把同样内容直接写进最近一次启动的 Agent 终端（不经过剪贴板）。
    fn insert_path(&self, ctx: &Invocation) -> Result<Option<String>, Error> {
        let Some(text) = self.current_ref(ctx) else {
            return Ok(None);
        };
        let Some(key) = host::storage::get(KEY_LAST_TERMINAL)
            .ok()
            .flatten()
            .filter(|k| !k.is_empty())
        else {
            notify(
                NotificationLevel::Warning,
                "还没有启动过 Agent 终端，请先点「启动 Agent」。",
            );
            return Ok(None);
        };
        let mut payload = text;
        if host::setting_bool("autoEnter").unwrap_or(false) {
            payload.push('\n');
        }
        if host::write_terminal(&key, &payload).is_err() {
            // 终端已被关闭：清理状态，让「插入路径」重新隐藏。
            clear_last_terminal();
            notify(NotificationLevel::Warning, "Agent 终端已关闭，请重新启动。");
        }
        Ok(None)
    }

    /// 启动：只有一个 Agent 时直接启动，多个时弹出选择，一个都没有时友好提示。
    fn launch_agent(&self, _ctx: &Invocation) -> Result<Option<String>, Error> {
        let cfg = config::load();
        if cfg.agents.is_empty() {
            notify(
                NotificationLevel::Warning,
                "还没有配置任何 Agent。请在「设置 › Quick Path」中添加。",
            );
            return Ok(None);
        }
        let agent = if cfg.agents.len() == 1 {
            cfg.agents[0].clone()
        } else {
            let items: Vec<PickItem> = cfg
                .agents
                .iter()
                .map(|a| PickItem::new(&a.id, &a.name))
                .collect();
            let Some(id) = host::pick("选择一个 Agent 启动", &items)? else {
                return Ok(None);
            };
            match cfg.agents.into_iter().find(|a| a.id == id) {
                Some(a) => a,
                None => return Ok(None),
            }
        };
        // 启动失败（如项目未受信任）给出可读提示，而不是把裸错误抛给界面。
        match self.start_agent(agent) {
            Ok(out) => Ok(out),
            Err(e) => {
                notify(NotificationLevel::Warning, &format!("启动失败：{e}"));
                Ok(None)
            }
        }
    }

    /// 在项目根起一个 zsh 登录终端，把 Agent 的启动命令敲进去执行。
    fn start_agent(&self, agent: Agent) -> Result<Option<String>, Error> {
        let seq = host::storage::get(KEY_TERM_SEQ)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        let key = format!("agent-{seq}");
        host::open_terminal_session(&key, &agent.name, &Command::new("zsh").arg("-l"))?;
        host::write_terminal(&key, &format!("{}\n", agent.command))?;
        host::storage::set(KEY_TERM_SEQ, &seq.to_string())?;
        host::storage::set(KEY_LAST_TERMINAL, &key)?;
        let _ = host::set_context("hasTerminal", true);
        Ok(None)
    }

    /// 算出「当前文件 + 选区」拼成的引用串；失败时已给出提示，返回 `None`。
    fn current_ref(&self, ctx: &Invocation) -> Option<String> {
        let Some(editor) = ctx.editor() else {
            notify(NotificationLevel::Warning, "请先打开一个文件。");
            return None;
        };
        let path = editor.path();
        if path.is_empty() {
            notify(NotificationLevel::Warning, "当前编辑器没有对应的文件。");
            return None;
        }
        let Some(project) = ctx.project() else {
            notify(
                NotificationLevel::Warning,
                "当前文件不在任何项目中，无法计算相对路径。",
            );
            return None;
        };
        let Ok(root) = project.root() else {
            notify(NotificationLevel::Warning, "无法读取项目根目录。");
            return None;
        };
        let sel = editor
            .selections()
            .ok()
            .and_then(|s| s.first().copied())
            .and_then(|r| {
                selection_lines(r.start.line, r.start.character, r.end.line, r.end.character)
            });
        let cfg = config::load();
        Some(render(
            &cfg.path_template,
            &relative_path(&root, &path),
            sel,
        ))
    }
}

/// 把 AI 相对路径算成「以 / 分隔、无 ./、无前导 /」的形式。
fn relative_path(root: &str, path: &str) -> String {
    let rel = Path::new(path)
        .strip_prefix(root)
        .unwrap_or(Path::new(path));
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// 终端关闭时清掉「最近终端」状态，并让插入按钮重新隐藏。
fn clear_last_terminal() {
    let _ = host::storage::delete(KEY_LAST_TERMINAL);
    let _ = host::set_context("hasTerminal", false);
}

/// 统一的用户提示。
fn notify(level: NotificationLevel, body: &str) {
    host::notify(level, "Quick Path", body);
}

register_plugin!(QuickPath);

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::mock::MockHost;
    use serde_json::json;

    const CAPS: &[&str] = &[
        "editor.read",
        "clipboard",
        "storage",
        "project.read",
        "process:zsh",
    ];
    const VUE: &str = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9";

    /// 安装带全套权限的 MockHost，并把项目根设为 /proj。
    fn host() -> MockHost {
        let host = MockHost::install().with_capabilities(CAPS);
        host.set_project_root(Some("/proj"));
        host
    }

    /// 只放一个 Claude Agent（直接启动、不弹选择）。
    fn single_agent(h: &MockHost) {
        h.set_setting(
            "agents",
            &json!([{ "name": "Claude Code", "command": "claude" }]).to_string(),
        );
        h.set_setting("pathTemplate", &json!(config::DEFAULT_TEMPLATE).to_string());
    }

    #[test]
    fn copy_path_single_line_to_clipboard() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.copy_path(&host.invocation()).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue#L5"));
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn copy_path_many_lines() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 8, 0)]);
        QuickPath.copy_path(&host.invocation()).unwrap();
        // 从第 5 行行首到第 9 行行首 = 5..8 行
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue#L5-L8"));
    }

    #[test]
    fn copy_path_no_selection_is_plain_path() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[]);
        QuickPath.copy_path(&host.invocation()).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue"));
    }

    #[test]
    fn copy_path_codex_preset() {
        let host = host();
        host.set_setting("templatePreset", &json!("Codex").to_string());
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 8, 0)]);
        QuickPath.copy_path(&host.invocation()).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue:5-8"));
    }

    #[test]
    fn copy_path_without_editor_notifies() {
        let host = host();
        QuickPath.copy_path(&host.invocation()).unwrap();
        assert!(host.clipboard().is_none());
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn launch_single_agent_writes_command() {
        let host = host();
        single_agent(&host);
        QuickPath.launch_agent(&host.invocation()).unwrap();
        let writes = host.terminal_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].1, "claude\n");
        // 最近终端 key 已记录，供「插入路径」复用
        assert_eq!(host.storage().get(KEY_LAST_TERMINAL), Some(&writes[0].0));
        assert_eq!(host.context_keys().get("hasTerminal"), Some(&true.into()));
    }

    #[test]
    fn launch_multiple_agents_picks_one() {
        let host = host();
        // 默认六个 Agent，Codex 在下标 1；选择项 key 就是下标。
        host.answer_next_prompt(Some(&["1"]));
        QuickPath.launch_agent(&host.invocation()).unwrap();
        // 选中的是 Codex，验证敲进去的是 codex 命令
        assert_eq!(host.terminal_writes().last().unwrap().1, "codex\n");
    }

    #[test]
    fn launch_without_agents_notifies_and_starts_nothing() {
        let host = host();
        host.set_setting("agents", "[]");
        QuickPath.launch_agent(&host.invocation()).unwrap();
        assert!(host.terminal_writes().is_empty());
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn insert_writes_terminal_and_skips_clipboard() {
        let host = host();
        single_agent(&host);
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.launch_agent(&host.invocation()).unwrap();
        QuickPath.insert_path(&host.invocation()).unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/A.vue#L5");
        assert!(host.clipboard().is_none());
    }

    #[test]
    fn insert_auto_enter_appends_newline() {
        let host = host();
        single_agent(&host);
        host.set_setting("autoEnter", "true");
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.launch_agent(&host.invocation()).unwrap();
        QuickPath.insert_path(&host.invocation()).unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/A.vue#L5\n");
    }

    #[test]
    fn insert_without_terminal_notifies() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.insert_path(&host.invocation()).unwrap();
        assert!(host.terminal_writes().is_empty());
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.level == NotificationLevel::Warning)
        );
    }

    #[test]
    fn terminal_exit_hides_insert_context() {
        let host = host();
        single_agent(&host);
        let mut plugin = QuickPath;
        plugin.launch_agent(&host.invocation()).unwrap();
        let key = host.storage().get(KEY_LAST_TERMINAL).unwrap().clone();
        let event = host.finish_terminal(&key).unwrap();
        plugin.handle_events(&[event], &host.invocation());
        assert!(!host.storage().contains_key(KEY_LAST_TERMINAL));
        assert_eq!(host.context_keys().get("hasTerminal"), Some(&false.into()));
    }
}
