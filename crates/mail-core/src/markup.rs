//! 写信正文的四种格式（富文本 / Markdown / HTML / 纯文本）互转。
//!
//! WP-10-3：Windows 版正文编辑区是一个多行文本框（`gpui-kit` 只提供
//! `Textarea`，没有原版 GTK 版的所见即所得 `RichEditor`），所以「格式」在
//! 这里表现为**同一个正文的四种表示**：切换格式时把当前正文转换到目标表示，
//! 用户因此能够在不离开写信器的情况下对照「写出来的样子」和「发出去的样子」。
//!
//! 转换一律经过 HTML 中转（四种表示都能写出、也都能读回的公共形态），
//! 与原版 `src/ui/compose/format.rs::on_load_as` 的做法一致。
//!
//! 这里实现的是**邮件写信场景够用**的 CommonMark 子集 + 常见 HTML 标签子集，
//! 不追求完整的 CommonMark 兼容（表格、脚注、定义列表等留给原版
//! `src/mail/markdown/` 的完整移植，见 WP-12）。

/// 写信正文的四种格式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComposeFormat {
    /// 富文本：按语法边写边排版，发出的是 HTML。
    #[default]
    Rich,
    /// Markdown：写 Markdown 源，发出的是排版后的邮件。
    Markdown,
    /// HTML：手写邮件的 HTML 源。
    Html,
    /// 纯文本：完全不排版，语法标记原样保留。
    Plain,
}

impl ComposeFormat {
    /// 四种格式的展示顺序（与原版菜单一致）。
    pub const ALL: [ComposeFormat; 4] = [
        ComposeFormat::Rich,
        ComposeFormat::Markdown,
        ComposeFormat::Html,
        ComposeFormat::Plain,
    ];

    /// 界面上显示的格式名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Rich => "富文本",
            Self::Markdown => "Markdown",
            Self::Html => "HTML 源码",
            Self::Plain => "纯文本",
        }
    }

    /// 格式切换菜单里一行说明。
    pub fn hint(self) -> &'static str {
        match self {
            Self::Rich => "边写边排版，发出的是 HTML",
            Self::Markdown => "按 Markdown 语法书写，发出的是排版后的邮件",
            Self::Html => "直接手写邮件的 HTML 源码",
            Self::Plain => "完全不排版，语法标记原样保留",
        }
    }

    /// 是否提供加粗 / 斜体 / 列表等语法插入按钮。
    ///
    /// 对齐原版，源码与纯文本模式直接编辑正文。
    pub fn has_format_toolbar(self) -> bool {
        self == Self::Rich
    }

    /// 存档 / 配置里使用的稳定标识。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rich => "rich",
            Self::Markdown => "markdown",
            Self::Html => "html",
            Self::Plain => "plain",
        }
    }

    /// 由 [`Self::as_str`] 的标识还原，未知值退回默认格式。
    pub fn from_str_lossy(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Self::Markdown,
            "html" => Self::Html,
            "plain" | "text" => Self::Plain,
            _ => Self::Rich,
        }
    }
}

/// 把正文从一种格式转换到另一种格式。
///
/// `body` 是当前编辑区里的原文；返回目标格式下的文本。同一格式之间是恒等转换。
pub fn convert(body: &str, from: ComposeFormat, to: ComposeFormat) -> String {
    if from == to {
        return body.to_string();
    }
    // 统一先归一到 HTML（四种格式的共同中转形态），再落到目标格式。
    let html = match from {
        ComposeFormat::Markdown => markdown_to_html(body),
        ComposeFormat::Plain => plain_to_html(body),
        // 旧草稿和从其他模式切来的富文本可直接保存 HTML。
        // 手输正文使用工具条插入的标记，并保留手动换行。
        ComposeFormat::Rich => {
            if looks_like_html(body) {
                body.to_string()
            } else {
                render_markdown(body, true)
            }
        }
        ComposeFormat::Html => body.to_string(),
    };
    match to {
        ComposeFormat::Rich | ComposeFormat::Html => html,
        ComposeFormat::Markdown => html_to_markdown(&html),
        ComposeFormat::Plain => html_to_plain(&html),
    }
}

/// 粗略判断一段正文是不是 HTML（用于富文本模式的兜底）。
pub fn looks_like_html(body: &str) -> bool {
    let trimmed = body.trim_start();
    if !trimmed.starts_with('<') {
        return false;
    }
    // `<` 后面紧跟标签名或 `/` 才算标签，避免把 "<3"、"a < b" 当成 HTML。
    let rest = &trimmed[1..];
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => true,
        Some('/') | Some('!') => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Markdown -> HTML
// ---------------------------------------------------------------------------

/// 行内元素占位符的私有区字符（处理代码段时先取出，避免二次解析）。
const PLACEHOLDER: char = '\u{E000}';

/// 把 Markdown 原文转成邮件可用的 HTML（块级 + 行内，CommonMark 常用子集）。
pub fn markdown_to_html(md: &str) -> String {
    render_markdown(md, false)
}

fn render_markdown(md: &str, hard_breaks: bool) -> String {
    let normalized = md.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // 围栏代码块。
        if let Some(fence) = fence_open(trimmed) {
            i += 1;
            let mut code = String::new();
            while i < lines.len() && !is_fence_close(lines[i].trim_start(), fence) {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            // 吃掉收尾围栏。
            if i < lines.len() {
                i += 1;
            }
            out.push_str("<pre style=\"margin:0 0 12px;padding:8px 10px;background:#f4f4f5;border-radius:4px;overflow:auto\"><code>");
            out.push_str(&escape_html(&code));
            out.push_str("</code></pre>\n");
            continue;
        }

        // 水平分割线。
        if is_hr(trimmed) {
            out.push_str("<hr>\n");
            i += 1;
            continue;
        }

        // 标题。
        if let Some((level, text)) = heading(trimmed) {
            out.push_str(&format!("<h{level}>{}</h{level}>\n", inline(text)));
            i += 1;
            continue;
        }

        // 引用块：把连续的 `>` 行收集起来递归转换。
        if trimmed.starts_with('>') {
            let mut inner = String::new();
            while i < lines.len() {
                let cur = lines[i].trim_start();
                if !cur.starts_with('>') {
                    break;
                }
                let stripped = cur[1..].strip_prefix(' ').unwrap_or(&cur[1..]);
                inner.push_str(stripped);
                inner.push('\n');
                i += 1;
            }
            out.push_str("<blockquote style=\"margin:0 0 12px;padding-left:10px;border-left:3px solid #d4d4d8;color:#52525b\">");
            out.push_str(render_markdown(inner.trim_end(), hard_breaks).trim_end());
            out.push_str("</blockquote>\n");
            continue;
        }

        // 无序列表。
        if bullet_item(trimmed).is_some() {
            out.push_str("<ul style=\"margin:0 0 12px;padding-left:22px\">\n");
            while i < lines.len() {
                let cur = lines[i].trim_start();
                match bullet_item(cur) {
                    Some(text) => {
                        out.push_str(&format!("<li>{}</li>\n", inline(text)));
                        i += 1;
                    }
                    None => break,
                }
            }
            out.push_str("</ul>\n");
            continue;
        }

        // 有序列表。
        if ordered_item(trimmed).is_some() {
            out.push_str("<ol style=\"margin:0 0 12px;padding-left:22px\">\n");
            while i < lines.len() {
                let cur = lines[i].trim_start();
                match ordered_item(cur) {
                    Some(text) => {
                        out.push_str(&format!("<li>{}</li>\n", inline(text)));
                        i += 1;
                    }
                    None => break,
                }
            }
            out.push_str("</ol>\n");
            continue;
        }

        // 空行：跳过。
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // 段落：收集到空行或下一个块级元素开头为止。
        let mut para = String::new();
        while i < lines.len() {
            let cur = lines[i];
            let t = cur.trim_start();
            if t.is_empty()
                || fence_open(t).is_some()
                || is_hr(t)
                || heading(t).is_some()
                || t.starts_with('>')
                || bullet_item(t).is_some()
                || ordered_item(t).is_some()
            {
                break;
            }
            if !para.is_empty() {
                para.push('\n');
            }
            para.push_str(cur.trim_end());
            i += 1;
        }
        let para = inline(&para);
        let para = if hard_breaks {
            para.replace('\n', "<br>")
        } else {
            para
        };
        out.push_str(&format!("<p style=\"margin:0 0 12px\">{para}</p>\n"));
    }
    out.trim_end().to_string()
}

/// 围栏代码块的开头：返回围栏字符（`` ` `` 或 `~`）与长度。
fn fence_open(line: &str) -> Option<char> {
    for ch in ['`', '~'] {
        if line.len() >= 3 && line.chars().take(3).all(|c| c == ch) {
            return Some(ch);
        }
    }
    None
}

/// 是否是该围栏代码块的收尾行。
fn is_fence_close(line: &str, fence: char) -> bool {
    !line.is_empty() && line.chars().all(|c| c == fence) && line.chars().count() >= 3
}

/// 水平分割线（`---` / `***` / `___`，至少三个）。
fn is_hr(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|ch| compact.chars().all(|c| c == *ch))
}

/// ATX 标题：`## 文本`。
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &line[level..];
    let text = rest.strip_prefix(' ').unwrap_or(rest);
    if rest.is_empty() {
        return Some((level, ""));
    }
    if !rest.starts_with(' ') {
        return None;
    }
    Some((level, text.trim_end()))
}

/// 无序列表项：返回去掉标记后的文本。
fn bullet_item(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "));
    rest.map(|text| text.trim_end())
}

/// 有序列表项：返回去掉 `1.` / `1)` 标记后的文本。
fn ordered_item(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))?;
    Some(rest.trim_end())
}

/// 行内元素转换：代码段 / 图片 / 链接 / 加粗 / 斜体 / 删除线 / 自动链接。
fn inline(text: &str) -> String {
    // 先把代码段抽出来占位，避免其中的 `*`、`_` 被当成强调标记。
    let mut codes: Vec<String> = Vec::new();
    let mut staged = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        if chars[idx] == '`' {
            let run = chars[idx..].iter().take_while(|c| **c == '`').count();
            let ticks: String = std::iter::repeat('`').take(run).collect();
            let tail: String = chars[idx + run..].iter().collect();
            if let Some(end) = tail.find(&ticks) {
                let code: String = chars[idx + run..idx + run + tail[..end].chars().count()]
                    .iter()
                    .collect();
                staged.push(PLACEHOLDER);
                staged.push_str(&codes.len().to_string());
                staged.push(PLACEHOLDER);
                codes.push(format!(
                    "<code style=\"padding:1px 4px;background:#f4f4f5;border-radius:3px\">{}</code>",
                    escape_html(&code)
                ));
                idx = idx + run + tail[..end].chars().count() + run;
                continue;
            }
        }
        staged.push(chars[idx]);
        idx += 1;
    }

    let escaped = escape_html(&staged);
    let mut out = escaped;

    // 图片 ![alt](src) 先于链接处理。
    out = replace_pairs(&out, "![", "](", ")", |alt, src| {
        format!("<img src=\"{}\" alt=\"{}\">", src.trim(), alt.trim())
    });
    out = replace_pairs(&out, "[", "](", ")", |label, href| {
        format!("<a href=\"{}\">{}</a>", href.trim(), label.trim())
    });
    out = replace_wrapped(&out, "**", "<strong>", "</strong>");
    out = replace_wrapped(&out, "__", "<strong>", "</strong>");
    out = replace_wrapped(&out, "~~", "<s>", "</s>");
    out = replace_wrapped(&out, "*", "<em>", "</em>");
    out = replace_wrapped(&out, "_", "<em>", "</em>");

    // 还原占位的代码段。
    let mut restored = String::new();
    let mut iter = out.chars().peekable();
    while let Some(c) = iter.next() {
        if c == PLACEHOLDER {
            let mut num = String::new();
            while let Some(&next) = iter.peek() {
                if next == PLACEHOLDER {
                    iter.next();
                    break;
                }
                num.push(next);
                iter.next();
            }
            if let Ok(n) = num.parse::<usize>() {
                if let Some(html) = codes.get(n) {
                    restored.push_str(html);
                    continue;
                }
            }
            restored.push(c);
            restored.push_str(&num);
            continue;
        }
        restored.push(c);
    }
    restored
}

/// 把 `open ... mid ... close` 形式的成对语法转换成标签（用于 `[文字](链接)`）。
fn replace_pairs(
    text: &str,
    open: &str,
    mid: &str,
    close: &str,
    build: impl Fn(&str, &str) -> String,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        let after_open = &rest[start + open.len()..];
        let Some(mid_at) = after_open.find(mid) else {
            break;
        };
        let inner = &after_open[..mid_at];
        let after_mid = &after_open[mid_at + mid.len()..];
        let Some(close_at) = after_mid.find(close) else {
            break;
        };
        let target = &after_mid[..close_at];
        if inner.trim().is_empty() || target.trim().is_empty() {
            break;
        }
        out.push_str(&rest[..start]);
        out.push_str(&build(inner, target));
        rest = &after_mid[close_at + close.len()..];
    }
    out.push_str(rest);
    out
}

/// 把成对的强调标记转换成标签（`**粗**` -> `<strong>粗</strong>`）。
fn replace_wrapped(text: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(marker) {
        let after = &rest[start + marker.len()..];
        let Some(end) = after.find(marker) else {
            break;
        };
        let inner = &after[..end];
        if inner.trim().is_empty() {
            // `**` 后面紧跟另一个 `**`：原样保留，继续往后找。
            out.push_str(&rest[..start + marker.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(open);
        out.push_str(inner);
        out.push_str(close);
        rest = &after[end + marker.len()..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// HTML -> Markdown / 纯文本
// ---------------------------------------------------------------------------

/// 把 HTML 转回 Markdown 源码。
pub fn html_to_markdown(html: &str) -> String {
    let mut writer = BlockWriter::new(MarkdownSink::default());
    scan_html(html, &mut writer);
    writer.finish()
}

/// 把 HTML 压平成纯文本（保留段落与换行，去掉所有标签）。
pub fn html_to_plain(html: &str) -> String {
    let mut writer = BlockWriter::new(PlainSink::default());
    scan_html(html, &mut writer);
    writer.finish()
}

/// 把纯文本折成 HTML（空行分段，段内换行用 `<br>`）。
pub fn plain_to_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    let mut out = String::new();
    for block in normalized.split("\n\n") {
        let block = block.trim_matches('\n');
        if block.trim().is_empty() {
            continue;
        }
        let body = escape_html(block).replace('\n', "<br>");
        out.push_str(&format!("<p style=\"margin:0 0 12px\">{body}</p>\n"));
    }
    out.trim_end().to_string()
}

/// 输出目标（Markdown 源码或纯文本）的差异只在这几个钩子上。
trait Sink {
    fn text(&mut self, text: &str);
    fn newline(&mut self);
    /// 结构性换行（列表项之间、块内换行），不带 Markdown 的硬换行空格。
    fn newline_break(&mut self);
    fn blank_line(&mut self);
    fn open_inline(&mut self, kind: InlineKind);
    fn close_inline(&mut self, kind: InlineKind);
    fn open_block(&mut self, kind: BlockKind);
    fn close_block(&mut self, kind: BlockKind);
    fn image(&mut self, alt: &str, src: &str);
    fn finish(self: Box<Self>) -> String;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineKind {
    Strong,
    Em,
    Strike,
    Code,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockKind {
    Heading(usize),
    BulletList,
    OrderedList,
    /// 列表项；`true` 表示它属于有序列表。
    Item(bool),
    Quote,
    Pre,
    Rule,
}

/// 逐块输出的游标：负责在块与块之间补空行，避免出现连续三行空白。
struct BlockWriter {
    sink: Box<dyn Sink>,
    /// 上一个输出字符是否是换行，用于判断当前是否已经在行首。
    at_line_start: bool,
}

impl BlockWriter {
    fn new(sink: impl Sink + 'static) -> Self {
        Self {
            sink: Box::new(sink),
            at_line_start: true,
        }
    }

    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.sink.text(text);
        self.at_line_start = text.ends_with('\n');
    }

    fn block_boundary(&mut self) {
        if !self.at_line_start {
            self.sink.blank_line();
            self.at_line_start = true;
        }
    }

    /// 只保证当前在行首（换一行但不空行，列表项之间用这个）。
    fn line_start(&mut self) {
        if !self.at_line_start {
            self.sink.newline_break();
            self.at_line_start = true;
        }
    }

    fn finish(mut self) -> String {
        self.block_boundary();
        self.sink.finish()
    }
}

/// Markdown 输出：强调用标记符号，块用 `- ` / `1. ` / `> ` / 围栏。
#[derive(Default)]
struct MarkdownSink {
    out: String,
    /// 当前是否在引用块内（引用块里的每行都要带 `> `）。
    quote_depth: usize,
}

impl MarkdownSink {
    fn ensure_line_start(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
            self.push_quote_prefix();
        }
    }

    fn push_quote_prefix(&mut self) {
        for _ in 0..self.quote_depth {
            self.out.push_str("> ");
        }
    }
}

impl Sink for MarkdownSink {
    fn text(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn newline(&mut self) {
        // 行内换行在 Markdown 里是行尾两个空格 + 换行。
        if self.out.ends_with('\n') {
            return;
        }
        self.out.push_str("  \n");
        self.push_quote_prefix();
    }

    fn newline_break(&mut self) {
        if self.out.ends_with('\n') {
            return;
        }
        self.out.push('\n');
        self.push_quote_prefix();
    }

    fn blank_line(&mut self) {
        while self.out.ends_with('\n') || self.out.ends_with(' ') {
            self.out.pop();
        }
        if !self.out.is_empty() {
            self.out.push_str("\n\n");
            self.push_quote_prefix();
        }
    }

    fn open_inline(&mut self, kind: InlineKind) {
        self.out.push_str(match kind {
            InlineKind::Strong => "**",
            InlineKind::Em => "*",
            InlineKind::Strike => "~~",
            InlineKind::Code => "`",
            InlineKind::Link => "[",
        });
    }

    fn close_inline(&mut self, kind: InlineKind) {
        self.out.push_str(match kind {
            InlineKind::Strong => "**",
            InlineKind::Em => "*",
            InlineKind::Strike => "~~",
            InlineKind::Code => "`",
            InlineKind::Link => "]",
        });
    }

    fn open_block(&mut self, kind: BlockKind) {
        match kind {
            BlockKind::Heading(level) => {
                self.ensure_line_start();
                self.out.push_str(&"#".repeat(level));
                self.out.push(' ');
            }
            BlockKind::Item(ordered) => {
                self.ensure_line_start();
                let marker = if ordered { "1. " } else { "- " };
                self.out.push_str(marker);
            }
            BlockKind::Quote => {
                self.ensure_line_start();
                self.quote_depth += 1;
                self.out.push_str("> ");
            }
            BlockKind::Rule => {
                self.ensure_line_start();
                self.out.push_str("---\n");
            }
            BlockKind::Pre => {
                self.ensure_line_start();
                self.out.push_str("```\n");
            }
            BlockKind::BulletList | BlockKind::OrderedList => {}
        }
    }

    fn close_block(&mut self, kind: BlockKind) {
        match kind {
            BlockKind::Pre => {
                self.ensure_line_start();
                self.out.push_str("```\n");
            }
            BlockKind::Heading(_) | BlockKind::Item(_) => self.ensure_line_start(),
            BlockKind::Quote => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.ensure_line_start();
            }
            BlockKind::BulletList | BlockKind::OrderedList | BlockKind::Rule => {}
        }
    }

    fn image(&mut self, alt: &str, src: &str) {
        self.out.push_str(&format!("![{alt}]({src})"));
    }

    fn finish(mut self: Box<Self>) -> String {
        while self.out.ends_with('\n') || self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out
    }
}

/// 纯文本输出：没有标记符号，块之间用空行分隔。
#[derive(Default)]
struct PlainSink {
    out: String,
}

impl PlainSink {
    fn ensure_line_start(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }
}

impl Sink for PlainSink {
    fn text(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn newline(&mut self) {
        self.ensure_line_start();
    }

    fn newline_break(&mut self) {
        self.ensure_line_start();
    }

    fn blank_line(&mut self) {
        while self.out.ends_with('\n') || self.out.ends_with(' ') {
            self.out.pop();
        }
        if !self.out.is_empty() {
            self.out.push_str("\n\n");
        }
    }

    fn open_inline(&mut self, _kind: InlineKind) {}

    fn close_inline(&mut self, _kind: InlineKind) {}

    fn open_block(&mut self, kind: BlockKind) {
        match kind {
            BlockKind::Item(_) => {
                self.ensure_line_start();
                self.out.push_str("- ");
            }
            BlockKind::Quote => {
                self.ensure_line_start();
                self.out.push_str("> ");
            }
            BlockKind::BulletList
            | BlockKind::OrderedList
            | BlockKind::Pre
            | BlockKind::Heading(_)
            | BlockKind::Rule => {}
        }
    }

    fn close_block(&mut self, kind: BlockKind) {
        match kind {
            BlockKind::Heading(_) | BlockKind::Item(_) => self.ensure_line_start(),
            BlockKind::Rule => {
                self.ensure_line_start();
                self.out.push_str("---\n");
            }
            _ => {}
        }
    }

    fn image(&mut self, alt: &str, _src: &str) {
        self.out.push_str(alt);
    }

    fn finish(mut self: Box<Self>) -> String {
        while self.out.ends_with('\n') || self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out
    }
}

/// 线性扫描 HTML：只认识写信场景里会出现的标签，未知标签直接丢弃（保留其文本）。
fn scan_html(html: &str, writer: &mut BlockWriter) {
    let mut cursor = 0usize;
    let bytes = html.as_bytes();
    let mut inline_stack: Vec<InlineKind> = Vec::new();
    let mut pre_depth = 0usize;
    // 列表嵌套：true 表示有序列表。
    let mut list_stack: Vec<bool> = Vec::new();
    // 链接的 href 在 `</a>` 时才需要，暂存。
    let mut pending_href: Option<String> = None;

    while cursor < html.len() {
        if bytes[cursor] == b'<' {
            let Some(end) = html[cursor..].find('>').map(|offset| cursor + offset) else {
                break;
            };
            let raw = &html[cursor + 1..end];
            let (closing, _self_closing, name, attrs) = parse_tag(raw);
            let tag = name.as_str();

            match (closing, tag) {
                (true, "p") | (true, "div") | (true, "section") | (true, "article") => {
                    writer.block_boundary();
                }
                (false, "p") | (false, "div") | (false, "section") | (false, "article") => {
                    writer.block_boundary();
                }
                (_, "br") => {
                    writer.sink.newline();
                    writer.at_line_start = true;
                }
                (_, "hr") => {
                    writer.block_boundary();
                    writer.sink.open_block(BlockKind::Rule);
                    writer.at_line_start = true;
                    writer.block_boundary();
                }
                (false, "h1")
                | (false, "h2")
                | (false, "h3")
                | (false, "h4")
                | (false, "h5")
                | (false, "h6") => {
                    writer.block_boundary();
                    let level = tag[1..].parse::<usize>().unwrap_or(1);
                    writer.sink.open_block(BlockKind::Heading(level));
                    writer.at_line_start = true;
                }
                (true, "h1")
                | (true, "h2")
                | (true, "h3")
                | (true, "h4")
                | (true, "h5")
                | (true, "h6") => {
                    writer.sink.close_block(BlockKind::Heading(1));
                    writer.block_boundary();
                }
                (false, "ul") => {
                    writer.block_boundary();
                    list_stack.push(false);
                    writer.sink.open_block(BlockKind::BulletList);
                }
                (true, "ul") => {
                    list_stack.pop();
                    writer.sink.close_block(BlockKind::BulletList);
                    writer.block_boundary();
                }
                (false, "ol") => {
                    writer.block_boundary();
                    list_stack.push(true);
                    writer.sink.open_block(BlockKind::OrderedList);
                }
                (true, "ol") => {
                    list_stack.pop();
                    writer.sink.close_block(BlockKind::OrderedList);
                    writer.block_boundary();
                }
                (false, "li") => {
                    let ordered = *list_stack.last().unwrap_or(&false);
                    writer.line_start();
                    writer.sink.open_block(BlockKind::Item(ordered));
                    writer.at_line_start = false;
                }
                (true, "li") => {
                    writer.sink.close_block(BlockKind::Item(false));
                    writer.at_line_start = true;
                }
                (false, "blockquote") => {
                    writer.block_boundary();
                    writer.sink.open_block(BlockKind::Quote);
                    writer.at_line_start = false;
                }
                (true, "blockquote") => {
                    writer.sink.close_block(BlockKind::Quote);
                    writer.block_boundary();
                }
                (false, "pre") => {
                    writer.block_boundary();
                    writer.sink.open_block(BlockKind::Pre);
                    writer.at_line_start = true;
                    pre_depth += 1;
                }
                (true, "pre") => {
                    pre_depth = pre_depth.saturating_sub(1);
                    writer.sink.close_block(BlockKind::Pre);
                    writer.block_boundary();
                }
                (false, "strong") | (false, "b") => {
                    writer.sink.open_inline(InlineKind::Strong);
                    inline_stack.push(InlineKind::Strong);
                }
                (true, "strong") | (true, "b") => {
                    close_inline(writer, &mut inline_stack, InlineKind::Strong)
                }
                (false, "em") | (false, "i") => {
                    writer.sink.open_inline(InlineKind::Em);
                    inline_stack.push(InlineKind::Em);
                }
                (true, "em") | (true, "i") => {
                    close_inline(writer, &mut inline_stack, InlineKind::Em)
                }
                (false, "s") | (false, "del") | (false, "strike") => {
                    writer.sink.open_inline(InlineKind::Strike);
                    inline_stack.push(InlineKind::Strike);
                }
                (true, "s") | (true, "del") | (true, "strike") => {
                    close_inline(writer, &mut inline_stack, InlineKind::Strike)
                }
                (false, "code") if pre_depth == 0 => {
                    writer.sink.open_inline(InlineKind::Code);
                    inline_stack.push(InlineKind::Code);
                }
                (true, "code") if pre_depth == 0 => {
                    close_inline(writer, &mut inline_stack, InlineKind::Code)
                }
                (false, "a") => {
                    pending_href = attr_value(&attrs, "href");
                    writer.sink.open_inline(InlineKind::Link);
                    inline_stack.push(InlineKind::Link);
                }
                (true, "a") => {
                    close_inline(writer, &mut inline_stack, InlineKind::Link);
                    if let Some(href) = pending_href.take() {
                        if !href.is_empty() {
                            writer.sink.text(&format!("({href})"));
                        }
                    }
                }
                (_, "img") => {
                    let src = attr_value(&attrs, "src").unwrap_or_default();
                    let alt = attr_value(&attrs, "alt").unwrap_or_default();
                    writer.sink.image(&alt, &src);
                }
                _ => {}
            }
            cursor = end + 1;
            continue;
        }

        // 文本：读到下一个 `<` 为止。
        let next = html[cursor..]
            .find('<')
            .map(|offset| cursor + offset)
            .unwrap_or(html.len());
        let mut text = decode_entities(&html[cursor..next]);
        if pre_depth == 0 {
            // 非代码块里的空白折成一个空格（HTML 的空白折叠规则），
            // 但行首/行尾的缩进换行直接丢掉。
            text = collapse_whitespace(&text);
        }
        if !text.is_empty() {
            writer.text(&text);
        }
        cursor = next;
    }
}

fn close_inline(writer: &mut BlockWriter, stack: &mut Vec<InlineKind>, kind: InlineKind) {
    if let Some(pos) = stack.iter().rposition(|item| *item == kind) {
        stack.truncate(pos);
        writer.sink.close_inline(kind);
    }
}

/// 解析 `<tag attr="v">`：返回（是否闭合、是否自闭合、标签名（小写）、原始属性串）。
fn parse_tag(raw: &str) -> (bool, bool, String, String) {
    let raw = raw.trim();
    let self_closing = raw.ends_with('/');
    let raw = raw.trim_end_matches('/').trim_end();
    let closing = raw.starts_with('/');
    let raw = if closing { &raw[1..] } else { raw };
    let name_end = raw.find(|c: char| c.is_whitespace()).unwrap_or(raw.len());
    let name = raw[..name_end].to_ascii_lowercase();
    (closing, self_closing, name, raw[name_end..].to_string())
}

/// 从属性串里取一个属性值（支持双引号、单引号与无引号）。
fn attr_value(attrs: &str, key: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut search_from = 0usize;
    while let Some(pos) = lower[search_from..].find(key) {
        let at = search_from + pos;
        // 属性名必须从边界开始（前面是空白），避免 `data-href` 命中 `href`。
        let boundary_ok = at > 0
            && attrs[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace());
        if !boundary_ok {
            search_from = at + key.len();
            continue;
        }
        let rest = &attrs[at + key.len()..];
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            search_from = at + key.len();
            continue;
        };
        let rest = rest.trim_start();
        let value = if let Some(inner) = rest.strip_prefix('"') {
            inner.split('"').next().unwrap_or("").to_string()
        } else if let Some(inner) = rest.strip_prefix('\'') {
            inner.split('\'').next().unwrap_or("").to_string()
        } else {
            rest.split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string()
        };
        return Some(decode_entities(&value));
    }
    None
}

/// HTML 空白折叠：连续空白合成一个空格。
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out
}

/// HTML 转义（`& < > "`）。
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// 从完整邮件 HTML 中提取可安全嵌入到另一封邮件正文中的片段：
/// 保留 `<style>...</style>`（常见于 `<head>` 内），并提取 `<body>...</body>`
/// 内容（保留 `<body>` 上的内联 `style` 属性），避免出现嵌套 `<html>/<body>`。
pub fn extract_embeddable_html(raw_html: &str) -> String {
    let trimmed = raw_html.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // 仅使用 ASCII 小写转换，保证字节偏移与原字符串严格对齐。
    let lower = trimmed.to_ascii_lowercase();

    // 收集所有 <style ...>...</style>（通常在 <head> 中，转发 HTML 邮件依赖它们还原样式）。
    let mut styles = String::new();
    let mut search_from = 0usize;
    while let Some(rel_start) = lower[search_from..].find("<style") {
        let start = search_from + rel_start;
        let after_tag = start + 6;
        let valid_boundary = lower[after_tag..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c.is_ascii_whitespace());
        if !valid_boundary {
            search_from = after_tag;
            continue;
        }
        let Some(rel_end) = lower[after_tag..].find("</style>") else {
            break;
        };
        let end = after_tag + rel_end + "</style>".len();
        styles.push_str(&trimmed[start..end]);
        styles.push('\n');
        search_from = end;
    }

    // 查找 <body>...</body>。
    let body_fragment = if let Some(body_open_start) = find_tag_start(&lower, "<body") {
        if let Some(rel_gt) = lower[body_open_start..].find('>') {
            let open_tag = &trimmed[body_open_start..body_open_start + rel_gt + 1];
            let content_start = body_open_start + rel_gt + 1;
            let content_end = lower[content_start..]
                .rfind("</body>")
                .map(|rel| content_start + rel)
                .unwrap_or(trimmed.len());
            let inner = trimmed[content_start..content_end].trim();
            if let Some(style_attr) = extract_style_attr(open_tag) {
                format!("<div style=\"{}\">{inner}</div>", escape_html(&style_attr))
            } else {
                format!("<div>{inner}</div>")
            }
        } else {
            trimmed.to_string()
        }
    } else {
        // 没有 <body> 时去掉外层 <!DOCTYPE> / <html> / <head>...</head> 壳。
        strip_document_shell(trimmed, &lower)
    };

    if styles.is_empty() {
        body_fragment
    } else if body_fragment.contains("<style") {
        // <body> 内部已经自带的 <style> 无需重复前置，仅保留 <head> 中的 <style>。
        let head_styles = extract_head_styles(trimmed, &lower);
        if head_styles.is_empty() {
            body_fragment
        } else {
            format!("{head_styles}{body_fragment}")
        }
    } else {
        format!("{styles}{body_fragment}")
    }
}

fn find_tag_start(lower: &str, prefix: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find(prefix) {
        let pos = from + rel;
        let after = pos + prefix.len();
        if lower[after..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c.is_ascii_whitespace())
        {
            return Some(pos);
        }
        from = after;
    }
    None
}

fn extract_head_styles(trimmed: &str, lower: &str) -> String {
    let Some(head_start) = find_tag_start(lower, "<head") else {
        return String::new();
    };
    let Some(rel_end) = lower[head_start..].find("</head>") else {
        return String::new();
    };
    let head_slice = &trimmed[head_start..head_start + rel_end];
    let head_lower = &lower[head_start..head_start + rel_end];
    let mut out = String::new();
    let mut pos = 0usize;
    while let Some(rel_s) = head_lower[pos..].find("<style") {
        let s = pos + rel_s;
        let after = s + 6;
        if !head_lower[after..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || c.is_ascii_whitespace())
        {
            pos = after;
            continue;
        }
        let Some(rel_e) = head_lower[after..].find("</style>") else {
            break;
        };
        let e = after + rel_e + "</style>".len();
        out.push_str(&head_slice[s..e]);
        out.push('\n');
        pos = e;
    }
    out
}

fn extract_style_attr(open_tag: &str) -> Option<String> {
    let lower = open_tag.to_ascii_lowercase();
    let idx = lower.find("style=")?;
    let rest = &open_tag[idx + "style=".len()..];
    let mut chars = rest.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let after_quote = &rest[quote.len_utf8()..];
    let end = after_quote.find(quote)?;
    let val = after_quote[..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

fn strip_document_shell(trimmed: &str, lower: &str) -> String {
    let mut s = trimmed.to_string();
    let mut l = lower.to_string();
    if let Some(head_start) = find_tag_start(&l, "<head") {
        if let Some(rel_end) = l[head_start..].find("</head>") {
            let head_end = head_start + rel_end + "</head>".len();
            s.replace_range(head_start..head_end, "");
            l = s.to_ascii_lowercase();
        }
    }
    if let Some(dt_start) = l.find("<!doctype") {
        if let Some(rel_gt) = l[dt_start..].find('>') {
            s.replace_range(dt_start..dt_start + rel_gt + 1, "");
            l = s.to_ascii_lowercase();
        }
    }
    if let Some(html_start) = find_tag_start(&l, "<html") {
        if let Some(rel_gt) = l[html_start..].find('>') {
            s.replace_range(html_start..html_start + rel_gt + 1, "");
            l = s.to_ascii_lowercase();
        }
    }
    if let Some(html_end) = l.rfind("</html>") {
        s.replace_range(html_end..html_end + "</html>".len(), "");
    }
    s.trim().to_string()
}

/// 将写信编辑区的正文（附言 + 纯文本引用）与原邮件完整 HTML 合并：
/// - 在 `body` 中按行定位 `separator`（如 `------------------ 转发邮件 ------------------`）；
/// - `separator` 之前的用户附言（含上方签名）按当前 `format` 转成 HTML；
/// - `separator` 及紧随的信头行渲染为结构化引用头，后接原邮件 `quoted_html`；
/// - 若用户已在编辑区删除 `separator`，则退化为普通的 `convert(body, format, ComposeFormat::Html)`。
pub fn compose_with_quoted_html(
    body: &str,
    format: ComposeFormat,
    separator: &str,
    quoted_html: &str,
    is_forward: bool,
    trailing_signature: Option<&str>,
) -> String {
    let sep_trimmed = separator.trim();
    if format == ComposeFormat::Plain || sep_trimmed.is_empty() || quoted_html.trim().is_empty() {
        return convert(body, format, ComposeFormat::Html);
    }

    // 按行查找分隔行在 body 中的字节起止位置。
    let mut offset = 0usize;
    let mut sep_byte_start = None;
    for raw_line in body.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        if line.trim() == sep_trimmed {
            sep_byte_start = Some(offset);
            break;
        }
        offset += raw_line.len();
    }
    if sep_byte_start.is_none() && body.trim() == sep_trimmed {
        sep_byte_start = Some(0);
    }

    let Some(sep_start) = sep_byte_start else {
        return convert(body, format, ComposeFormat::Html);
    };

    let before = body[..sep_start].trim_end_matches(['\r', '\n']);
    let after = &body[sep_start..];

    let before_html = if before.trim().is_empty() {
        String::new()
    } else {
        convert(before, format, ComposeFormat::Html)
    };

    // 收集分隔行及其紧随的非空信头行（发件人 / 时间 / 主题），遇到空行即止。
    let mut header_lines: Vec<&str> = Vec::new();
    for raw_line in after.lines().take(12) {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            break;
        }
        header_lines.push(trimmed);
    }
    let header_inner = header_lines
        .iter()
        .map(|l| escape_html(l))
        .collect::<Vec<_>>()
        .join("<br>");
    let header_html = format!(
        "<div class=\"kitemail-quote-header\" style=\"margin:16px 0 12px 0;padding-top:12px;border-top:1px solid #d0d7de;color:#57606a;font-size:12px;line-height:1.5;\">{header_inner}</div>"
    );

    let embedded_html = extract_embeddable_html(quoted_html);
    let quoted_block = if is_forward {
        format!("<div class=\"kitemail-forward-body\">{embedded_html}</div>")
    } else {
        format!(
            "<blockquote type=\"cite\" style=\"margin:8px 0 0 0;padding-left:12px;border-left:2px solid #d0d7de;\">{embedded_html}</blockquote>"
        )
    };

    let trailing_sig_html = trailing_signature
        .map(str::trim)
        .filter(|sig| !sig.is_empty() && after.trim_end().ends_with(sig))
        .map(|sig| {
            format!(
                "<div class=\"kitemail-signature\" style=\"margin-top:16px;\">{}</div>",
                convert(sig, format, ComposeFormat::Html)
            )
        })
        .unwrap_or_default();

    format!("{before_html}{header_html}{quoted_block}{trailing_sig_html}")
}

/// HTML 实体还原（命名实体 + 十进制 / 十六进制数字实体）。
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(semi) = after.find(';') else {
            out.push_str(after);
            return out;
        };
        if semi > 12 {
            // 太长不可能是实体，原样输出。
            out.push('&');
            rest = &after[1..];
            continue;
        }
        let entity = &after[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            _ => entity
                .strip_prefix('#')
                .and_then(|num| {
                    if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    }
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_bold_and_italic_become_tags() {
        let html = markdown_to_html("这是 **粗** 与 *斜* 和 `代码`");
        assert!(html.contains("<strong>粗</strong>"), "{html}");
        assert!(html.contains("<em>斜</em>"), "{html}");
        assert!(html.contains("<code"), "{html}");
        assert!(html.contains("代码</code>"), "{html}");
    }

    #[test]
    fn markdown_code_span_keeps_markers_literal() {
        let html = markdown_to_html("`**not bold**`");
        assert!(html.contains("**not bold**"), "{html}");
        assert!(!html.contains("<strong>"), "{html}");
    }

    #[test]
    fn markdown_lists_and_quote() {
        let html = markdown_to_html("- 一\n- 二\n\n1. 甲\n2. 乙\n\n> 引用");
        assert!(html.contains("<ul"), "{html}");
        assert!(html.contains("<li>一</li>"), "{html}");
        assert!(html.contains("<ol"), "{html}");
        assert!(html.contains("<li>乙</li>"), "{html}");
        assert!(html.contains("<blockquote"), "{html}");
        assert!(html.contains("引用"), "{html}");
    }

    #[test]
    fn markdown_fence_and_heading() {
        let html = markdown_to_html("## 标题\n\n```\nlet a = 1 < 2;\n```\n\n---");
        assert!(html.contains("<h2>标题</h2>"), "{html}");
        assert!(html.contains("let a = 1 &lt; 2;"), "{html}");
        assert!(html.contains("<hr>"), "{html}");
    }

    #[test]
    fn markdown_link_and_image() {
        let html = markdown_to_html("[点我](https://a.com) ![图](https://b.com/x.png)");
        assert!(
            html.contains("<a href=\"https://a.com\">点我</a>"),
            "{html}"
        );
        assert!(
            html.contains("<img src=\"https://b.com/x.png\" alt=\"图\">"),
            "{html}"
        );
    }

    #[test]
    fn html_to_markdown_round_trips_common_blocks() {
        let md = "# 标题\n\n这是 **粗** 与 *斜*\n\n- 一\n- 二\n\n> 引用\n\n```\ncode\n```";
        let html = markdown_to_html(md);
        let back = html_to_markdown(&html);
        assert!(back.contains("# 标题"), "{back}");
        assert!(back.contains("**粗**"), "{back}");
        assert!(back.contains("*斜*"), "{back}");
        assert!(back.contains("- 一"), "{back}");
        assert!(back.contains("> 引用"), "{back}");
        assert!(back.contains("```"), "{back}");
    }

    #[test]
    fn html_to_plain_strips_tags_and_keeps_breaks() {
        let plain = html_to_plain("<p>第一段</p><p>第二<br>行 &amp; 尾巴</p>");
        assert!(plain.contains("第一段"), "{plain}");
        assert!(plain.contains("第二"), "{plain}");
        assert!(plain.contains("行 & 尾巴"), "{plain}");
        assert!(!plain.contains('<'), "{plain}");
    }

    #[test]
    fn plain_to_html_keeps_paragraphs() {
        let html = plain_to_html("第一段\n\n第二段\n第二行");
        assert_eq!(html.matches("<p ").count(), 2, "{html}");
        assert!(html.contains("第二段<br>第二行"), "{html}");
    }

    #[test]
    fn convert_between_all_four_formats_keeps_text() {
        let md = "**重点** 内容";
        let html = convert(md, ComposeFormat::Markdown, ComposeFormat::Html);
        assert!(html.contains("<strong>重点</strong>"), "{html}");
        let rich = convert(md, ComposeFormat::Markdown, ComposeFormat::Rich);
        assert_eq!(rich, html);
        let back = convert(&html, ComposeFormat::Rich, ComposeFormat::Markdown);
        assert!(back.contains("**重点**"), "{back}");
        let plain = convert(&html, ComposeFormat::Html, ComposeFormat::Plain);
        assert_eq!(plain, "重点 内容");
        let same = convert(md, ComposeFormat::Markdown, ComposeFormat::Markdown);
        assert_eq!(same, md);
    }

    #[test]
    fn rich_format_renders_toolbar_markup_and_preserves_line_breaks() {
        let html = convert("第一行\n**重点**", ComposeFormat::Rich, ComposeFormat::Html);
        assert!(html.contains("第一行<br><strong>重点</strong>"), "{html}");

        let legacy = "<p><strong>旧草稿</strong></p>";
        assert_eq!(
            convert(legacy, ComposeFormat::Rich, ComposeFormat::Html),
            legacy
        );
        assert_eq!(
            convert("**原样**", ComposeFormat::Plain, ComposeFormat::Plain),
            "**原样**"
        );
    }

    #[test]
    fn looks_like_html_detects_tags_only() {
        assert!(looks_like_html("<p>x</p>"));
        assert!(looks_like_html("  <br>"));
        assert!(!looks_like_html("<3 你"));
        assert!(!looks_like_html("a < b"));
        assert!(!looks_like_html("普通文本"));
    }

    #[test]
    fn entities_decode() {
        assert_eq!(
            decode_entities("a &amp; b &#65; &#x42; &nbsp;c"),
            "a & b A B  c"
        );
        assert_eq!(decode_entities("100% & 完成"), "100% & 完成");
    }

    #[test]
    fn extract_embeddable_html_keeps_head_styles_and_body_content() {
        let raw = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>.btn{color:green;}</style></head><body style=\"margin:0\"><table class=\"btn\"><tr><td>View</td></tr></table></body></html>";
        let out = extract_embeddable_html(raw);
        assert!(out.contains("<style>.btn{color:green;}</style>"), "{out}");
        assert!(
            out.contains("<div style=\"margin:0\"><table class=\"btn\">"),
            "{out}"
        );
        assert!(!out.contains("<!DOCTYPE"), "{out}");
        assert!(!out.contains("<html"), "{out}");
        assert!(!out.contains("<body"), "{out}");
    }

    #[test]
    fn compose_with_quoted_html_preserves_original_html_for_forward_and_reply() {
        let sep = "------------------ 转发邮件 ------------------";
        let body = format!(
            "请查收以下通知\n\n{sep}\n发件人：GitHub <notifications@github.com>\n时间：11-26 20:02\n主题：Run failed\n\n纯文本兜底"
        );
        let orig_html = "<html><head><style>.card{border:1px solid #ccc;}</style></head><body><div class=\"card\"><a href=\"https://github.com\">View workflow run</a></div></body></html>";

        let fwd_html =
            compose_with_quoted_html(&body, ComposeFormat::Rich, sep, orig_html, true, None);
        assert!(fwd_html.contains("请查收以下通知"), "{fwd_html}");
        assert!(fwd_html.contains("kitemail-quote-header"), "{fwd_html}");
        assert!(
            fwd_html.contains("发件人：GitHub &lt;notifications@github.com&gt;"),
            "{fwd_html}"
        );
        assert!(
            fwd_html.contains("<style>.card{border:1px solid #ccc;}</style>"),
            "{fwd_html}"
        );
        assert!(
            fwd_html.contains(
                "<div class=\"card\"><a href=\"https://github.com\">View workflow run</a></div>"
            ),
            "{fwd_html}"
        );
        assert!(!fwd_html.contains("纯文本兜底"), "{fwd_html}");

        let reply_html =
            compose_with_quoted_html(&body, ComposeFormat::Rich, sep, orig_html, false, None);
        assert!(
            reply_html.contains("<blockquote type=\"cite\""),
            "{reply_html}"
        );

        // 若用户删除了分隔线，则退化为普通正文转换。
        let no_sep = compose_with_quoted_html(
            "仅保留自己的话",
            ComposeFormat::Rich,
            sep,
            orig_html,
            true,
            None,
        );
        assert!(no_sep.contains("仅保留自己的话"), "{no_sep}");
        assert!(!no_sep.contains("View workflow run"), "{no_sep}");
    }

    #[test]
    fn compose_with_quoted_html_keeps_trailing_signature() {
        let sep = "------------------ 转发邮件 ------------------";
        let sig = "-- \nAlice";
        let body = format!("请查收\n\n{sep}\n发件人：Bob\n\n原正文\n\n{sig}");
        let out = compose_with_quoted_html(
            &body,
            ComposeFormat::Rich,
            sep,
            "<p>原HTML</p>",
            true,
            Some(sig),
        );
        assert!(out.contains("<p>原HTML</p>"), "{out}");
        assert!(out.contains("kitemail-signature"), "{out}");
        assert!(out.contains("Alice"), "{out}");
    }
}
