//! oi-path-assistant 路径助手：插件入口。
//!
//! 负责命令分发（启动 / 拷贝 / 插入）与 `terminal-exited` 事件处理。
//! 具体逻辑拆到两个模块：`config`（读取 IDE 设置）、`template`（纯函数渲染）。

mod config;
mod template;

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
/// 上下文键：当前是否有可写入的 Agent 终端（控制「插入路径」是否显示）。
const CTX_HAS_TERMINAL: &str = "hasTerminal";

/// 还没有配置任何 Agent 时的统一提示。
const HINT_NO_AGENTS: &str = "还没有配置任何 Agent。请在「设置 › oi-path-assistant」中添加。";

/// 引用串里混入换行等控制字符时的提示（终端会把换行当成「提交」）。
const HINT_CONTROL_CHARS: &str =
    "文件路径或模板里含换行、Tab 等控制字符，已取消（避免把文件名当成命令执行）。";

/// 让 IDE 不再把常驻终端当成「命令在跑」的 OSC 133;D 标记（shell 集成的「命令结束」）。
///
/// 插件开的常驻终端会被 IDE 当成「有命令一直在跑」而一直转圈；先吐这个标记后，
/// IDE 认为命令已结束就不再转圈——只影响那个转圈图标，终端照常可用。
const TERM_CMD_DONE: &str = r"printf '\033]133;D\033\\'; ";

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
        let _ = host::set_context(CTX_HAS_TERMINAL, has);
        Ok(())
    }

    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            "copy-path" => self.copy_path(ctx, arg),
            "insert-path" => self.insert_path(ctx, arg),
            "launch-agent" => self.launch_default(),
            "pick-agent" => self.launch_pick(),
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
    ///
    /// `arg` 是右键菜单传来的目标路径（如在左侧文件树右键）：有它时以该文件为准，
    /// 没有时回退到当前编辑器 + 选区。
    fn copy_path(&self, ctx: &Invocation, arg: Option<&str>) -> Result<Option<String>, Error> {
        let Some(text) = self.resolve_ref(ctx, arg) else {
            return Ok(None);
        };
        host::copy_to_clipboard(&text)?;
        if config::notify_on_copy() {
            notify(NotificationLevel::Info, &format!("已复制：{text}"));
        }
        Ok(None)
    }

    /// 插入：把同样内容直接写进最近一次启动的 Agent 终端（不经过剪贴板）。
    fn insert_path(&self, ctx: &Invocation, arg: Option<&str>) -> Result<Option<String>, Error> {
        let Some(text) = self.resolve_ref(ctx, arg) else {
            return Ok(None);
        };
        let Some(key) = last_terminal() else {
            notify(
                NotificationLevel::Warning,
                "还没有启动过 Agent 终端，请先点「启动 Agent」。",
            );
            return Ok(None);
        };
        let payload = if config::auto_enter() {
            format!("{text}\n")
        } else {
            text
        };
        if host::write_terminal(&key, &payload).is_err() {
            // 终端已被关闭：清理状态，让「插入路径」重新隐藏。
            clear_last_terminal();
            notify(NotificationLevel::Warning, "Agent 终端已关闭，请重新启动。");
        }
        Ok(None)
    }

    /// 左键启动：直接启动「默认」Agent（设置里勾选的那个），不弹框。
    fn launch_default(&self) -> Result<Option<String>, Error> {
        match config::default_agent() {
            Some(agent) => self.start_or_warn(agent),
            None => {
                notify(NotificationLevel::Warning, HINT_NO_AGENTS);
                Ok(None)
            }
        }
    }

    /// 右键选择启动：弹出列表，选哪个启动哪个。
    fn launch_pick(&self) -> Result<Option<String>, Error> {
        let agents = config::agents();
        if agents.is_empty() {
            notify(NotificationLevel::Warning, HINT_NO_AGENTS);
            return Ok(None);
        }
        // 选项 key 直接用下标，选完按同样下标取回 Agent。
        let items: Vec<PickItem> = agents
            .iter()
            .enumerate()
            .map(|(i, a)| PickItem::new(&i.to_string(), &a.name))
            .collect();
        let Some(index) = host::pick("选择一个 Agent 启动", &items)?.and_then(|k| k.parse().ok())
        else {
            return Ok(None);
        };
        match agents.into_iter().nth(index) {
            Some(agent) => self.start_or_warn(agent),
            None => Ok(None),
        }
    }

    /// 启动失败（如项目未受信任）给出可读提示，而不是把裸错误抛给界面。
    fn start_or_warn(&self, agent: Agent) -> Result<Option<String>, Error> {
        match self.start_agent(agent) {
            Ok(out) => Ok(out),
            Err(e) => {
                notify(NotificationLevel::Warning, &format!("启动失败：{e}"));
                Ok(None)
            }
        }
    }

    /// 在项目根起一个 zsh 登录终端，用 `exec` 把 Agent 命令替换掉这个 shell 再执行。
    ///
    /// 关键点：不能让登录 shell 在 Agent 退出后继续停在提示符上。否则之后「插入路径」
    /// 自动回车时，文件名里的 `$(...)`、反引号等会被残留的 shell 当成命令执行。
    /// 用 `exec` 后，shell 自身变成 Agent 进程：Agent 一旦退出终端即关闭，不存在
    /// 「Agent 已退出但 shell 还在」的中间态；`|| exit` 保证命令启动失败时直接关终端，
    /// 而不是退回到一个可被自动输入的 shell。
    fn start_agent(&self, agent: Agent) -> Result<Option<String>, Error> {
        let seq = host::storage::get(KEY_TERM_SEQ)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        let key = format!("agent-{seq}");
        host::open_terminal_session(&key, &agent.name, &Command::new("zsh").arg("-l"))?;
        host::write_terminal(
            &key,
            &format!("{TERM_CMD_DONE}exec {} || exit\n", agent.command),
        )?;
        host::storage::set(KEY_TERM_SEQ, &seq.to_string())?;
        host::storage::set(KEY_LAST_TERMINAL, &key)?;
        let _ = host::set_context(CTX_HAS_TERMINAL, true);
        Ok(None)
    }

    /// 解析本次引用的目标：右键菜单带有目标路径 `arg` 时优先用它（左侧文件树等），
    /// 否则回退到「当前编辑器 + 选区」。
    fn resolve_ref(&self, ctx: &Invocation, arg: Option<&str>) -> Option<String> {
        let Some(path) = arg.map(str::trim).filter(|p| !p.is_empty()) else {
            return self.current_ref(ctx);
        };
        let Some(project) = ctx.project() else {
            notify(
                NotificationLevel::Warning,
                "该文件不在任何项目中，无法计算相对路径。",
            );
            return None;
        };
        let Ok(root) = project.root() else {
            notify(NotificationLevel::Warning, "无法读取项目根目录。");
            return None;
        };
        // 右键菜单没有行选区，只拼纯路径。
        self.check_text(render(
            &config::template(),
            &relative_path(&root, path),
            None,
        ))
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
        let text = render(&config::template(), &relative_path(&root, &path), sel);
        self.check_text(text)
    }

    /// 终端输入必须只有一行：换行、Tab、CR 等控制字符都可能被终端当成「提交」或改变输入，
    /// 恶意文件名/模板借此能注入额外命令。这里宁可直接拒绝，也不静默改写路径。
    fn check_text(&self, text: String) -> Option<String> {
        if text.chars().any(char::is_control) {
            notify(NotificationLevel::Warning, HINT_CONTROL_CHARS);
            return None;
        }
        Some(text)
    }
}

/// 取「最近一次启动的终端 key」；没有或为空时返回 `None`。
fn last_terminal() -> Option<String> {
    host::storage::get(KEY_LAST_TERMINAL)
        .ok()
        .flatten()
        .filter(|k| !k.is_empty())
}

/// 把绝对路径算成「以 / 分隔、无 ./、无前导 /」的项目相对路径。
///
/// 这里用字符串而非 `std::path`：插件编译成 wasm，`Path` 会按 unix 语义解析，
/// 在 Windows 的 `\` 路径上会失效；先把 `\` 统一成 `/` 即可跨平台正确。
fn relative_path(root: &str, path: &str) -> String {
    let root = root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let path = path.replace('\\', "/");
    // 只有落在项目根「之下」（或正好等于根）才做相对化，避免 /proj 误配 /proj2。
    let rel = path
        .strip_prefix(root)
        .filter(|r| r.is_empty() || r.starts_with('/'))
        .unwrap_or(&path);
    rel.split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// 终端关闭时清掉「最近终端」状态，并让插入按钮重新隐藏。
fn clear_last_terminal() {
    let _ = host::storage::delete(KEY_LAST_TERMINAL);
    let _ = host::set_context(CTX_HAS_TERMINAL, false);
}

/// 统一的用户提示。
fn notify(level: NotificationLevel, body: &str) {
    host::notify(level, "oi-path-assistant", body);
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
            &json!([{ "name": "Claude Code", "command": "claude", "default": true }]).to_string(),
        );
        h.set_setting("pathTemplate", &json!(config::DEFAULT_TEMPLATE).to_string());
    }

    #[test]
    fn copy_path_single_line_to_clipboard() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.copy_path(&host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue#L5"));
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn copy_path_many_lines() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 8, 0)]);
        QuickPath.copy_path(&host.invocation(), None).unwrap();
        // 从第 5 行行首到第 9 行行首 = 5..8 行
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue#L5-L8"));
    }

    #[test]
    fn copy_path_no_selection_is_plain_path() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[]);
        QuickPath.copy_path(&host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue"));
    }

    #[test]
    fn copy_path_from_project_view_uses_target_path() {
        let host = host();
        // 当前编辑器停在 A.vue，但在左侧文件树右键的是 B.vue：应以参数 B.vue 为准。
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath
            .copy_path(&host.invocation(), Some("/proj/src/B.vue"))
            .unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/B.vue"));
    }

    #[test]
    fn insert_path_from_project_view_uses_target_path() {
        let host = host();
        single_agent(&host);
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.launch_default().unwrap();
        QuickPath
            .insert_path(&host.invocation(), Some("/proj/src/B.vue"))
            .unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/B.vue");
    }

    #[test]
    fn copy_path_codex_preset() {
        let host = host();
        host.set_setting("templatePreset", &json!("Codex").to_string());
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 8, 0)]);
        QuickPath.copy_path(&host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue:5-8"));
    }

    #[test]
    fn copy_path_without_editor_notifies() {
        let host = host();
        QuickPath.copy_path(&host.invocation(), None).unwrap();
        assert!(host.clipboard().is_none());
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn launch_single_agent_writes_command() {
        let host = host();
        single_agent(&host);
        QuickPath.launch_default().unwrap();
        let writes = host.terminal_writes();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].1.ends_with("exec claude || exit\n"));
        // 先吐 OSC 133;D，让 IDE 不再把常驻终端当成“命令运行中”而转圈
        assert!(writes[0].1.contains("133;D"));
        // 最近终端 key 已记录，供「插入路径」复用
        assert_eq!(host.storage().get(KEY_LAST_TERMINAL), Some(&writes[0].0));
        assert_eq!(
            host.context_keys().get(CTX_HAS_TERMINAL),
            Some(&true.into())
        );
    }

    #[test]
    fn launch_pick_selects_agent() {
        let host = host();
        // 默认六个 Agent，Codex 在下标 1；选择项 key 就是下标。
        host.answer_next_prompt(Some(&["1"]));
        QuickPath.launch_pick().unwrap();
        // 选中的是 Codex，验证敲进去的是 codex 命令
        assert!(
            host.terminal_writes()
                .last()
                .unwrap()
                .1
                .ends_with("exec codex || exit\n")
        );
    }

    #[test]
    fn launch_default_uses_marked_agent() {
        let host = host();
        // 把「默认」勾在第二项 Codex 上，左键启动应直接跑 codex。
        host.set_setting(
            "agents",
            &json!([
                { "name": "Claude Code", "command": "claude", "default": false },
                { "name": "Codex", "command": "codex", "default": true }
            ])
            .to_string(),
        );
        QuickPath.launch_default().unwrap();
        assert!(
            host.terminal_writes()
                .last()
                .unwrap()
                .1
                .ends_with("exec codex || exit\n")
        );
    }

    #[test]
    fn launch_without_agents_notifies_and_starts_nothing() {
        let host = host();
        host.set_setting("agents", "[]");
        QuickPath.launch_default().unwrap();
        assert!(host.terminal_writes().is_empty());
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn insert_writes_terminal_and_skips_clipboard() {
        let host = host();
        single_agent(&host);
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.launch_default().unwrap();
        QuickPath.insert_path(&host.invocation(), None).unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/A.vue#L5");
        assert!(host.clipboard().is_none());
    }

    #[test]
    fn insert_auto_enter_appends_newline() {
        let host = host();
        single_agent(&host);
        host.set_setting("autoEnter", "true");
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.launch_default().unwrap();
        QuickPath.insert_path(&host.invocation(), None).unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/A.vue#L5\n");
    }

    #[test]
    fn insert_rejects_control_chars_in_path() {
        let host = host();
        single_agent(&host);
        // 恶意文件名：内嵌换行可让终端把后半截当成独立命令提交（即使 autoEnter=false）。
        host.open_editor(
            "/proj/note\nprintf OI_PATH_CONTROL_PROBE\n#.rs",
            "rust",
            VUE,
            &[],
        );
        QuickPath.launch_default().unwrap();
        let before = host.terminal_writes().len();
        QuickPath.insert_path(&host.invocation(), None).unwrap();
        // 不向终端写入任何内容，并给出可读提示
        assert_eq!(host.terminal_writes().len(), before);
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.level == NotificationLevel::Warning && n.body == HINT_CONTROL_CHARS)
        );
    }

    /// 回归：Agent 退出后终端关闭，插入一个「不含控制字符、但含命令替换」的文件名
    /// 也不应被任何 shell 自动执行（旧实现会让残留的 zsh 提示符执行 `$(...)`）。
    #[test]
    fn insert_after_agent_exit_does_not_reach_shell() {
        let host = host();
        single_agent(&host);
        host.set_setting("autoEnter", "true");
        host.open_editor(
            "/proj/note$(printf OI_SHELL_SUBSTITUTION_PROBE).rs",
            "rust",
            VUE,
            &[],
        );
        let mut plugin = QuickPath;
        plugin.launch_default().unwrap();
        // Agent 退出 = 终端关闭（因为启动时用 exec 替换了 shell）
        let key = host.storage().get(KEY_LAST_TERMINAL).unwrap().clone();
        let event = host.finish_terminal(&key).unwrap();
        plugin.handle_events(&[event], &host.invocation());
        let before = host.terminal_writes().len();
        plugin.insert_path(&host.invocation(), None).unwrap();
        // 不向任何残留终端写入内容，并给出提示
        assert_eq!(host.terminal_writes().len(), before);
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.level == NotificationLevel::Warning)
        );
    }

    #[test]
    fn insert_without_terminal_notifies() {
        let host = host();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        QuickPath.insert_path(&host.invocation(), None).unwrap();
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
        plugin.launch_default().unwrap();
        let key = host.storage().get(KEY_LAST_TERMINAL).unwrap().clone();
        let event = host.finish_terminal(&key).unwrap();
        plugin.handle_events(&[event], &host.invocation());
        assert!(!host.storage().contains_key(KEY_LAST_TERMINAL));
        assert_eq!(
            host.context_keys().get(CTX_HAS_TERMINAL),
            Some(&false.into())
        );
    }

    #[test]
    fn relative_path_normalizes_separators() {
        assert_eq!(relative_path("/proj", "/proj/src/A.vue"), "src/A.vue");
        // 根带尾斜杠也能正确相对化
        assert_eq!(relative_path("/proj/", "/proj/src/A.vue"), "src/A.vue");
        // 相同前缀但不是子目录，不能误配
        assert_eq!(relative_path("/proj", "/proj2/x"), "proj2/x");
        // Windows 风格反斜杠
        assert_eq!(
            relative_path(r"C:\proj", r"C:\proj\src\A.java"),
            "src/A.java"
        );
        // 折叠 ./ 与多余分隔符
        assert_eq!(relative_path("/proj", "/proj/./a//b.rs"), "a/b.rs");
    }
}
