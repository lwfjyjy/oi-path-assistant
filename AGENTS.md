# oi-path-assistant 路径助手

> OpenIDEA 插件（Rust + `openidea-plugin` SDK）。通用开发规范见上级目录 `../AGENTS.md`。

## Agent skills

### Issue tracker

本机 Markdown，spec 与 issue 存放在 `.scratch/<feature>/`。见 `docs/agents/issue-tracker.md`。

### Triage labels

默认 5 个标签：needs-triage / needs-info / ready-for-agent / ready-for-human / wontfix。见 `docs/agents/triage-labels.md`。

### Domain docs

单上下文（single-context）：根目录 `GLOSSARY.md` + `docs/adr/`。见 `docs/agents/domain.md`。

## 发布与更新流程

> 目标：把新版本从代码送到插件市场。除「创建 Release 并上传安装包」必须用 Playwright（浏览器需已登录 GitHub）外，其余可用 GitHub MCP + 本地 git 完成。全程优先用工具，不要手工点网页（Release 上传除外）。

### 1. 升级版本号

同步修改 `Cargo.toml` 与 `plugin.toml` 的 `version`（语义化版本，如 `0.1.3`）。

### 2. 本地构建与验证（沙箱/离线环境）

```bash
# 仅本地开发/构建时，临时把 Cargo.toml 依赖改为本地 SDK
# openidea-plugin = { path = "../.agent-cache/openidea-src/sdk/rust/openidea-plugin" }
export CARGO_HOME="$PWD/../.agent-cache/cargo-home"
export CARGO_NET_OFFLINE=true
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
cargo fmt && cargo test --offline && cargo clippy --all-targets --offline
openidea-plugin build .
```

- 提交前必须把依赖改回：`openidea-plugin = { git = "https://github.com/l0ng-ai/openidea", tag = "v0.1.45" }`。
- `Cargo.lock` 必须锁定该 tag 对应的 commit，且要提交；不要提交本地 path 依赖。

### 3. 签名打包

```bash
openidea-plugin pack . --key ../.agent-cache/keys/lwf.key --publisher lwf \
  --out ../.agent-cache/release/lwf.oi-path-assistant-<version>.oiplugin
shasum -a 256 ../.agent-cache/release/lwf.oi-path-assistant-<version>.oiplugin
```

- 私钥 `../.agent-cache/keys/lwf.key` **绝不提交、绝不外传**；丢了无法再以 `lwf` 身份发版。
- `pack` 不会自动用私钥，必须显式 `--key`。

### 4. 提交并推送代码/tag

```bash
git add -A && git commit -m "release: v<version>"
git tag v<version>
git push origin main && git push origin v<version>
```

### 5. 创建 GitHub Release 并上传安装包（唯一必须 Playwright 的步骤）

1. 打开 `https://github.com/lwfjyjy/oi-path-assistant/releases/new?tag=v<version>`。
2. 标题填 `v<version>`；正文写本次更新说明 + wasm 与安装包 sha256。
3. `Attach binaries` 选择上一步的 `.oiplugin`，等上传完成，点 **Publish release**。
4. 用 GitHub API 核验：asset 大小、state=uploaded、sha256 与本地一致。

> GitHub MCP 没有「创建 Release / 上传资产」工具，所以这一步只能浏览器；代码/PR/评论等其它 GitHub 操作优先用 MCP。

### 6. 更新市场登记仓库（PR）

- 仓库 `lwfjyjy/openidea-plugins`（fork），分支 `add-lwf-oi-path-assistant`。
- 更新 `plugins/lwf.oi-path-assistant.toml`：只保留最新 `[[versions]]`，填 `version` / `package`（指向 v<version> Release 资产）/ `sha256`。
- push 后更新/新建 PR，registry CI 必须为 **success**。
- 若 CI 因「资产尚未上传」失败：上传资产后，推一个空提交重新触发（无需权限点 re-run）。
- 合并后市场索引约 **1 小时**刷新；在 设置 › 插件 › Available 搜 `oi-path-assistant` 安装。

### 7. 安装与升级注意

- 插件 id 为 `lwf.oi-path-assistant`；与旧 id `lwf.quick-path` 不同，升级前需先卸载旧版。
- 旧版设置（Agent 列表等）**不会迁移**，需重新填写。

### 工具分工速查

| 工作 | 用什么 |
| --- | --- |
| 编译 / 测试 / 签名打包 / git commit / tag push | 本地命令 |
| 建分支、提交文件、开/改/合并 PR、评论、请求评审、读 Release/CI | GitHub MCP |
| 创建 Release + 上传 `.oiplugin` | Playwright（浏览器已登录） |
| 重跑 CI | 推空提交触发 |

