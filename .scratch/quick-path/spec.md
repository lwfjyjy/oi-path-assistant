# Quick Path 路径助手 — Spec

Status: ready-for-agent

## Problem Statement

在 OpenIDEA 里和 AI agent（Claude Code、Codex、OpenCode 等）对话时，要让 agent 精确知道"我说的是哪个文件的哪几行"，得手敲一段引用路径，例如 `@src/views/AAATemplate/AAATemplate.vue#L5-L9`。麻烦有三点：

- **格式各不相同**：不同 agent 支持的引用格式不一样（Claude Code 用 `#L5-L9`，Codex 用 `:5-9`），记不住、容易写错。
- **手拼容易错**：文件相对路径 + 行号，手动拼写慢且易出错，尤其行号。
- **拷贝/粘贴割裂**：拷到剪贴板还要切窗口粘贴；有时只想直接塞进正在跑的 agent 终端里。

## Solution

做一个 OpenIDEA 插件「Quick Path 路径助手」：

- **顶部工具栏** 3 个图标按钮：**启动 Agent**、**拷贝路径**、**插入路径**。
- **编辑器右键菜单**：**拷贝路径**、**插入路径**（与工具栏同逻辑）。
- **设置**：一个「路径助手」工具窗口（Web 面板）管理 Agent 列表和路径模板；一个原生设置页放 2 个轻量开关。
- 拷贝/插入时，按**路径模板**把当前打开文件（以及选中的行）拼成符合目标 agent 格式的字符串：**拷贝**写入剪贴板；**插入**不经过剪贴板，直接键入到"最近一次启动的 agent 终端"。

## User Stories

1. 作为 OpenIDEA 用户，我想在顶部工具栏看到「启动 Agent」「拷贝路径」「插入路径」三个图标按钮，这样不用记快捷键也能一键操作。
2. 作为用户，我想在编辑器右键菜单里也有「拷贝路径」「插入路径」，这样手不离鼠标就能用。
3. 作为用户，我只配置了一个 Agent 时，点「启动 Agent」就直接在终端把它启动起来，这样少一步选择。
4. 作为用户，我配置了多个 Agent 时，点「启动 Agent」会弹出可选列表让我挑一个再启动，这样我能随时切换不同 agent。
5. 作为用户，我还没配置任何 Agent 时点「启动 Agent」，希望得到友好提示（告诉我去设置里添加），这样不会以为坏了。
6. 作为用户，我点「启动 Agent」后，希望它在一个终端里真的把对应命令跑起来，这样我能立刻开始和 agent 对话。
7. 作为用户，我想在设置面板里新增一个 Agent，填写图标、名称、启动命令，这样能接入任意工具。
8. 作为用户，我想在设置面板里编辑/删除已有 Agent，这样配置能随需要调整。
9. 作为用户，我想拖动 Agent 列表调换顺序，这样常用的 agent 排在前面、下拉时更好找。
10. 作为用户，我想给 Agent 从内置的一组品牌 logo 里选图标，这样界面好看且一眼能认出是哪个工具。
11. 作为用户，我想能上传自己的 SVG 作为 Agent 图标，这样公司内部工具也能有专属图标。
12. 作为用户，我想在设置里编辑「路径模板」，用占位符拼出我要的格式，这样能适配任何 agent。
13. 作为用户，我想在路径模板旁有「快速写入」按钮，能一键选 Claude Code / Codex / 纯路径等预设，这样不用自己记模板语法。
14. 作为用户，我打开一个文件、**没有选中任何行**时点「拷贝路径」，只拷贝文件相对路径（不带行号），这样能引用整个文件。
15. 作为用户，我选中了**一行**时点「拷贝路径」，得到「路径 + 该行行号」的格式（如 `.../A.vue#L5`），这样能精确指向那一行。
16. 作为用户，我选中了**多行**时点「拷贝路径」，得到「路径 + 起止行号」的格式（如 `.../A.vue#L5-L9`），这样能指向一段代码。
17. 作为用户，我用的是 Codex（冒号格式）时，同一套操作应产出 `.../A.vue:5-9`，这样不用改插件就能适配不同 agent。
18. 作为用户，我拷贝成功后想看到一条「已复制」的提示（可关闭），这样知道操作生效了。
19. 作为用户，我点「插入路径」时，它把和拷贝完全一样的内容**直接写进最近启动的 agent 终端**、且**不经过剪贴板**，这样我能立刻让 agent 看到引用。
20. 作为用户，我还没启动过任何 agent 终端时，不希望看到「插入路径」按钮（因为它没地方可插），这样界面不会误导我。
21. 作为用户，我启动过一个 agent 终端之后，「插入路径」按钮应该出现，这样我知道现在可以插入了。
22. 作为用户，我插入的内容默认**不自动回车**执行，这样不会误触发 agent 发送；我可以在设置里打开"插入后自动回车"。
23. 作为用户，无论当前主题是亮色还是暗色，Agent 图标都要看得清，这样不会出现"图标消失"的尴尬。
24. 作为用户，我拷贝出来的路径要用**项目相对路径**、以 `/` 分隔、且不带 `./`，这样粘到任何 agent 里都能正确定位。
25. 作为用户，当我没有打开任何文件却触发拷贝/插入时，希望有明确提示而不是静默失败。

## Implementation Decisions

**模块划分**（全部为本插件首次新建，无既有代码可复用）：
- `config`：配置数据模型、预设列表、内置默认 Agent、读写（`host::storage_get/set` 存 JSON）。
- `template`：纯函数，把「模板 + 相对路径 + 起止行」渲染成最终字符串。
- `lib`（插件入口）：命令分发（`run_command`）、`terminal-exited` 事件处理、工具窗口 `Views` 消息处理。
- `settings.schema.json`：原生设置页 schema（2 个开关）。
- `config_page.html`：工具窗口面板（原生 JS，内嵌，无需 `web/` 目录）。

**配置数据模型**（storage 键 `config`，JSON）：
```json
{
  "version": 1,
  "pathTemplate": "@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]",
  "presetId": "claude",
  "agents": [
    { "id": "claude", "name": "Claude Code", "command": "claude",
      "icon": { "type": "builtin", "key": "claude" } }
  ]
}
```
- `icon`：`{type:"builtin",key}` 取自内置 logo 集；或 `{type:"svg",data}` 存用户上传的 SVG 文本。
- 内置默认 Agent：Claude Code(`claude`)、Codex(`codex`)、OpenCode(`opencode`)、Pi(`pi`)、CodeBuddy(`codebuddy`)、Qoder(`qoder`)。

**路径模板语法**（占位符 + 条件段）：
- `{path}`：项目相对路径（正斜杠、无 `./`）。
- `{start}` / `{end}`：1 基行号。
- `[ONE: ...]`：**恰好选中 1 行**时输出其中内容；否则整段消失。
- `[MANY: ...]`：**选中多行**时输出其中内容；否则整段消失。
- 未选中任何行时，`ONE` 与 `MANY` 段都不输出 → 只剩纯路径。
- 结束列为 0（整行选择常见）时结束行减 1，避免多算一行。
- 只取第一段选区（多光标忽略其余）。
- 内置预设：Claude Code `@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]`；Codex `@{path}[ONE::{start}][MANY::{start}-{end}]`；纯路径 `{path}`。

**Agent 启动方式**：平台无法对用户自定义的任意命令逐个授权（`process:<program>` 需固定程序名，无通配、无运行时批准）。因此统一做法是：用 `host::open_terminal_session` 在项目根起一个 **zsh 登录交互终端**（cwd=项目根，标签=Agent 名），再用 `host::write_terminal` 把该 Agent 的启动命令敲进去执行。限制：仅 zsh、依赖登录 shell 的 PATH、每插件最多 8 个终端、仅受信任项目可用。

**插入按钮动态显隐**：用 `host::set_context("hasTerminal", true/false)` 设置 `plugin.lwf.quick-path.hasTerminal` 上下文，菜单项 `when = "plugin.lwf.quick-path.hasTerminal"`，实现"启动过终端后才出现/退出后消失"。最近终端的 key 存进 storage，防止闲置回收后状态丢失；订阅 `terminal-exited` 事件更新该上下文。

**命令与贡献点**（`plugin.toml`）：
- commands：`launch-agent`、`copy-path`、`insert-path`、`open-config`（category `Quick Path`；前三个带 `icon=icons/*.svg`）。
- menus：`mainToolbar` 三条（`insert-path` 带 `when`）、`editor.context` 两条（`insert-path` 带 `when`）。
- views：`config`（location=`right`，代码渲染 `Body::Html`，内容来自内嵌的 `config_page.html`）。
- settings：`schema = "settings.schema.json"`，含 `autoEnter`（默认 false）、`notifyOnCopy`（默认 true）。

**权限（required）**：`editor.read`（读选区行号）、`clipboard`（写剪贴板）、`storage`（存配置）、`project.read`（取项目根算相对路径）、`process:zsh`（起终端）。全部为功能刚需；`process:zsh` 属高风险能力，仅在受信任项目可用。

**纯路径计算**：`editor.path()` 得绝对路径 → 相对 `project.root()` → 统一 `/`、去掉 `./`。不需要考虑"是否已保存"。

## Testing Decisions

- **好的测试只测外部行为**，不测内部实现细节：断言"输入什么状态、命令产出什么字符串 / 剪贴板里是什么 / 终端被写入了什么"。
- **接缝 1：`template` 渲染（纯函数）单元测试**。覆盖：无选中→纯路径；选中 1 行→`ONE` 段；选中多行→`MANY` 段；结束列=0 的减行；Claude 与 Codex 两种预设；空路径/空模板等边界。
- **接缝 2：命令层集成测试（SDK `MockHost`）**。用 `MockHost::install()` + `with_capabilities([...])` + `open_editor(path, lang, text, selections)` 模拟编辑器；跑 `copy-path` 断言 `host` 记录的剪贴板内容；跑 `insert-path` 断言终端写入内容；跑 `launch-agent` 断言命令被敲入终端。仿照上级 `AGENTS.md` 第 2.6 节的 `MockHost` 示例写法。
- 先例：上级 `AGENTS.md` 的 `MockHost` 单元测试示例即为本仓既有测试范式。

## Out of Scope

- **编辑器选区浮动工具栏**：平台没有该贡献点，用顶部工具栏 + 右键菜单替代。
- **自定义 Web 设置页替换原生设置**：SDK 不支持，只能用原生 schema 页。
- **插入到"指定某个终端"**：只插入到"最近一次启动"的终端；不做多终端选择。
- **非 zsh 的 shell**（bash/fish 等）：首版只支持 zsh。
- **在非受信任项目里启动终端**：受平台限制，不做。
- **读取剪贴板**：Rust 侧无此 API，且本功能不需要。

## Further Notes

- 3 个单色/透明 logo（Codex、Pi、OpenCode）在暗色主题下会看不清，渲染时给它们加**浅色圆角底板**，保证明暗都可见。
- 用户可上传 SVG 作图标；上传内容需做同样的净化校验（禁脚本/外部引用），并限制大小。
- 「插入后自动回车」默认关闭，避免误触发 agent 发送；仅当用户显式打开 `autoEnter` 才在插入内容后追加换行并回车。
