//! Best-effort rich rendering of raw Markdown into styled `ratatui` lines:
//! boxed code blocks, bordered aligned tables, underlined headings, nested
//! bullets, callouts, and inline styling. Block-level, not a full CommonMark
//! renderer; output line count does not match the source.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// A heading drawn at a larger size via kitty's text sizing protocol. Its
/// `scale` blank placeholder lines start at `line` in `Preview::lines`.
pub struct ScaledHeading {
    pub line: usize,
    pub scale: u16,
    pub text: String,
    pub style: Style,
}

pub struct Preview {
    pub lines: Vec<Line<'static>>,
    pub scaled: Vec<ScaledHeading>,
}

/// Render raw Markdown `content` for a scrollable `Paragraph`. Colors come
/// from `theme`; with `scale_headings`, H1/H2 become `ScaledHeading` entries.
pub fn render_preview(content: &str, theme: &Theme, scale_headings: bool) -> Preview {
    let src_lines: Vec<&str> = content.lines().collect();
    let defs = scan_link_definitions(&src_lines);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(src_lines.len());
    let mut scaled = Vec::new();
    let mut i = 0;

    while i < src_lines.len() {
        let raw = src_lines[i];
        let trimmed = raw.trim_start();

        if let Some(fence) = fence_marker(trimmed) {
            let lang = trimmed.trim_start_matches(['`', '~']).trim();
            let mut j = i + 1;
            while j < src_lines.len() && !src_lines[j].trim_start().starts_with(fence.as_str()) {
                j += 1;
            }
            lines.extend(render_code_block(lang, &src_lines[i + 1..j], theme));
            i = j + 1;
            continue;
        }

        if let Some((depth, text)) = heading(trimmed) {
            if !lines.is_empty() && !is_blank(lines.last()) && depth <= 2 {
                lines.push(Line::from(""));
            }
            if scale_headings && depth <= 2 {
                let scale = if depth == 1 { 3 } else { 2 };
                scaled.push(ScaledHeading {
                    line: lines.len(),
                    scale,
                    text: plain_text(&text, &defs, theme),
                    style: heading_style(depth, theme),
                });
                lines.extend((0..scale).map(|_| Line::from("")));
            } else {
                lines.extend(render_heading(depth, &text, &defs, theme));
            }
            i += 1;
            continue;
        }

        if is_horizontal_rule(trimmed) {
            lines.push(Line::from(Span::styled(
                "─".repeat(60),
                Style::default().fg(theme.muted),
            )));
            i += 1;
            continue;
        }

        // Invisible in real Markdown.
        if parse_definition_line(trimmed).is_some() {
            i += 1;
            continue;
        }

        if raw.contains('|')
            && src_lines
                .get(i + 1)
                .is_some_and(|next| is_table_separator(next))
        {
            let aligns = parse_alignments(src_lines[i + 1]);
            let mut all_rows = vec![parse_table_cells(raw)];
            let mut j = i + 2;
            while j < src_lines.len() {
                let candidate = src_lines[j];
                if candidate.trim().is_empty()
                    || !candidate.contains('|')
                    || is_table_separator(candidate)
                {
                    break;
                }
                all_rows.push(parse_table_cells(candidate));
                j += 1;
            }

            let widths = table_column_widths(&all_rows, &defs, theme);
            lines.push(render_table_border('┌', '┬', '┐', &widths, theme));
            lines.push(render_table_row(
                &all_rows[0],
                &widths,
                &aligns,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                &defs,
                theme,
            ));
            lines.push(render_table_border('├', '┼', '┤', &widths, theme));
            for row in &all_rows[1..] {
                lines.push(render_table_row(
                    row,
                    &widths,
                    &aligns,
                    Style::default(),
                    &defs,
                    theme,
                ));
            }
            lines.push(render_table_border('└', '┴', '┘', &widths, theme));

            i = j;
            continue;
        }

        if is_table_separator(raw) {
            lines.push(Line::from(Span::styled(
                "─".repeat(raw.trim().len().clamp(8, 80)),
                Style::default().fg(theme.muted),
            )));
            i += 1;
            continue;
        }

        if let Some((level, rest)) = blockquote_prefix(raw) {
            let color = blockquote_color(level, theme);
            let bar = Span::styled("┃ ".repeat(level.max(1)), Style::default().fg(color));
            if let Some((label, icon, callout_color)) = callout_marker(rest, theme) {
                lines.push(Line::from(vec![
                    bar.clone(),
                    Span::styled(
                        format!("{icon} {label}"),
                        Style::default()
                            .fg(callout_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                i += 1;
                continue;
            }
            let mut spans = vec![bar];
            spans.extend(parse_inline(
                rest,
                Style::default().fg(color).add_modifier(Modifier::ITALIC),
                &defs,
                theme,
            ));
            lines.push(Line::from(spans));
            i += 1;
            continue;
        }

        if let Some((indent, marker, checked, rest)) = list_item(raw) {
            let mut spans = vec![Span::raw(" ".repeat(indent))];
            spans.push(Span::styled(marker, Style::default().fg(theme.accent)));
            let mut text_style = Style::default();
            if let Some(is_checked) = checked {
                if is_checked {
                    spans.push(Span::styled("☑ ", Style::default().fg(theme.success)));
                    text_style = text_style.fg(theme.muted);
                } else {
                    spans.push(Span::styled("☐ ", Style::default().fg(theme.selector)));
                }
            }
            spans.extend(parse_inline(rest, text_style, &defs, theme));
            lines.push(Line::from(spans));
            i += 1;
            continue;
        }

        if raw.trim().is_empty() {
            lines.push(Line::from(""));
            i += 1;
            continue;
        }

        lines.push(Line::from(parse_inline(
            raw,
            Style::default(),
            &defs,
            theme,
        )));
        i += 1;
    }

    Preview { lines, scaled }
}

/// Inline markup stripped, control characters removed.
fn plain_text(text: &str, defs: &HashMap<String, String>, theme: &Theme) -> String {
    parse_inline(text, Style::default(), defs, theme)
        .iter()
        .flat_map(|s| s.content.chars())
        .filter(|c| !c.is_control())
        .collect()
}

fn is_blank(line: Option<&Line<'_>>) -> bool {
    line.is_none_or(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

fn render_heading(
    depth: usize,
    text: &str,
    defs: &HashMap<String, String>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let style = heading_style(depth, theme);
    let (prefix, rule) = match depth {
        1 => ("▌ ", Some('═')),
        2 => ("▌ ", Some('─')),
        3 => ("▎ ", None),
        4 => ("▸ ", None),
        _ => ("· ", None),
    };
    let mut spans = vec![Span::styled(prefix, Style::default().fg(theme.accent))];
    spans.extend(parse_inline(text, style, defs, theme));
    let width = spans_width(&spans);
    let mut out = vec![Line::from(spans)];
    if let Some(ch) = rule {
        out.push(Line::from(Span::styled(
            ch.to_string().repeat(width.max(4)),
            Style::default().fg(if depth == 1 {
                theme.header
            } else {
                theme.muted
            }),
        )));
    }
    out
}

fn render_code_block(lang: &str, body: &[&str], theme: &Theme) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme.muted);
    let code = Style::default().fg(theme.code_fg).bg(theme.code_bg);
    let body: Vec<String> = body.iter().map(|l| l.replace('\t', "    ")).collect();
    let head = if lang.is_empty() {
        String::new()
    } else {
        format!("─ {lang} ")
    };
    let head_width = UnicodeWidthStr::width(head.as_str());
    let content_width = body
        .iter()
        .map(|l| UnicodeWidthStr::width(l.as_str()))
        .max()
        .unwrap_or(0)
        .max(20);
    // Inner width: one space of padding on each side of the code.
    let inner = (content_width + 2).max(head_width + 1);

    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(Line::from(Span::styled(
        format!("╭{head}{}╮", "─".repeat(inner - head_width)),
        border,
    )));
    for line in &body {
        let pad = inner - 1 - UnicodeWidthStr::width(line.as_str());
        out.push(Line::from(vec![
            Span::styled("│", border),
            Span::styled(format!(" {line}{}", " ".repeat(pad)), code),
            Span::styled("│", border),
        ]));
    }
    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(inner)),
        border,
    )));
    out
}

/// Recognize GitHub-style alert markers such as `[!NOTE]`.
fn callout_marker(rest: &str, theme: &Theme) -> Option<(&'static str, &'static str, Color)> {
    match rest.trim().to_ascii_uppercase().as_str() {
        "[!NOTE]" => Some(("Note", "ℹ", theme.function)),
        "[!TIP]" => Some(("Tip", "💡", theme.success)),
        "[!IMPORTANT]" => Some(("Important", "❗", theme.keyword)),
        "[!WARNING]" => Some(("Warning", "⚠", theme.accent)),
        "[!CAUTION]" => Some(("Caution", "🛑", Color::Red)),
        _ => None,
    }
}

/// Parse a `[label]: url` reference definition line, if `trimmed` is one.
fn parse_definition_line(trimmed: &str) -> Option<(String, String)> {
    let rest = trimmed.strip_prefix('[')?;
    let close = rest.find(']')?;
    let label = rest[..close].trim().to_string();
    let after = rest[close + 1..].strip_prefix(':')?.trim_start();
    let url_end = after.find(char::is_whitespace).unwrap_or(after.len());
    let url = &after[..url_end];
    if label.is_empty() || url.is_empty() {
        return None;
    }
    Some((label.to_lowercase(), url.to_string()))
}

/// All `[label]: url` definitions in the document, keyed by lowercased label.
fn scan_link_definitions(src_lines: &[&str]) -> HashMap<String, String> {
    src_lines
        .iter()
        .filter_map(|line| parse_definition_line(line.trim_start()))
        .collect()
}

fn fence_marker(trimmed: &str) -> Option<String> {
    if trimmed.starts_with("```") {
        Some("```".to_string())
    } else if trimmed.starts_with("~~~") {
        Some("~~~".to_string())
    } else {
        None
    }
}

fn heading(trimmed: &str) -> Option<(usize, String)> {
    if !trimmed.starts_with('#') {
        return None;
    }
    let depth = trimmed.chars().take_while(|&c| c == '#').count();
    if depth == 0 || depth > 6 {
        return None;
    }
    let rest = &trimmed[depth..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim().to_string();
    Some((depth, text))
}

fn heading_style(depth: usize, theme: &Theme) -> Style {
    let base = Style::default().fg(theme.header);
    match depth {
        1 => base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        2 | 3 => base.add_modifier(Modifier::BOLD),
        _ => base.add_modifier(Modifier::ITALIC),
    }
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let t = trimmed.trim();
    if t.len() < 3 {
        return false;
    }
    let first = t.chars().next().unwrap();
    (first == '-' || first == '*' || first == '_') && t.chars().all(|c| c == first)
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || !t.contains('|') {
        return false;
    }
    t.trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':')
    })
}

fn parse_table_cells(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// Display width after Markdown syntax is stripped (e.g. `**bold**` -> `bold`).
fn rendered_cell_width(cell: &str, defs: &HashMap<String, String>, theme: &Theme) -> usize {
    parse_inline(cell, Style::default(), defs, theme)
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

/// Widest rendered cell per column, across every row of a table block.
fn table_column_widths(
    rows: &[Vec<String>],
    defs: &HashMap<String, String>,
    theme: &Theme,
) -> Vec<usize> {
    let col_count = rows.iter().map(|row| row.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; col_count];
    for row in rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(rendered_cell_width(cell, defs, theme));
        }
    }
    widths
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

fn parse_alignments(separator: &str) -> Vec<Align> {
    separator
        .trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| {
            let cell = cell.trim();
            match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            }
        })
        .collect()
}

fn render_table_row(
    cells: &[String],
    widths: &[usize],
    aligns: &[Align],
    cell_style: Style,
    defs: &HashMap<String, String>,
    theme: &Theme,
) -> Line<'static> {
    let border = Style::default().fg(theme.muted);
    let mut spans = Vec::new();
    for (idx, target) in widths.iter().enumerate() {
        spans.push(Span::styled("│ ", border));
        let cell_spans = cells
            .get(idx)
            .map(|c| parse_inline(c, cell_style, defs, theme))
            .unwrap_or_default();
        let free = target.saturating_sub(spans_width(&cell_spans));
        let (left, right) = match aligns.get(idx).copied().unwrap_or(Align::Left) {
            Align::Left => (0, free),
            Align::Right => (free, 0),
            Align::Center => (free / 2, free - free / 2),
        };
        if left > 0 {
            spans.push(Span::raw(" ".repeat(left)));
        }
        spans.extend(cell_spans);
        if right > 0 {
            spans.push(Span::raw(" ".repeat(right)));
        }
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("│", border));
    Line::from(spans)
}

fn render_table_border(
    left: char,
    mid: char,
    right: char,
    widths: &[usize],
    theme: &Theme,
) -> Line<'static> {
    let mut text = String::new();
    text.push(left);
    for (idx, width) in widths.iter().enumerate() {
        if idx > 0 {
            text.push(mid);
        }
        text.push_str(&"─".repeat(width + 2));
    }
    text.push(right);
    Line::from(Span::styled(text, Style::default().fg(theme.muted)))
}

fn blockquote_prefix(raw: &str) -> Option<(usize, &str)> {
    let mut rest = raw.trim_start();
    if !rest.starts_with('>') {
        return None;
    }
    let mut level = 0;
    while let Some(r) = rest.strip_prefix('>') {
        level += 1;
        rest = r.trim_start();
    }
    Some((level, rest))
}

/// Cycle blockquote color by nesting depth so quoted replies stand out.
fn blockquote_color(level: usize, theme: &Theme) -> Color {
    match level % 3 {
        1 => theme.muted,
        2 => theme.selector,
        _ => theme.keyword,
    }
}

fn list_item(raw: &str) -> Option<(usize, String, Option<bool>, &str)> {
    let indent = raw.len() - raw.trim_start().len();
    let trimmed = raw.trim_start();

    if let Some(rest) = ["- ", "* ", "+ "]
        .iter()
        .find_map(|p| trimmed.strip_prefix(p))
    {
        let (checked, rest) = checkbox(rest);
        let bullet = ["• ", "◦ ", "▪ "][(indent / 2) % 3];
        return Some((indent, bullet.to_string(), checked, rest));
    }

    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let after = &trimmed[digits.len()..];
        if let Some(rest) = [". ", ") "].iter().find_map(|p| after.strip_prefix(p)) {
            let (checked, rest) = checkbox(rest);
            return Some((indent, format!("{}. ", digits), checked, rest));
        }
    }

    None
}

fn checkbox(s: &str) -> (Option<bool>, &str) {
    if let Some(rest) = s.strip_prefix("[ ] ") {
        return (Some(false), rest);
    }
    if let Some(rest) = ["[x] ", "[X] "].iter().find_map(|p| s.strip_prefix(p)) {
        return (Some(true), rest);
    }
    (None, s)
}

/// Render a single line of inline Markdown (emphasis, code spans, links,
/// images, strikethrough) into styled spans, falling back to plain text for
/// anything it doesn't recognize.
fn parse_inline(
    text: &str,
    base: Style,
    defs: &HashMap<String, String>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    while i < len {
        if chars[i] == '!'
            && i + 1 < len
            && chars[i + 1] == '['
            && let Some((alt, _url, consumed)) = parse_link_like(&chars, i + 1)
        {
            flush(&mut buf, &mut spans, base);
            spans.push(Span::styled(format!("🖼 {}", alt), base.fg(theme.keyword)));
            i += 1 + consumed;
            continue;
        }

        if chars[i] == '['
            && let Some((label, url, consumed)) = parse_link_like(&chars, i)
        {
            flush(&mut buf, &mut spans, base);

            if !url.is_empty() {
                spans.push(Span::styled(
                    label,
                    base.fg(theme.function).add_modifier(Modifier::UNDERLINED),
                ));
                i += consumed;
                continue;
            }

            // No inline `(url)`: try reference-style `[text][ref]` / shortcut `[text]`.
            let mut end = i + consumed;
            let mut ref_label = label.clone();
            if end < len
                && chars[end] == '['
                && let Some((r, _, ref_consumed)) = parse_link_like(&chars, end)
            {
                end += ref_consumed;
                if !r.is_empty() {
                    ref_label = r;
                }
            }

            if defs.contains_key(&ref_label.to_lowercase()) {
                spans.push(Span::styled(
                    label,
                    base.fg(theme.function).add_modifier(Modifier::UNDERLINED),
                ));
                i = end;
            } else {
                spans.push(Span::styled(format!("[{label}]"), base));
                i += consumed;
            }
            continue;
        }

        if chars[i] == '`'
            && let Some((code, consumed)) = parse_delim(&chars, i, 1)
        {
            flush(&mut buf, &mut spans, base);
            spans.push(Span::styled(
                format!(" {code} "),
                Style::default().fg(theme.code_fg).bg(theme.code_bg),
            ));
            i += consumed;
            continue;
        }

        if chars[i] == '~'
            && i + 1 < len
            && chars[i + 1] == '~'
            && let Some((inner, consumed)) = parse_delim(&chars, i, 2)
        {
            flush(&mut buf, &mut spans, base);
            spans.extend(parse_inline(
                &inner,
                base.add_modifier(Modifier::CROSSED_OUT),
                defs,
                theme,
            ));
            i += consumed;
            continue;
        }

        if (chars[i] == '*' || chars[i] == '_')
            && i + 1 < len
            && chars[i + 1] == chars[i]
            && let Some((inner, consumed)) = parse_delim(&chars, i, 2)
            && !inner.is_empty()
        {
            flush(&mut buf, &mut spans, base);
            spans.extend(parse_inline(
                &inner,
                base.add_modifier(Modifier::BOLD),
                defs,
                theme,
            ));
            i += consumed;
            continue;
        }

        if (chars[i] == '*' || chars[i] == '_')
            && let Some((inner, consumed)) = parse_delim(&chars, i, 1)
            && !inner.is_empty()
        {
            flush(&mut buf, &mut spans, base);
            spans.extend(parse_inline(
                &inner,
                base.add_modifier(Modifier::ITALIC),
                defs,
                theme,
            ));
            i += consumed;
            continue;
        }

        buf.push(chars[i]);
        i += 1;
    }

    flush(&mut buf, &mut spans, base);
    spans
}

fn flush(buf: &mut String, spans: &mut Vec<Span<'static>>, style: Style) {
    if !buf.is_empty() {
        spans.push(Span::styled(std::mem::take(buf), style));
    }
}

/// Find a closing run of `width` identical delimiter chars starting at `start`,
/// returning the text between the delimiters and the total chars consumed
/// (including both delimiter runs).
fn parse_delim(chars: &[char], start: usize, width: usize) -> Option<(String, usize)> {
    let delim = &chars[start..start + width];
    let mut j = start + width;
    while j + width <= chars.len() {
        if &chars[j..j + width] == delim {
            let inner: String = chars[start + width..j].iter().collect();
            return Some((inner, j + width - start));
        }
        j += 1;
    }
    None
}

/// Parse a `[label](url)` or `[label]` construct starting at `chars[start] == '['`.
/// Returns the label text, the URL (empty if omitted), and total chars consumed.
fn parse_link_like(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut j = start + 1;
    while j < chars.len() && chars[j] != ']' && chars[j] != '\n' {
        j += 1;
    }
    if j >= chars.len() || chars[j] != ']' {
        return None;
    }
    let label: String = chars[start + 1..j].iter().collect();

    if j + 1 < chars.len() && chars[j + 1] == '(' {
        let mut k = j + 2;
        while k < chars.len() && chars[k] != ')' {
            k += 1;
        }
        if k < chars.len() {
            let url: String = chars[j + 2..k].iter().collect();
            return Some((label, url, k + 1 - start));
        }
    }

    Some((label, String::new(), j + 1 - start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_preview(content: &str) -> Vec<Line<'static>> {
        super::render_preview(content, &Theme::dark(), false).lines
    }

    #[test]
    fn test_scaled_headings_reserve_placeholder_lines() {
        let preview = super::render_preview("# A\n## B\n### C", &Theme::dark(), true);
        assert_eq!(preview.scaled.len(), 2);
        assert_eq!((preview.scaled[0].line, preview.scaled[0].scale), (0, 3));
        assert_eq!((preview.scaled[1].line, preview.scaled[1].scale), (3, 2));
        assert_eq!(preview.scaled[0].text, "A");
        assert!(preview.lines[..5].iter().all(|l| line_text(l).is_empty()));
        assert!(line_text(&preview.lines[5]).contains('C'));
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn test_heading_levels() {
        let lines = render_preview("# One\n## Two\n###### Six");
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.contains(&"▌ One".to_string()));
        assert!(texts.contains(&"▌ Two".to_string()));
        assert!(texts.contains(&"· Six".to_string()));
        assert!(texts.iter().any(|t| t.starts_with('═')));
        assert!(
            lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn test_bold_and_italic() {
        let lines = render_preview("**bold** and _italic_ text");
        let text = line_text(&lines[0]);
        assert_eq!(text, "bold and italic text");

        let bold_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "bold")
            .unwrap();
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));

        let italic_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "italic")
            .unwrap();
        assert!(italic_span.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_inline_code() {
        let lines = render_preview("Use `cargo build` to compile");
        let code_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == " cargo build ")
            .unwrap();
        assert_eq!(code_span.style.bg, Some(Theme::dark().code_bg));
    }

    #[test]
    fn test_link_shows_label_only() {
        let lines = render_preview("[mq](https://github.com/harehare/mq)");
        assert_eq!(line_text(&lines[0]), "mq");
    }

    #[test]
    fn test_unordered_list_item() {
        let lines = render_preview("- first item");
        assert_eq!(line_text(&lines[0]), "• first item");
    }

    #[test]
    fn test_ordered_list_item() {
        let lines = render_preview("1. first item");
        assert_eq!(line_text(&lines[0]), "1. first item");
    }

    #[test]
    fn test_task_list_checked() {
        let lines = render_preview("- [x] done\n- [ ] todo");
        assert_eq!(line_text(&lines[0]), "• ☑ done");
        assert_eq!(line_text(&lines[1]), "• ☐ todo");
    }

    #[test]
    fn test_blockquote() {
        let lines = render_preview("> quoted text");
        assert_eq!(line_text(&lines[0]), "┃ quoted text");
    }

    #[test]
    fn test_code_block() {
        let lines = render_preview("```rust\nfn main() {}\n```");
        assert_eq!(lines.len(), 3);
        assert!(line_text(&lines[0]).starts_with("╭─ rust "));
        assert!(line_text(&lines[1]).contains("fn main() {}"));
        assert!(line_text(&lines[2]).starts_with('╰'));
        assert_eq!(lines[1].spans[1].style.bg, Some(Theme::dark().code_bg));
        // Box edges line up.
        let w = |i: usize| UnicodeWidthStr::width(line_text(&lines[i]).as_str());
        assert_eq!(w(0), w(1));
        assert_eq!(w(1), w(2));
    }

    #[test]
    fn test_horizontal_rule() {
        let lines = render_preview("---");
        assert!(line_text(&lines[0]).chars().all(|c| c == '─'));
    }

    #[test]
    fn test_table() {
        let lines = render_preview("| A | B |\n| - | - |\n| 1 | 2 |");
        assert_eq!(lines.len(), 5);
        assert!(line_text(&lines[0]).starts_with('┌'));
        assert_eq!(line_text(&lines[1]), "│ A │ B │");
        assert!(line_text(&lines[2]).starts_with('├'));
        assert_eq!(line_text(&lines[3]), "│ 1 │ 2 │");
        assert!(line_text(&lines[4]).starts_with('└'));
    }

    #[test]
    fn test_table_columns_align_by_rendered_width() {
        let lines =
            render_preview("| Name | Note |\n| - | - |\n| **bold** | x |\n| a | much longer |");
        let widths: Vec<usize> = lines
            .iter()
            .map(|l| UnicodeWidthStr::width(line_text(l).as_str()))
            .collect();
        assert!(widths.iter().all(|w| *w == widths[0]));
    }

    #[test]
    fn test_table_alignment() {
        let lines = render_preview("| A |\n| --: |\n| 1 |\n| long |");
        assert_eq!(line_text(&lines[3]), "│    1 │");
    }

    #[test]
    fn test_callout() {
        let lines = render_preview("> [!NOTE]\n> body");
        assert_eq!(line_text(&lines[0]), "┃ ℹ Note");
    }

    #[test]
    fn test_nested_list_uses_different_bullet() {
        let lines = render_preview("- a\n  - b");
        assert_eq!(line_text(&lines[1]), "  ◦ b");
    }

    #[test]
    fn test_strikethrough() {
        let lines = render_preview("~~removed~~");
        let span = &lines[0].spans[0];
        assert_eq!(span.content.as_ref(), "removed");
        assert!(span.style.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn test_image_shows_alt() {
        let lines = render_preview("![alt text](image.png)");
        assert_eq!(line_text(&lines[0]), "🖼 alt text");
    }

    #[test]
    fn test_empty_lines_preserved() {
        let lines = render_preview("a\n\nb");
        assert_eq!(lines.len(), 3);
        assert_eq!(line_text(&lines[1]), "");
    }

    #[test]
    fn test_reference_style_link_resolves_to_definition() {
        let lines = render_preview("See [mq][ref] for details.\n\n[ref]: https://example.com");
        assert_eq!(line_text(&lines[0]), "See mq for details.");
        let span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "mq")
            .unwrap();
        assert!(span.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn test_shortcut_reference_link_resolves_to_definition() {
        let lines = render_preview("See [mq] for details.\n\n[mq]: https://example.com");
        let span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "mq")
            .unwrap();
        assert!(span.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn test_undefined_bracket_renders_as_literal_text() {
        let lines = render_preview("This is [not a link] in prose.");
        assert_eq!(line_text(&lines[0]), "This is [not a link] in prose.");
    }

    #[test]
    fn test_definition_line_is_hidden() {
        let lines = render_preview("a\n[ref]: https://example.com\nb");
        assert_eq!(lines.len(), 2);
    }
}
