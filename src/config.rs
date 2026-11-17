//! 配置数据模型与读写。
//!
//! 整份配置以 JSON 存在 storage 键 [`STORAGE_KEY`] 里：路径模板、当前预设 id、Agent 列表。
//! 读取时缺失或损坏就回落到内置默认值；写入前统一做规范化与校验，保证面板拿到的永远是合法数据。

use openidea_plugin::{Error, host};
use serde::{Deserialize, Serialize};

/// storage 中保存整份配置的键。
pub const STORAGE_KEY: &str = "config";
/// Agent 数量上限，避免面板无限膨胀。
pub const MAX_AGENTS: usize = 50;
/// 用户上传 SVG 图标的字节上限。
pub const MAX_SVG: usize = 16 * 1024;

/// 一个路径模板预设，供面板「快速写入」使用。
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub template: &'static str,
}

/// 内置预设：Claude Code / Codex / 纯路径。
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "claude",
        name: "Claude Code",
        template: "@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]",
    },
    Preset {
        id: "codex",
        name: "Codex",
        template: "@{path}[ONE::{start}][MANY::{start}-{end}]",
    },
    Preset {
        id: "path",
        name: "纯路径",
        template: "{path}",
    },
];

/// Agent 图标：内置 logo 或用户上传的 SVG 文本。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AgentIcon {
    Builtin { key: String },
    Svg { data: String },
}

/// 一个可启动的 Agent。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub command: String,
    pub icon: AgentIcon,
}

/// 整份配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_template")]
    pub path_template: String,
    #[serde(default = "default_preset")]
    pub preset_id: String,
    #[serde(default)]
    pub agents: Vec<Agent>,
}

fn default_version() -> u32 {
    1
}
fn default_template() -> String {
    PRESETS[0].template.to_owned()
}
fn default_preset() -> String {
    PRESETS[0].id.to_owned()
}

impl Default for Config {
    /// 首次使用时的默认值：六个常见 Agent，模板用 Claude Code 预设。
    fn default() -> Self {
        let builtin = [
            ("claude", "Claude Code"),
            ("codex", "Codex"),
            ("opencode", "OpenCode"),
            ("pi", "Pi"),
            ("codebuddy", "CodeBuddy"),
            ("qoder", "Qoder"),
        ];
        let agents = builtin
            .iter()
            .map(|(id, name)| Agent {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                command: (*id).to_owned(),
                icon: AgentIcon::Builtin {
                    key: (*id).to_owned(),
                },
            })
            .collect();
        Self {
            version: 1,
            path_template: default_template(),
            preset_id: default_preset(),
            agents,
        }
    }
}

impl Config {
    /// 规范化：字段收敛到合法范围（读取后、保存前都调用）。
    pub fn normalize(&mut self) {
        self.version = 1;
        self.agents.truncate(MAX_AGENTS);
        for a in &mut self.agents {
            a.id = a.id.trim().to_owned();
            a.name = a.name.trim().to_owned();
            a.command = a.command.trim().to_owned();
        }
    }

    /// 保存前校验，返回给用户看的错误说明。
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for a in &self.agents {
            if a.id.is_empty() {
                return Err("Agent 缺少 id".to_owned());
            }
            if !seen.insert(a.id.as_str()) {
                return Err(format!("Agent id 重复：{}", a.id));
            }
            if a.name.is_empty() {
                return Err("Agent 名称不能为空".to_owned());
            }
            if a.command.is_empty() {
                return Err(format!("Agent「{}」的启动命令不能为空", a.name));
            }
            if let AgentIcon::Svg { data } = &a.icon {
                sanitize_svg(data).map_err(|e| format!("Agent「{}」的图标无效：{e}", a.name))?;
            }
        }
        Ok(())
    }
}

/// 从 storage 读取配置；缺失或损坏时返回内置默认值。
pub fn load() -> Config {
    let mut cfg = host::storage::get(STORAGE_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Config>(&s).ok())
        .unwrap_or_default();
    cfg.normalize();
    cfg
}

/// 写入 storage。
pub fn save(cfg: &Config) -> Result<(), Error> {
    let text = serde_json::to_string(cfg).map_err(|e| Error::Failed(e.to_string()))?;
    host::storage::set(STORAGE_KEY, &text)
}

/// 校验用户上传的 SVG：限制大小、必须是 SVG 根、禁脚本/外部引用等危险内容。
///
/// 图标最终以 `data:image/svg+xml` 的 `<img>` 呈现（本身不执行脚本），这里再做一层静态净化，
/// 避免明显可疑的内容被保存进配置。
pub fn sanitize_svg(src: &str) -> Result<(), String> {
    let s = src.trim();
    if s.is_empty() {
        return Err("图标内容为空".to_owned());
    }
    if s.len() > MAX_SVG {
        return Err("图标过大（最多 16 KiB）".to_owned());
    }
    if !s.starts_with("<svg") {
        return Err("不是有效的 SVG（应以 <svg 开头）".to_owned());
    }
    let low = s.to_ascii_lowercase();
    for bad in [
        "<script",
        "<foreignobject",
        "<!doctype",
        "<!entity",
        "<![cdata",
        "<?xml-stylesheet",
        "<iframe",
        "<object",
        "<embed",
        "javascript:",
        "data:text/html",
    ] {
        if low.contains(bad) {
            return Err(format!("包含不允许的内容：{bad}"));
        }
    }
    let bytes = low.as_bytes();
    for i in 0..bytes.len() {
        // 事件处理属性：on<字母>...=（要求前面是边界，避免误伤 font- 之类）
        if bytes[i] == b'o' && bytes.get(i + 1) == Some(&b'n') {
            let boundary = i == 0
                || matches!(
                    bytes[i - 1],
                    b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'' | b'<'
                );
            if boundary {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let mut k = j;
                while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                    k += 1;
                }
                if j > i + 2 && bytes.get(k) == Some(&b'=') {
                    return Err("包含事件处理属性".to_owned());
                }
            }
        }
        // href 只允许以 # 开头的片段引用，杜绝外部资源
        if bytes[i..].starts_with(b"href") {
            let mut k = i + 4;
            while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                k += 1;
            }
            if bytes.get(k) == Some(&b'=') {
                k += 1;
                while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'"' | b'\'') {
                    k += 1;
                }
                if bytes.get(k).is_some_and(|c| *c != b'#') {
                    return Err("包含外部引用（href 必须指向 #片段）".to_owned());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_six_agents_and_claude_template() {
        let cfg = Config::default();
        assert_eq!(cfg.agents.len(), 6);
        assert_eq!(
            cfg.path_template,
            "@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]"
        );
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_command_and_duplicate_id() {
        let mut cfg = Config::default();
        cfg.agents[0].command.clear();
        assert!(cfg.validate().is_err());

        let mut cfg = Config::default();
        cfg.agents[1].id = cfg.agents[0].id.clone();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn sanitize_svg_accepts_plain_and_rejects_dangerous() {
        assert!(
            sanitize_svg(r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0h4"/></svg>"#)
                .is_ok()
        );
        assert!(sanitize_svg(r#"<svg><script>alert(1)</script></svg>"#).is_err());
        assert!(sanitize_svg(r#"<svg onload="alert(1)"></svg>"#).is_err());
        assert!(sanitize_svg(r#"<svg><image href="https://x/y.png"/></svg>"#).is_err());
        assert!(sanitize_svg(r##"<svg><use href="#local"/></svg>"##).is_ok());
        assert!(sanitize_svg("not an svg").is_err());
    }
}
