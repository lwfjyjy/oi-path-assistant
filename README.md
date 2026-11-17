# Quick Path 路径助手

在 OpenIDEA 里和 AI Agent（Claude Code、Codex、OpenCode 等）对话时，一键把「当前文件的相对路径 + 选中行号」拼成对应 Agent 的引用格式，**拷贝到剪贴板**或**直接敲进正在运行的 Agent 终端**，再也不用记 `#L5-L9` / `:5-9` 这些格式。

## 功能

- **顶部工具栏 3 个图标**：启动 Agent、拷贝路径、插入路径。
- **编辑器右键菜单**：拷贝路径、插入路径（与工具栏同逻辑）。
- **路径模板**：`{path}` 相对路径、`{start}`/`{end}` 行号、`[ONE: …]` 单行段、`[MANY: …]` 多行段；文档里给了 Claude Code / Codex / 纯路径三种现成模板。
- **原生设置页管理 Agent**（设置 › Quick Path）：在设置页的表格里增删、排序、直接编辑 Agent 的名称与启动命令，内置 6 个常见 Agent。
- **插入路径**不经过剪贴板，直接写进「最近一次启动」的 Agent 终端；默认不自动回车，可在设置里开启。
- 未启动过 Agent 终端时，自动隐藏「插入路径」按钮（通过 `when` 上下文控制）。

## 使用

1. 打开任意项目文件。
2. 点「启动 Agent」（只配置了一个就直接启动；多个会弹列表让你选；一个都没有会提示去设置里添加）。
3. 选中若干行（或什么都不选），点「拷贝路径」把引用拷到剪贴板，或点「插入路径」直接塞进 Agent 终端。
4. 打开 **设置 › Quick Path**：在「Agent 与路径模板」页里增删 / 排序 Agent、修改路径模板、开关自动回车与拷贝提示。

## 路径模板语法

| 占位 / 段 | 含义 |
| --- | --- |
| `{path}` | 项目相对路径，统一 `/` 分隔、不带 `./` |
| `{start}` / `{end}` | 1 基行号 |
| `[ONE: …]` | 恰好选中 1 行时输出，否则整段消失 |
| `[MANY: …]` | 选中多行时输出，否则整段消失 |

不选任何行时，`ONE` / `MANY` 段都不输出，只剩纯路径。结束列为 0（整行选择）时结束行自动减 1，避免多算一行。

常见模板（可直接填进设置页的「路径模板」）：

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

> 「启动 Agent」的做法是：在项目根开一个 **zsh 登录终端**，再把 Agent 的启动命令敲进去执行（平台无法对用户自定义的任意命令逐个授权，只能固定用 `zsh`）。因此仅支持 zsh、依赖登录 shell 的 PATH，每个插件最多 8 个终端。

## 构建

```bash
openidea-plugin build .      # 编译 wasm 并校验清单，产物在 dist/plugin.wasm
```

首次构建会从 GitHub 拉取 `openidea-plugin` SDK（tag `v0.1.45`）。

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
quick-path/
├─ plugin.toml            # 插件清单（命令 / 菜单 / 设置 / 权限）
├─ settings.json          # 原生设置页：Agent 表格 / 路径模板 / 开关
├─ icons/                 # 3 个工具栏命令图标
├─ src/
│  ├─ lib.rs              # 插件入口：命令分发、终端事件
│  ├─ config.rs           # 从 IDE 设置读取 Agent 与模板
│  └─ template.rs         # 路径模板纯函数渲染
└─ .scratch/quick-path/spec.md  # 需求规格
```
