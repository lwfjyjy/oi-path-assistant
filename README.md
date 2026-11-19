# oi-path-assistant 路径助手

在 OpenIDEA 里和 AI Agent（Claude Code、Codex、OpenCode 等）对话时，一键把「当前文件的相对路径 + 选中行号」拼成对应 Agent 的引用格式，**拷贝到剪贴板**或**直接敲进正在运行的 Agent 终端**，再也不用记 `#L5-L9` / `:5-9` 这些格式。

## 功能

- **顶部工具栏 3 个图标**：启动 Agent、拷贝路径、插入路径。
- **编辑器右键菜单**：选择 Agent 启动…（弹出列表选一个）、拷贝路径、插入路径。
- **左侧文件树右键菜单**：拷贝路径、插入路径、选择 Agent 启动…（对该文件生效，不依赖当前打开的文件）。
- **路径模板**：`{path}` 相对路径、`{start}`/`{end}` 行号、`[ONE: …]` 单行段、`[MANY: …]` 多行段；文档里给了 Claude Code / Codex / 纯路径三种现成模板。
- **原生设置页管理 Agent**（设置 › oi-path-assistant）：分「Agent 列表」和「路径模板」两页；Agent 表格可增删、排序、编辑名称与启动命令，并用「默认」列勾选左键直接启动的那个，内置 6 个常见 Agent。
- **插入路径**不经过剪贴板，直接写进「最近一次启动」的 Agent 终端；默认不自动回车，可在设置里开启。
- 未启动过 Agent 终端时，自动隐藏「插入路径」按钮（通过 `when` 上下文控制）。

## 使用

1. 打开任意项目文件。
2. 左键点「启动 Agent」：直接启动设置里勾了「默认」的那个；想临时换一个，就在编辑器里右键 →「选择 Agent 启动…」弹列表选；一个都没配置会提示去设置里添加。
3. 选中若干行（或什么都不选），点「拷贝路径」把引用拷到剪贴板，或点「插入路径」直接塞进 Agent 终端。
4. 打开 **设置 › oi-path-assistant**：「Agent 列表」页管理 Agent 并勾选默认；「路径模板」页改模板、开关自动回车与拷贝提示。

## 路径模板语法

「自定义模板」文本框填了就以它为准（覆盖预设）；留空时用「模板预设」下拉（Claude Code / Codex / 纯路径）。

| 占位 / 段 | 含义 |
| --- | --- |
| `{path}` | 项目相对路径，统一 `/` 分隔、不带 `./`（Windows 的 `\` 会自动归一为 `/`） |
| `{start}` / `{end}` | 1 基行号 |
| `[ONE: …]` | 恰好选中 1 行时输出，否则整段消失 |
| `[MANY: …]` | 选中多行时输出，否则整段消失 |

不选任何行时，`ONE` / `MANY` 段都不输出，只剩纯路径。结束列为 0（整行选择）时结束行自动减 1，避免多算一行。

常见模板（可直接填进设置页的「自定义模板」）：

- Claude Code：`@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]`
- Codex：`@{path}[ONE::{start}][MANY::{start}-{end}]`
- 纯路径：`{path}`

## 所需权限（required）

| 能力 | 用途 |
| --- | --- |
| `editor.read` | 读取当前文件与选区行号 |
| `clipboard` | 把引用写入剪贴板 |
| `storage` | 保存运行状态（最近一次启动的终端 key） |
| `project.read` | 读取项目根目录，计算相对路径 |
| `process:zsh` | 在项目根启动 zsh 终端并执行 Agent 命令（高风险，仅受信任项目可用） |

> 「启动 Agent」的做法是：在项目根开一个 **zsh 登录终端**，再用 `exec` 把 Agent 的启动命令**替换掉这个 shell** 执行（平台无法对用户自定义的任意命令逐个授权，只能固定用 `zsh`）。因此仅支持 zsh、依赖登录 shell 的 PATH，每个插件最多 8 个终端。
>
> ⚠️ 因为用了 `exec`，Agent 退出后终端会**随之关闭**，不会留下一个可以被自动输入的 shell 提示符。启动命令请填**单个可执行命令**（可带参数，如 `claude --model opus`），不要写 `a && b`、管道等 shell 组合语句。

## 构建

```bash
openidea-plugin build .      # 编译 wasm 并校验清单，产物在 dist/plugin.wasm
```

首次构建会从 GitHub 拉取 `openidea-plugin` SDK（tag `v0.1.45`）。

发布取证的固定输入（可复现构建）：

| 项 | 值 |
| --- | --- |
| Rust | `1.100.0`（CI 用 `dtolnay/rust-toolchain@1.100.0` 固定） |
| 依赖锁 | `Cargo.lock`（已提交，SDK 精确到 commit `c5be2998ec7190567c9da8a6f375aeda4becad2e` = tag `v0.1.45`） |
| 命令 | `openidea-plugin build .`（内部 `cargo build --release --target wasm32-wasip2`） |
| 构建环境 | 官方 CLI 0.1.45，`CARGO_HOME` 任意；CI 见 `.github/workflows/build.yml` |

<details>
<summary>离线开发（内网 / 无网络时）</summary>

把 `Cargo.toml` 里的依赖临时换成本地 SDK 副本，并用工作区内的 `CARGO_HOME` 构建：

```toml
openidea-plugin = { path = "../.agent-cache/openidea-src/sdk/rust/openidea-plugin" }
```

```bash
export CARGO_HOME=$PWD/../.agent-cache/cargo-home
export CARGO_NET_OFFLINE=true
cargo test --offline
openidea-plugin build .
```

发布 / 提交前记得把依赖改回上面的 git 写法。
</details>

## 目录结构

```
oi-path-assistant/
├─ plugin.toml            # 插件清单（命令 / 菜单 / 设置页 / 权限）
├─ settings-agents.json   # 设置页「Agent 列表」的结构与默认值
├─ settings-template.json # 设置页「路径模板」的结构与默认值
├─ icons/                 # 4 个命令图标（启动 / 拷贝 / 插入 / 选择）
├─ src/
│  ├─ lib.rs              # 插件入口：命令分发、终端事件、路径换算
│  ├─ config.rs           # 从 IDE 设置读取 Agent 与模板（含内置默认）
│  └─ template.rs         # 路径模板纯函数渲染
└─ .scratch/oi-path-assistant/spec.md  # 需求规格
```
