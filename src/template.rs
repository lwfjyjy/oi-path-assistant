//! 路径模板渲染：纯函数、无副作用，方便单测。
//!
//! 模板语法：
//! - `{path}` 项目相对路径；`{start}` / `{end}` 是 1 基行号。
//! - `[ONE: ...]` 只在恰好选中 1 行时输出其中内容，否则整段消失。
//! - `[MANY: ...]` 只在选中多行时输出其中内容，否则整段消失。
//! - 未选中任何行时两段都不输出，只剩纯路径。

/// 1 基、闭区间的行选择（start..=end）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSel {
    pub start: u32,
    pub end: u32,
}

impl LineSel {
    /// 是否只选中了一行。
    pub fn is_one(&self) -> bool {
        self.start == self.end
    }
}

/// 把编辑器的 0 基起止位置换算成 1 基行范围；空选区（光标）返回 `None`。
///
/// 结束列等于 0 且跨行时视为「整行选择」，结束行减 1，避免把光标所在的下一行也算进去。
pub fn selection_lines(
    start_line: u32,
    start_char: u32,
    end_line: u32,
    end_char: u32,
) -> Option<LineSel> {
    if start_line == end_line && start_char == end_char {
        return None;
    }
    let end = if end_char == 0 && end_line > start_line {
        end_line - 1
    } else {
        end_line
    };
    if end < start_line {
        return None;
    }
    Some(LineSel {
        start: start_line + 1,
        end: end + 1,
    })
}

/// 渲染模板：先处理条件段，再替换占位符。
pub fn render(template: &str, path: &str, sel: Option<LineSel>) -> String {
    let body = expand(template, sel);
    let (start, end) = match sel {
        Some(s) => (s.start.to_string(), s.end.to_string()),
        None => (String::new(), String::new()),
    };
    body.replace("{path}", path)
        .replace("{start}", &start)
        .replace("{end}", &end)
}

/// 处理 `[ONE: ...]` / `[MANY: ...]` 条件段；非条件方括号原样保留。
fn expand(template: &str, sel: Option<LineSel>) -> String {
    let mut out = String::new();
    let mut rest = template;
    loop {
        let Some(open) = rest.find('[') else {
            out.push_str(rest);
            break;
        };
        let Some(close_rel) = rest[open..].find(']') else {
            out.push_str(rest);
            break;
        };
        let close = open + close_rel;
        out.push_str(&rest[..open]);
        let seg = &rest[open + 1..close];
        if let Some(inner) = seg.strip_prefix("ONE:") {
            if sel.is_some_and(|s| s.is_one()) {
                out.push_str(inner);
            }
        } else if let Some(inner) = seg.strip_prefix("MANY:") {
            if sel.is_some_and(|s| !s.is_one()) {
                out.push_str(inner);
            }
        } else {
            out.push('[');
            out.push_str(seg);
            out.push(']');
        }
        rest = &rest[close + 1..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "@{path}[ONE:#L{start}][MANY:#L{start}-L{end}]";
    const CODEX: &str = "@{path}[ONE::{start}][MANY::{start}-{end}]";

    #[test]
    fn no_selection_is_plain_path() {
        assert_eq!(render(CLAUDE, "src/A.vue", None), "@src/A.vue");
        assert_eq!(render(CODEX, "src/A.vue", None), "@src/A.vue");
    }

    #[test]
    fn one_line_uses_one_branch() {
        let sel = Some(LineSel { start: 5, end: 5 });
        assert_eq!(render(CLAUDE, "src/A.vue", sel), "@src/A.vue#L5");
        assert_eq!(render(CODEX, "src/A.vue", sel), "@src/A.vue:5");
    }

    #[test]
    fn many_lines_use_many_branch() {
        let sel = Some(LineSel { start: 5, end: 9 });
        assert_eq!(render(CLAUDE, "src/A.vue", sel), "@src/A.vue#L5-L9");
        assert_eq!(render(CODEX, "src/A.vue", sel), "@src/A.vue:5-9");
    }

    #[test]
    fn end_column_zero_drops_last_line() {
        // 从第 5 行行首拖到第 7 行行首：整行是 5、6 两行 → 结束行减 1
        assert_eq!(
            selection_lines(4, 0, 6, 0),
            Some(LineSel { start: 5, end: 6 })
        );
        // 起止在同一行行首、无长度 → 视为无选区
        assert_eq!(selection_lines(4, 0, 4, 0), None);
    }

    #[test]
    fn caret_is_none_and_plain_selection_counts() {
        assert_eq!(selection_lines(2, 3, 2, 3), None);
        assert_eq!(
            selection_lines(2, 0, 2, 4),
            Some(LineSel { start: 3, end: 3 })
        );
    }

    #[test]
    fn edge_empty_path_and_empty_template() {
        assert_eq!(render("{path}", "", None), "");
        assert_eq!(render("", "src/A.vue", None), "");
        // 未知方括号段原样保留
        assert_eq!(render("a[b]c{path}", "p", None), "a[b]cp");
    }
}
