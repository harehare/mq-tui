//! Best-effort rendering of raw Markdown source into styled `ratatui` lines,
//! so the TUI can show something close to a rendered preview without needing
//! a full CommonMark renderer. Operates line-by-line on the source text
//! (preserving the user's original layout) and applies simple inline styling
//! for emphasis, code spans, links, and images.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Render raw Markdown `content` into a list of styled lines suitable for
/// display in a scrollable `Paragraph`. Colors are drawn from `theme` so the
/// preview stays readable under both the dark and light color schemes.
pub fn render_preview(content: &str, theme: &Theme) -> Vec<Line<'static>> {
    let src_lines: Vec<&str> = content.lines().collect();
    let defs = scan_link_definitions(&src_lines);
    let mut lines = Vec::with_capacity(src_lines.len());
    let mut in_code_block = false;
    let mut code_fence = String::new();
    let mut i = 0;

    while i < src_lines.len() {
        let raw = src_lines[i];
        let trimmed = raw.trim_start();

        if in_code_block {
            if trimmed.starts_with(code_fence.as_str()) {
                in_code_block = false;
                lines.push(Line::from(Span::styled(
                    code_fence.clone(),
                    Style::default().fg(theme.muted),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    raw.to_string(),
                    Style::default().fg(theme.string_lit),
                )));
            }
            i += 1;
            continue;
        }

        if let Some(fence) = fence_marker(trimmed) {
            in_code_block = true;
            let lang = trimmed.trim_start_matches(['`', '~']).trim().to_string();
            code_fence = fence;
            let title = if lang.is_empty() {
                code_fence.clone()
            } else {
                format!("{} {}", code_fence, lang)
            };
            lines.push(Line::from(Span::styled(
                title,
                Style::default().fg(theme.muted),
            )));
            i += 1;
            continue;
        }

        if let Some((depth, text)) = heading(trimmed) {
            lines.push(Line::from(Span::styled(text, heading_style(depth, theme))));
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

        // Invisible in real Markdown; dimmed here to keep line counts in sync.
        if parse_definition_line(trimmed).is_some() {
            lines.push(Line::from(Span::styled(
                raw.to_string(),
                Style::default()
                    .fg(theme.muted)
                    .add_modifier(Modifier::ITALIC),
            )));
            i += 1;
            continue;
        }

        // Table header + separator: render the whole block so columns align.
        if raw.contains('|')
            && src_lines
                .get(i + 1)
                .is_some_and(|next| is_table_separator(next))
        {
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
            lines.push(render_table_row(
                &all_rows[0],
                &widths,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                &defs,
                theme,
            ));
            lines.push(render_table_separator(&widths, theme));
            for row in &all_rows[1..] {
                lines.push(render_table_row(
                    row,
                    &widths,
                    Style::default(),
                    &defs,
                    theme,
                ));
            }

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

        if raw.contains('|') && raw.trim().starts_with('|') {
            lines.push(Line::from(table_row_spans(
                raw,
                Style::default(),
                &defs,
                theme,
            )));
            i += 1;
            continue;
        }

        if let Some((level, rest)) = blockquote_prefix(raw) {
            let color = blockquote_color(level, theme);
            let mut spans = vec![Span::styled(
                "┃ ".repeat(level.max(1)),
                Style::default().fg(color),
            )];
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
            if let Some(is_checked) = checked {
                spans.push(Span::styled(
                    if is_checked { "☑ " } else { "☐ " },
                    Style::default().fg(theme.selector),
                ));
            }
            spans.extend(parse_inline(rest, Style::default(), &defs, theme));
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

    lines
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

fn table_row_spans(
    raw: &str,
    cell_style: Style,
    defs: &HashMap<String, String>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let cells: Vec<&str> = raw.trim().trim_matches('|').split('|').collect();
    let mut spans = Vec::new();
    for (idx, cell) in cells.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(theme.muted)));
        }
        spans.extend(parse_inline(cell.trim(), cell_style, defs, theme));
    }
    spans
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

fn render_table_row(
    cells: &[String],
    widths: &[usize],
    cell_style: Style,
    defs: &HashMap<String, String>,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (idx, cell) in cells.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(theme.muted)));
        }
        let cell_spans = parse_inline(cell, cell_style, defs, theme);
        let rendered_width: usize = cell_spans
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum();
        spans.extend(cell_spans);
        let target = widths.get(idx).copied().unwrap_or(0);
        if rendered_width < target {
            spans.push(Span::styled(
                " ".repeat(target - rendered_width),
                cell_style,
            ));
        }
    }
    Line::from(spans)
}

fn render_table_separator(widths: &[usize], theme: &Theme) -> Line<'static> {
    let sep_style = Style::default().fg(theme.muted);
    let mut spans = Vec::new();
    for (idx, width) in widths.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled("─┼─", sep_style));
        }
        spans.push(Span::styled("─".repeat((*width).max(1)), sep_style));
    }
    Line::from(spans)
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
        return Some((indent, "• ".to_string(), checked, rest));
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
                code,
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
        super::render_preview(content, &Theme::dark())
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn test_heading_levels() {
        let lines = render_preview("# One\n## Two\n###### Six");
        assert_eq!(line_text(&lines[0]), "One");
        assert_eq!(line_text(&lines[1]), "Two");
        assert_eq!(line_text(&lines[2]), "Six");
        assert!(
            lines[0].spans[0]
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
            .find(|s| s.content.as_ref() == "cargo build")
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
        assert_eq!(line_text(&lines[1]), "fn main() {}");
        assert_eq!(lines[1].spans[0].style.fg, Some(Theme::dark().string_lit));
    }

    #[test]
    fn test_horizontal_rule() {
        let lines = render_preview("---");
        assert!(line_text(&lines[0]).chars().all(|c| c == '─'));
    }

    #[test]
    fn test_table() {
        let lines = render_preview("| A | B |\n| - | - |\n| 1 | 2 |");
        assert_eq!(line_text(&lines[0]), "A │ B");
        assert_eq!(line_text(&lines[2]), "1 │ 2");
    }

    #[test]
    fn test_table_columns_align_by_rendered_width() {
        let lines =
            render_preview("| Name | Note |\n| - | - |\n| **bold** | x |\n| a | much longer |");
        // Row 1 is the separator (uses '┼', not '│'); compare the data rows.
        let col_pos = |idx: usize| line_text(&lines[idx]).find('│').unwrap();
        assert_eq!(col_pos(0), col_pos(2));
        assert_eq!(col_pos(0), col_pos(3));
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
    fn test_definition_line_rendered_dimmed_and_preserves_line_count() {
        let lines = render_preview("a\n[ref]: https://example.com\nb");
        assert_eq!(lines.len(), 3);
        assert!(line_text(&lines[1]).contains("https://example.com"));
    }

    #[test]
    fn test_preview_line_count_matches_source_line_count() {
        let content = "# Title\n\nSome *text* with a [link](url).\n\n- item\n\n> quote\n";
        let lines = render_preview(content);
        assert_eq!(lines.len(), content.lines().count());
    }
}
