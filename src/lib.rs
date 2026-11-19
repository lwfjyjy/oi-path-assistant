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
use template::{LineSel, render, selection_lines};

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

/// 插件对外显示的名字（通知标题）。
const PLUGIN_NAME: &str = "oi-path-assistant";

/// 统一的用户提示文案，避免多处重复。
const HINT_NO_TERMINAL: &str = "还没有启动过 Agent 终端，请先点「启动 Agent」。";
const HINT_TERMINAL_CLOSED: &str = "Agent 终端已关闭，请重新启动。";
const HINT_WINDOWS_PERMISSION: &str =
    "\nWindows 请在「设置 › 插件 › oi-path-assistant › 权限」里允许 PowerShell 或 cmd。";
const HINT_NO_SHELL: &str = "没有可用的终端 Shell";

/// 插件本体（无状态：配置每次从 IDE 设置读取）。
#[derive(Default)]
pub struct PathAssistant;

impl Plugin for PathAssistant {
    /// 激活时订阅终端退出事件，并按上次留下的终端 key 恢复「可插入」上下文。
    fn activate(&mut self, _ctx: &Context) -> Result<(), Error> {
        // 订阅失败也不该阻止插件加载，故忽略错误。
        let _ = host::subscribe(EventKind::TerminalExited);
        let _ = host::set_context(CTX_HAS_TERMINAL, last_terminal().is_some());
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
            "launch-agent" => self.launch_default(ctx),
            "pick-agent" => self.launch_pick(ctx),
            other => Err(Error::NotFound(format!("command {other}"))),
        }
    }

    /// 终端退出/关闭后，如果正好是最近那个，清掉记录并让「插入路径」重新隐藏。
    fn handle_events(&mut self, events: &[Event], _ctx: &Invocation) {
        for event in events {
            let Some(exited) = event.terminal_exited() else {
                continue;
            };
            if last_terminal().as_deref() == Some(exited.key.as_str()) {
                clear_last_terminal();
            }
        }
    }
}

impl PathAssistant {
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
            notify(NotificationLevel::Warning, HINT_NO_TERMINAL);
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
            notify(NotificationLevel::Warning, HINT_TERMINAL_CLOSED);
        }
        Ok(None)
    }

    /// 左键启动：直接启动「默认」Agent（设置里勾选的那个），不弹框。
    fn launch_default(&self, ctx: &Invocation) -> Result<Option<String>, Error> {
        match config::default_agent() {
            Some(agent) => self.start_or_warn(agent, ctx),
            None => {
                notify(NotificationLevel::Warning, HINT_NO_AGENTS);
                Ok(None)
            }
        }
    }

    /// 右键选择启动：弹出列表，选哪个启动哪个。
    fn launch_pick(&self, ctx: &Invocation) -> Result<Option<String>, Error> {
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
            Some(agent) => self.start_or_warn(agent, ctx),
            None => Ok(None),
        }
    }

    /// 启动失败（如项目未受信任或缺少 Shell 权限）给出可读提示，而不是把裸错误抛给界面。
    fn start_or_warn(&self, agent: Agent, ctx: &Invocation) -> Result<Option<String>, Error> {
        match self.start_agent(agent, ctx) {
            Ok(out) => Ok(out),
            Err(e) => {
                let hint = if is_windows(ctx) {
                    HINT_WINDOWS_PERMISSION
                } else {
                    ""
                };
                notify(NotificationLevel::Warning, &format!("启动失败：{e}{hint}"));
                Ok(None)
            }
        }
    }

    /// 在项目根起一个终端，把 Agent 命令交给对应平台的 Shell 执行。
    ///
    /// 关键点：不能让 Shell 在 Agent 退出后继续停在提示符上。否则之后「插入路径」
    /// 自动回车时，文件名里的 `$(...)`、反引号等会被残留的 shell 当成命令执行。
    /// POSIX 用 `exec` 让 Agent 替换 shell，Agent 退出终端即关闭；PowerShell 用
    /// `try/finally` 保证即使 Agent 抛出终止性异常也会关闭终端；cmd 用 `& exit`。
    fn start_agent(&self, agent: Agent, ctx: &Invocation) -> Result<Option<String>, Error> {
        let seq = host::storage::get(KEY_TERM_SEQ)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            + 1;
        let key = format!("agent-{seq}");
        let mut last_error = None;
        for shell in shell_candidates(ctx) {
            match host::open_terminal_session(&key, &agent.name, &shell.launch_command()) {
                Ok(_) => {
                    host::write_terminal(&key, &shell.line(&agent.command))?;
                    host::storage::set(KEY_TERM_SEQ, &seq.to_string())?;
                    host::storage::set(KEY_LAST_TERMINAL, &key)?;
                    let _ = host::set_context(CTX_HAS_TERMINAL, true);
                    return Ok(None);
                }
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error.unwrap_or_else(|| Error::Failed(HINT_NO_SHELL.into())))
    }

    /// 解析本次引用的目标：右键菜单带有目标路径 `arg` 时优先用它（左侧文件树等），
    /// 否则回退到「当前编辑器 + 选区」。
    fn resolve_ref(&self, ctx: &Invocation, arg: Option<&str>) -> Option<String> {
        match arg.map(str::trim).filter(|p| !p.is_empty()) {
            // 右键菜单没有行选区，只拼纯路径。
            Some(path) => self.ref_text(ctx, path, None),
            None => self.current_ref(ctx),
        }
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
        let sel = editor
            .selections()
            .ok()
            .and_then(|s| s.first().copied())
            .and_then(|r| {
                selection_lines(r.start.line, r.start.character, r.end.line, r.end.character)
            });
        self.ref_text(ctx, &path, sel)
    }

    /// 把「项目内绝对路径 + 选区」渲染成引用串；项目根或相对路径缺失时提示并返回 `None`。
    fn ref_text(&self, ctx: &Invocation, path: &str, sel: Option<LineSel>) -> Option<String> {
        let Some(project) = ctx.project() else {
            notify(
                NotificationLevel::Warning,
                "文件不在任何项目中，无法计算相对路径。",
            );
            return None;
        };
        let Ok(root) = project.root() else {
            notify(NotificationLevel::Warning, "无法读取项目根目录。");
            return None;
        };
        let Some(rel) = relative_path(&root, path) else {
            notify(
                NotificationLevel::Warning,
                "无法计算相对路径（文件不在项目内，或就是项目根目录）。",
            );
            return None;
        };
        self.check_text(render(&config::template(), &rel, sel))
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

/// 项目路径看起来是 Windows 路径（`C:\…` 或含反斜杠）。
fn is_windows(ctx: &Invocation) -> bool {
    let root = ctx
        .project()
        .and_then(|p| p.root().ok())
        .unwrap_or_default();
    root.contains('\\') || root.as_bytes().get(1) == Some(&b':')
}

/// 支持的终端 Shell。集中在这里，新增平台只改这一处。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalShell {
    Zsh,
    Bash,
    PowerShell,
    Pwsh,
    Cmd,
}

impl TerminalShell {
    /// 传给 host 的程序名。
    fn program(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::PowerShell => "powershell",
            Self::Pwsh => "pwsh",
            Self::Cmd => "cmd",
        }
    }

    /// 打开终端的命令；POSIX 登录 shell 需要 `-l` 才会加载用户的 PATH。
    fn launch_command(self) -> Command {
        let command = Command::new(self.program());
        if matches!(self, Self::Zsh | Self::Bash) {
            command.arg("-l")
        } else {
            command
        }
    }

    /// 启动语句：先清掉 IDE 的「命令运行中」标记，再执行 Agent，
    /// 并保证 Agent 退出后终端关闭（不残留可被自动输入的提示符）。
    fn line(self, command: &str) -> String {
        match self {
            Self::PowerShell | Self::Pwsh => format!(
                r#"[Console]::Write("$([char]27)]133;D$([char]27)\"); try {{ & {command} }} finally {{ exit }}
"#
            ),
            Self::Cmd => format!("{command} & exit\n"),
            Self::Zsh | Self::Bash => format!("{TERM_CMD_DONE}exec {command} || exit\n"),
        }
    }
}

/// 各平台的 Shell 候选：逐个尝试，避免某个 Shell 未安装时直接失败。
const UNIX_SHELLS: &[TerminalShell] = &[TerminalShell::Zsh, TerminalShell::Bash];
const WINDOWS_SHELLS: &[TerminalShell] = &[
    TerminalShell::PowerShell,
    TerminalShell::Pwsh,
    TerminalShell::Cmd,
];

fn shell_candidates(ctx: &Invocation) -> &'static [TerminalShell] {
    if is_windows(ctx) {
        WINDOWS_SHELLS
    } else {
        UNIX_SHELLS
    }
}

/// 取「最近一次启动的终端 key」；没有或为空时返回 `None`。
fn last_terminal() -> Option<String> {
    host::storage::get(KEY_LAST_TERMINAL)
        .ok()
        .flatten()
        .filter(|k| !k.is_empty())
}

/// 把绝对路径算成「以 / 分隔、无 ./、无前导 /」的项目相对路径；不在项目内返回 `None`。
///
/// 这里用字符串而非 `std::path`：插件编译成 wasm，`Path` 会按 unix 语义解析，
/// 在 Windows 的 `\` 路径上会失效；先把 `\` 统一成 `/` 即可跨平台正确。
/// 前缀比较忽略 ASCII 大小写（Windows 盘符/目录不区分大小写），但仍按原路径切片，
/// 因此输出保留用户路径的真实大小写。
fn relative_path(root: &str, path: &str) -> Option<String> {
    let root = root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return None;
    }
    let path = path.replace('\\', "/");
    let prefix = path.get(..root.len())?;
    // 只有 Windows 盘符路径忽略大小写；POSIX 路径保持大小写敏感。
    let same = if root.as_bytes().get(1) == Some(&b':') {
        prefix.eq_ignore_ascii_case(root)
    } else {
        prefix == root
    };
    if !same {
        return None;
    }
    let rel = path[root.len()..].strip_prefix('/')?;
    // 拒绝含 `..` 的路径，避免把项目外的文件伪装成项目相对路径。
    let parts: Vec<&str> = rel
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if parts.contains(&"..") {
        return None;
    }
    let rel = parts.join("/");
    (!rel.is_empty()).then_some(rel)
}

/// 终端关闭时清掉「最近终端」状态，并让插入按钮重新隐藏。
fn clear_last_terminal() {
    let _ = host::storage::delete(KEY_LAST_TERMINAL);
    let _ = host::set_context(CTX_HAS_TERMINAL, false);
}

/// 统一的用户提示。
fn notify(level: NotificationLevel, body: &str) {
    host::notify(level, PLUGIN_NAME, body);
}

register_plugin!(PathAssistant);

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
    fn mock() -> MockHost {
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
        let host = mock();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        PathAssistant.copy_path(&host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue#L5"));
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn copy_path_many_lines() {
        let host = mock();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 8, 0)]);
        PathAssistant.copy_path(&host.invocation(), None).unwrap();
        // 从第 5 行行首到第 9 行行首 = 5..8 行
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue#L5-L8"));
    }

    #[test]
    fn copy_path_no_selection_is_plain_path() {
        let host = mock();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[]);
        PathAssistant.copy_path(&host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue"));
    }

    #[test]
    fn copy_path_from_project_view_uses_target_path() {
        let host = mock();
        // 当前编辑器停在 A.vue，但在左侧文件树右键的是 B.vue：应以参数 B.vue 为准。
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        PathAssistant
            .copy_path(&host.invocation(), Some("/proj/src/B.vue"))
            .unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/B.vue"));
    }

    #[test]
    fn insert_path_from_project_view_uses_target_path() {
        let host = mock();
        single_agent(&host);
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        PathAssistant.launch_default(&host.invocation()).unwrap();
        PathAssistant
            .insert_path(&host.invocation(), Some("/proj/src/B.vue"))
            .unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/B.vue");
    }

    #[test]
    fn copy_path_codex_preset() {
        let host = mock();
        host.set_setting("templatePreset", &json!("Codex").to_string());
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 8, 0)]);
        PathAssistant.copy_path(&host.invocation(), None).unwrap();
        assert_eq!(host.clipboard().as_deref(), Some("@src/A.vue:5-8"));
    }

    #[test]
    fn copy_path_without_editor_notifies() {
        let host = mock();
        PathAssistant.copy_path(&host.invocation(), None).unwrap();
        assert!(host.clipboard().is_none());
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn copy_path_outside_project_warns() {
        let host = mock();
        // 编辑器打开了项目之外的文件：不能退化成去掉前导 / 的假相对路径。
        host.open_editor("/other/A.vue", "vue", VUE, &[]);
        PathAssistant.copy_path(&host.invocation(), None).unwrap();
        assert!(host.clipboard().is_none());
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.level == NotificationLevel::Warning)
        );
    }

    #[test]
    fn launch_single_agent_writes_command() {
        let host = mock();
        single_agent(&host);
        PathAssistant.launch_default(&host.invocation()).unwrap();
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
    fn launch_uses_powershell_on_windows_project() {
        let host = MockHost::install().with_capabilities(&[
            "editor.read",
            "clipboard",
            "storage",
            "project.read",
            "process:powershell",
        ]);
        // Windows 项目路径没有 zsh：自动改用 PowerShell，并把 Agent 命令交给它执行。
        host.set_project_root(Some(r"C:\proj"));
        host.set_setting(
            "agents",
            &json!([{ "name": "Claude Code", "command": "claude", "default": true }]).to_string(),
        );
        PathAssistant
            .run_command("launch-agent", &host.invocation(), None)
            .unwrap();
        let writes = host.terminal_writes();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].1.contains("[Console]::Write"));
        // 用 try/finally 包裹：Agent 抛终止性异常时也能关闭 Shell，避免残留提示符。
        assert!(writes[0].1.ends_with("try { & claude } finally { exit }\n"));
    }

    #[test]
    fn shell_lines_are_safe_and_platform_specific() {
        // POSIX：先清 OSC 标记，再用 exec 替换 shell，Agent 退出即关终端。
        let posix = TerminalShell::Zsh.line("claude");
        assert!(posix.contains("133;D"));
        assert!(posix.ends_with("exec claude || exit\n"));

        // PowerShell：用 try/finally 调用 Agent；即使抛终止性异常也会 exit 关闭终端。
        let powershell = TerminalShell::PowerShell.line("claude");
        assert!(powershell.starts_with("[Console]::Write("));
        assert!(powershell.ends_with("try { & claude } finally { exit }\n"));
        assert!(powershell.contains("finally { exit }"));

        // cmd：执行完立刻退出。
        assert_eq!(TerminalShell::Cmd.line("claude"), "claude & exit\n");

        // 平台不同，候选不同；且 POSIX 候选按 zsh → bash 顺序回退。
        let host = mock();
        assert_eq!(shell_candidates(&host.invocation()), UNIX_SHELLS);
    }

    #[test]
    fn powershell_line_always_closes_shell() {
        // 回归：PowerShell 必须用 try/finally 包住 Agent，终止性异常时也要 exit 关终端，
        // 否则残留的交互 Shell 会被后续「插入路径」的自动回车当成命令执行。
        let line = TerminalShell::Pwsh.line("agent.ps1");
        assert!(line.contains("try { & agent.ps1 } finally { exit }"));
        assert!(!line.contains("; exit\n"));
    }

    #[test]
    fn launch_falls_back_to_bash_when_zsh_missing() {
        // 只授予 bash 权限：zsh 打开失败后应自动尝试 bash，而不是直接报错。
        let host = MockHost::install().with_capabilities(&[
            "editor.read",
            "clipboard",
            "storage",
            "project.read",
            "process:bash",
        ]);
        host.set_project_root(Some("/proj"));
        single_agent(&host);
        PathAssistant.launch_default(&host.invocation()).unwrap();
        let writes = host.terminal_writes();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].1.ends_with("exec claude || exit\n"));
        assert_eq!(host.storage().get(KEY_LAST_TERMINAL), Some(&writes[0].0));
    }

    #[test]
    fn launch_pick_selects_agent() {
        let host = mock();
        // 默认六个 Agent，Codex 在下标 1；选择项 key 就是下标。
        host.answer_next_prompt(Some(&["1"]));
        PathAssistant.launch_pick(&host.invocation()).unwrap();
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
        let host = mock();
        // 把「默认」勾在第二项 Codex 上，左键启动应直接跑 codex。
        host.set_setting(
            "agents",
            &json!([
                { "name": "Claude Code", "command": "claude", "default": false },
                { "name": "Codex", "command": "codex", "default": true }
            ])
            .to_string(),
        );
        PathAssistant.launch_default(&host.invocation()).unwrap();
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
        let host = mock();
        host.set_setting("agents", "[]");
        PathAssistant.launch_default(&host.invocation()).unwrap();
        assert!(host.terminal_writes().is_empty());
        assert_eq!(host.notifications().len(), 1);
    }

    #[test]
    fn insert_writes_terminal_and_skips_clipboard() {
        let host = mock();
        single_agent(&host);
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        PathAssistant.launch_default(&host.invocation()).unwrap();
        PathAssistant.insert_path(&host.invocation(), None).unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/A.vue#L5");
        assert!(host.clipboard().is_none());
    }

    #[test]
    fn insert_auto_enter_appends_newline() {
        let host = mock();
        single_agent(&host);
        host.set_setting("autoEnter", "true");
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        PathAssistant.launch_default(&host.invocation()).unwrap();
        PathAssistant.insert_path(&host.invocation(), None).unwrap();
        assert_eq!(host.terminal_writes().last().unwrap().1, "@src/A.vue#L5\n");
    }

    #[test]
    fn insert_rejects_control_chars_in_path() {
        let host = mock();
        single_agent(&host);
        // 恶意文件名：内嵌换行可让终端把后半截当成独立命令提交（即使 autoEnter=false）。
        host.open_editor(
            "/proj/note\nprintf OI_PATH_CONTROL_PROBE\n#.rs",
            "rust",
            VUE,
            &[],
        );
        PathAssistant.launch_default(&host.invocation()).unwrap();
        let before = host.terminal_writes().len();
        PathAssistant.insert_path(&host.invocation(), None).unwrap();
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
        let host = mock();
        single_agent(&host);
        host.set_setting("autoEnter", "true");
        host.open_editor(
            "/proj/note$(printf OI_SHELL_SUBSTITUTION_PROBE).rs",
            "rust",
            VUE,
            &[],
        );
        let mut plugin = PathAssistant;
        plugin.launch_default(&host.invocation()).unwrap();
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
        let host = mock();
        host.open_editor("/proj/src/A.vue", "vue", VUE, &[(4, 0, 4, 3)]);
        PathAssistant.insert_path(&host.invocation(), None).unwrap();
        assert!(host.terminal_writes().is_empty());
        assert!(
            host.notifications()
                .iter()
                .any(|n| n.level == NotificationLevel::Warning)
        );
    }

    #[test]
    fn terminal_exit_hides_insert_context() {
        let host = mock();
        single_agent(&host);
        let mut plugin = PathAssistant;
        plugin.launch_default(&host.invocation()).unwrap();
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
        assert_eq!(
            relative_path("/proj", "/proj/src/A.vue").as_deref(),
            Some("src/A.vue")
        );
        // 根带尾斜杠也能正确相对化
        assert_eq!(
            relative_path("/proj/", "/proj/src/A.vue").as_deref(),
            Some("src/A.vue")
        );
        // 相同前缀但不是子目录，不能误配
        assert_eq!(relative_path("/proj", "/proj2/x"), None);
        // 项目根自身没有可引用的相对路径
        assert_eq!(relative_path("/proj", "/proj"), None);
        // 空根不能把任意绝对路径都当成相对路径
        assert_eq!(relative_path("", "/etc/passwd"), None);
        // 路径穿越必须拒绝
        assert_eq!(relative_path("/proj", "/proj/../etc/passwd"), None);
        // POSIX 路径区分大小写
        assert_eq!(relative_path("/proj", "/PROJ/x"), None);
        // Windows 风格反斜杠
        assert_eq!(
            relative_path(r"C:\proj", r"C:\proj\src\A.java").as_deref(),
            Some("src/A.java")
        );
        // Windows 路径大小写不敏感，但仍保留原路径大小写
        assert_eq!(
            relative_path(r"C:\Proj", r"c:\proj\src\A.java").as_deref(),
            Some("src/A.java")
        );
        // 折叠 ./ 与多余分隔符
        assert_eq!(
            relative_path("/proj", "/proj/./a//b.rs").as_deref(),
            Some("a/b.rs")
        );
    }
}
