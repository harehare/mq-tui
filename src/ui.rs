pub mod preview;
pub mod treeview;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Padding, Paragraph,
        Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
};

use crate::app::{App, Mode};
use crate::theme::Theme;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Break `line` into chunks of at most `width` display columns, preserving
/// all original characters/spacing exactly (no word-boundary reflow), so
/// list rows stay fully visible instead of being clipped.
pub(crate) fn wrap_to_width(line: &str, width: usize) -> Vec<String> {
    if width == 0 || line.is_empty() {
        return vec![line.to_string()];
    }

    let mut rows = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for ch in line.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > width && !current.is_empty() {
            rows.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    rows.push(current);
    rows
}

pub fn draw_ui(frame: &mut Frame, app: &App) {
    let theme = app.theme();
    let show_tabs = app.document_count() > 1;

    let mut constraints = Vec::with_capacity(4);
    if show_tabs {
        constraints.push(Constraint::Length(3)); // Tab bar
    }
    constraints.push(Constraint::Length(3)); // Query input / title bar
    constraints.push(Constraint::Min(0)); // Results area
    constraints.push(Constraint::Length(1)); // Status line (+ key hints)

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(frame.area());

    let mut next_chunk = 0;
    if show_tabs {
        draw_tab_bar(frame, app, chunks[next_chunk]);
        next_chunk += 1;
    }

    let header_area = chunks[next_chunk];
    next_chunk += 1;
    let results_area = chunks[next_chunk];
    next_chunk += 1;
    let status_area = chunks[next_chunk];

    match app.mode() {
        Mode::Query => draw_query_input(frame, app, header_area),
        Mode::OpenFile => draw_open_file_input(frame, app, header_area),
        Mode::Search => draw_search_input(frame, app, header_area),
        Mode::SaveQuery => draw_save_query_input(frame, app, header_area),
        _ => draw_title_bar(frame, app, header_area),
    }

    // While searching or typing a query, keep showing the view that mode
    // was entered from (e.g. stay on the tree/preview so it updates live).
    let display_mode = if app.mode() == Mode::Search {
        app.search_return_mode()
    } else if app.mode() == Mode::Query {
        app.query_return_mode()
    } else {
        app.mode()
    };

    match display_mode {
        Mode::TreeView => {
            if let Some(tree_view) = app.tree_view() {
                let crumbs = tree_view.breadcrumb();
                let title = if crumbs.is_empty() {
                    "Document Tree".to_string()
                } else {
                    format!("Document Tree - {}", crumbs.join(" \u{203a} "))
                };
                tree_view.render_with_title(frame, results_area, &title, &theme);
            }
        }
        Mode::Preview => {
            draw_preview(frame, app, results_area);
        }
        Mode::Favorites => {
            draw_favorites_list(frame, app, results_area);
        }
        _ => {
            // Show sidebar if enabled
            if app.show_tree_sidebar() && app.sidebar_tree_view().is_some() {
                let sidebar = app.sidebar_split_percent();
                let main_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(sidebar),       // Sidebar
                        Constraint::Percentage(100 - sidebar), // Main content
                    ])
                    .split(results_area);

                // Draw sidebar
                if let Some(sidebar) = app.sidebar_tree_view() {
                    sidebar.render_with_title(frame, main_chunks[0], "Headers", &theme);
                }

                // Draw main content area (results and/or detail)
                if app.show_detail() && !app.results().is_empty() {
                    let left = app.detail_split_percent();
                    let detail_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(left),       // Results list
                            Constraint::Percentage(100 - left), // Detail view
                        ])
                        .split(main_chunks[1]);

                    draw_results_list(frame, app, detail_chunks[0]);
                    draw_detail_view(frame, app, detail_chunks[1]);
                } else {
                    draw_results_list(frame, app, main_chunks[1]);
                }
            } else {
                // No sidebar - original layout
                if app.show_detail() && !app.results().is_empty() {
                    let left = app.detail_split_percent();
                    let detail_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Percentage(left),       // Results list
                            Constraint::Percentage(100 - left), // Detail view
                        ])
                        .split(results_area);

                    draw_results_list(frame, app, detail_chunks[0]);
                    draw_detail_view(frame, app, detail_chunks[1]);
                } else {
                    draw_results_list(frame, app, results_area);
                }
            }
        }
    }

    draw_status_line(frame, app, status_area);

    // Query mode shows errors inline (in the query box) instead of a
    // blocking popup, since they re-fire on every keystroke while typing.
    if app.mode() != Mode::Query
        && let Some(error) = app.error_msg()
    {
        draw_error_popup(frame, error);
    }

    if app.mode() == Mode::Query {
        draw_completions_popup(frame, app, header_area);
    }

    if app.mode() == Mode::Help {
        draw_help_screen(frame, app);
    }

    if app.mode() == Mode::CommandPalette {
        draw_command_palette(frame, app);
    }
}

/// Mode-specific key hints folded into the left side of the status line
/// (toggle with `show_hint_bar` in config.toml, or `--no-hints`).
fn hint_text(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => {
            ": query  t tree  p preview  s sidebar  d detail  y/Y copy  / search  Ctrl+K palette  ? help"
        }
        Mode::Query => "Enter run  Esc cancel  \u{2191}/\u{2193} history  Tab complete",
        Mode::TreeView => "j/k move  Enter/Space expand  / search  Esc exit",
        Mode::Preview => "j/k scroll  s split  </> resize  g/G top/bottom  Esc exit",
        Mode::Search => "Enter confirm  Esc cancel",
        Mode::SaveQuery => "Enter save  Esc cancel",
        Mode::Favorites => "Enter run  d delete  Esc close",
        Mode::CommandPalette => "type to filter  \u{2191}/\u{2193} select  Enter run  Esc close",
        Mode::OpenFile => "Enter open  Esc cancel",
        Mode::Help => "any key to close",
    }
}

/// Command palette overlay: a fuzzy-filterable list of app actions (Ctrl+K).
fn draw_command_palette(frame: &mut Frame, app: &App) {
    let theme = app.theme();
    let frame_area = frame.area();
    let width = frame_area.width.clamp(30, 70).min(frame_area.width);
    let height = frame_area.height.clamp(8, 16).min(frame_area.height);
    let x = (frame_area.width.saturating_sub(width)) / 2;
    let y = (frame_area.height.saturating_sub(height)) / 3;
    let area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title("Command Palette")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let query_line = Paragraph::new(Line::from(vec![
        Span::styled("> ", Style::default().fg(theme.accent)),
        Span::styled(app.palette_query(), Style::default().fg(theme.accent)),
    ]));
    frame.render_widget(query_line, chunks[0]);
    frame.set_cursor_position(Position::new(
        chunks[0].x + 2 + app.palette_cursor() as u16,
        chunks[0].y,
    ));

    let matches = app.filtered_palette_actions();
    let selected = app.palette_selected();

    if matches.is_empty() {
        let empty = Paragraph::new("No matching command").style(Style::default().fg(theme.muted));
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let items: Vec<ListItem> = matches
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let style = if i == selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.selector)
            };
            let desc_style = if i == selected {
                style
            } else {
                Style::default().fg(theme.muted)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<20}", action.label), style),
                Span::styled(action.description, desc_style),
            ]))
        })
        .collect();

    let list = List::new(items);
    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(list, chunks[1], &mut state);
}

/// Draw the tab bar showing all open documents, with the active one highlighted.
fn draw_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme();
    let titles: Vec<Line> = app
        .document_names()
        .into_iter()
        .map(|name| Line::from(Span::raw(name.to_string())))
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Files (←/→, Tab/Shift+Tab to switch)"),
        )
        .select(app.active_doc_index())
        .highlight_style(
            Style::default()
                .fg(theme.selection_fg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::raw(" | "));

    frame.render_widget(tabs, area);
}

/// Draw the path input box used in Mode::OpenFile
fn draw_open_file_input(frame: &mut Frame, app: &App, area: Rect) {
    let open_file_block = Block::default()
        .title("Open File (Enter to confirm, Esc to cancel)")
        .borders(Borders::ALL)
        .style(Style::default());

    let open_file_text = Paragraph::new(app.open_file_path())
        .style(Style::default().fg(app.theme().accent))
        .block(open_file_block);

    frame.render_widget(open_file_text, area);

    let cursor_x = app.open_file_cursor() as u16 + 1; // +1 for block border
    frame.set_cursor_position(Position::new(
        area.x + cursor_x,
        area.y + 1, // +1 for block border
    ));
}

fn draw_search_input(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title("Search (Enter to confirm, Esc to cancel)")
        .borders(Borders::ALL);

    let text = Paragraph::new(app.search_query())
        .style(Style::default().fg(app.theme().accent))
        .block(block);

    frame.render_widget(text, area);

    let cursor_x = app.search_cursor() as u16 + 1;
    frame.set_cursor_position(Position::new(area.x + cursor_x, area.y + 1));
}

fn draw_save_query_input(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(
            "Save query as (Enter to confirm, Esc to cancel) - {}",
            app.query()
        ))
        .borders(Borders::ALL);

    let text = Paragraph::new(app.save_query_name())
        .style(Style::default().fg(app.theme().accent))
        .block(block);

    frame.render_widget(text, area);

    let cursor_x = app.save_query_cursor() as u16 + 1;
    frame.set_cursor_position(Position::new(area.x + cursor_x, area.y + 1));
}

fn draw_favorites_list(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme();
    let saved = app.saved_queries();
    let block = Block::default()
        .title("Favorites (Enter to run, d to delete, Esc to close)")
        .borders(Borders::ALL);

    if saved.is_empty() {
        let text = Paragraph::new("No saved queries yet - press 'S' to save the current query")
            .style(Style::default().fg(theme.muted))
            .block(block);
        frame.render_widget(text, area);
        return;
    }

    let wrap_width = area.width.saturating_sub(2).max(1) as usize;

    let items: Vec<ListItem> = saved
        .iter()
        .enumerate()
        .map(|(i, saved)| {
            let is_selected = i == app.favorites_selected();
            let style = if is_selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else {
                Style::default()
            };
            let name_style = style.add_modifier(Modifier::BOLD).fg(if is_selected {
                theme.selection_fg
            } else {
                theme.selector
            });
            let query_style = style.fg(if is_selected {
                theme.selection_fg
            } else {
                theme.muted
            });

            let text = format!("{}  {}", saved.name, saved.query);
            let name_len = saved.name.chars().count();
            let lines: Vec<Line> = wrap_to_width(&text, wrap_width)
                .iter()
                .enumerate()
                .map(|(seg_i, segment)| {
                    if seg_i == 0 {
                        let chars: Vec<char> = segment.chars().collect();
                        let split_at = name_len.min(chars.len());
                        let name_part: String = chars[..split_at].iter().collect();
                        let rest_part: String = chars[split_at..].iter().collect();
                        Line::from(vec![
                            Span::styled(name_part, name_style),
                            Span::styled(rest_part, query_style),
                        ])
                    } else {
                        Line::from(Span::styled(segment.clone(), query_style))
                    }
                })
                .collect();

            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items).block(block);
    let mut state = ListState::default();
    state.select(Some(app.favorites_selected()));
    frame.render_stateful_widget(list, area, &mut state);
}

const QUERY_KEYWORDS: &[&str] = &[
    "and", "as", "break", "catch", "continue", "def", "do", "elif", "else", "end", "fn", "foreach",
    "if", "import", "include", "let", "loop", "match", "module", "nodes", "none", "not", "or",
    "self", "true", "false", "try", "unless", "until", "var", "while",
];

/// Best-effort tokenizer for coloring the query bar as the user types.
/// Not a real lexer (mq-lang's isn't public) — just close enough for display.
fn highlight_query(query: &str, theme: &Theme) -> Vec<Span<'static>> {
    let chars: Vec<char> = query.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;

    let selector_style = Style::default()
        .fg(theme.selector)
        .add_modifier(Modifier::BOLD);
    let keyword_style = Style::default()
        .fg(theme.keyword)
        .add_modifier(Modifier::BOLD);
    let string_style = Style::default().fg(theme.string_lit);
    let number_style = Style::default().fg(theme.number_lit);
    let function_style = Style::default().fg(theme.function);
    let pipe_style = Style::default().fg(theme.pipe).add_modifier(Modifier::BOLD);
    let comment_style = Style::default().fg(theme.comment);
    let default_style = Style::default().fg(theme.accent);

    while i < chars.len() {
        let c = chars[i];

        if c == '#' {
            spans.push(Span::styled(
                chars[i..].iter().collect::<String>(),
                comment_style,
            ));
            break;
        }

        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                let closing = chars[i] == '"';
                i += 1;
                if closing {
                    break;
                }
            }
            spans.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                string_style,
            ));
            continue;
        }

        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            spans.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                number_style,
            ));
            continue;
        }

        if c == '.' {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '*' | '>' | '^' | '<'))
            {
                i += 1;
            }
            spans.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                selector_style,
            ));
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let style = if QUERY_KEYWORDS.contains(&word.to_lowercase().as_str()) {
                keyword_style
            } else if chars.get(i) == Some(&'(') {
                function_style
            } else {
                default_style
            };
            spans.push(Span::styled(word, style));
            continue;
        }

        if c == '|' {
            spans.push(Span::styled("|".to_string(), pipe_style));
            i += 1;
            continue;
        }

        let start = i;
        i += 1;
        while i < chars.len()
            && !matches!(chars[i], '#' | '"' | '.' | '|')
            && !chars[i].is_ascii_digit()
            && !chars[i].is_alphabetic()
            && chars[i] != '_'
        {
            i += 1;
        }
        spans.push(Span::styled(
            chars[start..i].iter().collect::<String>(),
            default_style,
        ));
    }

    spans
}

fn draw_query_input(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme();
    let has_error = app.error_msg().is_some();
    let mut query_block = Block::default().title("Query").borders(Borders::ALL);

    if let Some(error) = app.error_msg() {
        query_block = query_block.title_bottom(
            Line::from(Span::styled(
                error.to_string(),
                Style::default().fg(Color::Red),
            ))
            .alignment(Alignment::Left),
        );
    }

    // A broken query turns fully red/underlined; otherwise it's syntax-highlighted.
    let mut spans = if has_error {
        vec![Span::styled(
            app.query().to_string(),
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::UNDERLINED),
        )]
    } else {
        highlight_query(app.query(), &theme)
    };

    // Inline "ghost text": the remainder of the top completion candidate,
    // shown dimmed after the cursor (fish-shell style autosuggestion).
    if !has_error && let Some(suffix) = app.ghost_suffix() {
        spans.push(Span::styled(
            suffix,
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ));
    }

    let query_text = Paragraph::new(Line::from(spans)).block(query_block);

    frame.render_widget(query_text, area);

    let cursor_x = app.cursor_position() as u16 + 1; // +1 for block border
    frame.set_cursor_position(Position::new(
        area.x + cursor_x,
        area.y + 1, // +1 for block border
    ));
}

/// Completion suggestions popup under the query box; Tab accepts the first entry.
fn draw_completions_popup(frame: &mut Frame, app: &App, header_area: Rect) {
    let theme = app.theme();
    let completions = app.active_completions();
    if completions.is_empty() {
        return;
    }
    let selected = app.completion_selected_index();

    let frame_area = frame.area();
    let height = (completions.len() as u16 + 2).min(10);
    let y = header_area.y + header_area.height;
    if y >= frame_area.height {
        return;
    }
    let height = height.min(frame_area.height - y);
    let area = Rect::new(header_area.x, y, header_area.width, height);

    frame.render_widget(Clear, area);

    let prefix = app.completion_prefix();
    let items: Vec<ListItem> = completions
        .iter()
        .enumerate()
        .map(|(i, (name, description))| {
            let style = if i == selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.selector)
            };

            // Bold the prefix the user actually typed within the candidate
            // name, so it's obvious at a glance why each entry matched.
            let mut name_spans = if i != selected && !prefix.is_empty() && name.starts_with(prefix)
            {
                let (matched, rest) = name.split_at(prefix.len());
                vec![
                    Span::raw(" "),
                    Span::styled(
                        matched.to_string(),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(rest.to_string(), style),
                    Span::raw(" "),
                ]
            } else {
                vec![Span::styled(format!(" {name} "), style)]
            };
            name_spans.push(Span::styled(
                format!(" {description}"),
                Style::default().fg(theme.muted),
            ));

            ListItem::new(Line::from(name_spans))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Tab/Shift+Tab to cycle"),
    );

    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_results_list(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme();
    let results = app.results();

    let results_block = Block::default().title("Results").borders(Borders::ALL);

    if results.is_empty() {
        let text = if app.query().is_empty() {
            "Enter a query to filter results"
        } else {
            "No results found"
        };

        let empty_text = Paragraph::new(text)
            .style(Style::default().fg(theme.muted))
            .block(results_block);

        frame.render_widget(empty_text, area);
        return;
    }

    // Use the combined render for display so that blank lines between nodes are
    // preserved exactly as they appear in the source (render_with_theme uses
    // source positions to decide inter-node spacing).
    //
    // To compute the correct scroll position (selected_line), render the prefix
    // nodes[0..selected_idx+1] and subtract the selected node's own line count.
    // This correctly accounts for variable inter-node spacing without reimplementing
    // the position-aware rendering logic.
    let selected_idx = app.selected_idx();
    let selected_content_lines = mq_markdown::Markdown::new(vec![results[selected_idx].clone()])
        .to_string()
        .lines()
        .count()
        .max(1);

    let selected_line = if selected_idx == 0 {
        0
    } else {
        mq_markdown::Markdown::new(results[..selected_idx + 1].to_vec())
            .to_string()
            .lines()
            .count()
            .saturating_sub(selected_content_lines)
    };

    let selected_end_line = selected_line + selected_content_lines;

    let search_term = app.active_search_term().filter(|t| !t.is_empty());
    let wrap_width = area.width.saturating_sub(2).max(1) as usize;

    let items: Vec<ListItem> = mq_markdown::Markdown::new(results.to_vec())
        .to_string()
        .lines()
        .enumerate()
        .map(|(i, value)| {
            let is_selected = i >= selected_line && i < selected_end_line;
            let base_style = if is_markdown_header(value) {
                Style::default()
                    .fg(theme.header)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let lines: Vec<Line> = wrap_to_width(value, wrap_width)
                .iter()
                .map(|segment| {
                    Line::from(highlight_matches(segment, search_term, base_style, &theme))
                })
                .collect();

            ListItem::new(lines).style(if is_selected {
                Style::default()
                    .fg(theme.selection_fg)
                    .bg(theme.selection_bg)
            } else {
                Style::default()
            })
        })
        .collect();

    let list = List::new(items)
        .block(results_block)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut state = ListState::default();
    state.select(Some(selected_line));

    frame.render_stateful_widget(list, area, &mut state);
}

/// Draw a best-effort rendered preview of the active document's full source.
fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    app.set_preview_viewport_height(area.height);

    let theme = app.theme();
    let lines = preview::render_preview(app.active_doc_content(), &theme);
    let total_lines = lines.len();
    let inner_height = area.height.saturating_sub(2).max(1);
    let max_scroll = total_lines.saturating_sub(inner_height as usize) as u16;
    let scroll = app.preview_scroll().min(max_scroll);

    let hint = if app.preview_split() {
        "s to unsplit"
    } else {
        "s to split with source"
    };
    let percent = if max_scroll == 0 {
        100
    } else {
        (scroll as u32 * 100 / max_scroll as u32).min(100)
    };
    let title = format!(
        "Preview - {} [{percent}%] (j/k scroll, u/d ½-page, b/f page, g/G top/bottom, {hint}, p/Esc to exit)",
        app.filename().unwrap_or("untitled")
    );

    let preview_area = if app.preview_split() {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let source_lines: Vec<Line> = app
            .active_doc_content()
            .lines()
            .map(|l| Line::from(l.to_string()))
            .collect();
        let source_block = Block::default().title("Source").borders(Borders::ALL);
        let source_paragraph = Paragraph::new(source_lines)
            .block(source_block)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(source_paragraph, chunks[0]);

        chunks[1]
    } else {
        area
    };

    let block = Block::default().title(title).borders(Borders::ALL);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, preview_area);

    if total_lines > inner_height as usize {
        let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            preview_area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

/// Style every case-insensitive occurrence of `term` in `line` distinctly.
fn highlight_matches(
    line: &str,
    term: Option<&str>,
    base_style: Style,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let Some(term) = term else {
        return vec![Span::styled(line.to_string(), base_style)];
    };
    let line_lower = line.to_lowercase();
    let term_lower = term.to_lowercase();
    let mut spans = Vec::new();
    let mut pos = 0;

    while let Some(offset) = line_lower[pos..].find(&term_lower) {
        let match_start = pos + offset;
        let match_end = match_start + term.len();
        if match_start > pos {
            spans.push(Span::styled(line[pos..match_start].to_string(), base_style));
        }
        spans.push(Span::styled(
            line[match_start..match_end].to_string(),
            base_style
                .fg(theme.match_fg)
                .bg(theme.match_bg)
                .add_modifier(Modifier::BOLD),
        ));
        pos = match_end;
    }
    if pos < line.len() {
        spans.push(Span::styled(line[pos..].to_string(), base_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(line.to_string(), base_style));
    }

    spans
}

/// Check if a line is a markdown header (starts with #)
fn is_markdown_header(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#') && trimmed.chars().nth(1).is_some_and(|c| c == ' ' || c == '#')
}

/// Draw the status line at the bottom, folding in the mode-specific key
/// hints (when enabled) so the bottom bar stays a single line.
fn draw_status_line(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme();
    let exec_time = app.last_exec_time();
    let results_count = app.results().len();

    let doc_info = if app.document_count() > 1 {
        format!("[{}/{}] ", app.active_doc_index() + 1, app.document_count())
    } else {
        String::new()
    };

    let watch_info = if app.watch() { "👀 watching | " } else { "" };

    let show_hints = app.show_hint_bar();
    let status = format!(
        "{}{}{} results | {:.2}ms{}",
        watch_info,
        doc_info,
        results_count,
        exec_time.as_secs_f64() * 1000.0,
        if show_hints { "" } else { " | Press q to quit" }
    );

    if !show_hints {
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(theme.muted)),
            area,
        );
        return;
    }

    let display_mode = if app.mode() == Mode::Search {
        app.search_return_mode()
    } else {
        app.mode()
    };
    let status_width = status.width() as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(status_width)])
        .split(area);

    frame.render_widget(
        Paragraph::new(hint_text(display_mode)).style(Style::default().fg(theme.muted)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(status)
            .style(Style::default().fg(theme.muted))
            .alignment(Alignment::Right),
        chunks[1],
    );
}

fn draw_title_bar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme();
    let title = app.filename().unwrap_or("None");
    let title_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);

    let title_spans = vec![
        Span::styled(title, Style::default().fg(theme.success).bold()),
        Span::raw(" | "),
        Span::styled(
            app.mode().to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::styled(
            "Press 's' for sidebar, 't' for tree view, 'p' for preview, 'o' to open a file, '?' for help",
            Style::default().fg(theme.muted),
        ),
    ];

    let title_text = Paragraph::new(Line::from(title_spans))
        .block(title_block)
        .alignment(Alignment::Center);

    frame.render_widget(title_text, area);
}

fn draw_detail_view(frame: &mut Frame, app: &App, area: Rect) {
    let results = app.results();
    if results.is_empty() || app.selected_idx() >= results.len() {
        return;
    }

    let selected_item = &results[app.selected_idx()];
    let title = format!(
        "Detail View - {}/{} \u{203a} {}",
        app.selected_idx() + 1,
        results.len(),
        node_type_label(selected_item)
    );
    let detail_block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .padding(Padding::new(1, 1, 1, 1));

    let detailed_content = format!("{:#?}", selected_item);

    let detail_text = Paragraph::new(detailed_content)
        .style(Style::default())
        .block(detail_block)
        .wrap(Wrap { trim: false });

    frame.render_widget(detail_text, area);
}

/// Short variant name for a node, used in the detail view's breadcrumb title.
fn node_type_label(node: &mq_markdown::Node) -> &'static str {
    use mq_markdown::Node;
    match node {
        Node::Heading(_) => "Heading",
        Node::List(_) => "List",
        Node::Code(_) => "Code Block",
        Node::CodeInline(_) => "Inline Code",
        Node::Blockquote(_) => "Blockquote",
        Node::Strong(_) => "Strong",
        Node::Emphasis(_) => "Emphasis",
        Node::Delete(_) => "Strikethrough",
        Node::Link(_) => "Link",
        Node::LinkRef(_) => "Link Ref",
        Node::Image(_) => "Image",
        Node::ImageRef(_) => "Image Ref",
        Node::Text(_) => "Text",
        Node::HorizontalRule(_) => "Horizontal Rule",
        Node::TableAlign(_) => "Table Header",
        Node::TableRow(_) => "Table Row",
        Node::TableCell(_) => "Table Cell",
        Node::Break(_) => "Line Break",
        Node::Html(_) => "HTML",
        Node::Math(_) => "Math",
        Node::MathInline(_) => "Inline Math",
        Node::Yaml(_) => "YAML",
        Node::Toml(_) => "TOML",
        Node::Fragment(_) => "Fragment",
        Node::Footnote(_) => "Footnote",
        Node::FootnoteRef(_) => "Footnote Ref",
        Node::Definition(_) => "Definition",
        Node::MdxFlowExpression(_) => "MDX Flow Expression",
        Node::MdxJsxFlowElement(_) => "MDX JSX Element",
        Node::MdxJsxTextElement(_) => "MDX JSX Text",
        Node::MdxTextExpression(_) => "MDX Text Expression",
        Node::MdxJsEsm(_) => "MDX JS ESM",
        Node::Empty => "Empty",
        Node::Callout(_) => "Callout",
        Node::Embed(_) => "Embed",
        Node::WikiLink(_) => "WikiLink",
    }
}

fn max_line_width(lines: &[Line]) -> u16 {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.width())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0) as u16
}

fn draw_help_screen(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let help_block = Block::default()
        .title("Keyboard Controls")
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .style(Style::default().bg(Color::Black));

    let left_column = vec![
        Line::from(vec![Span::styled(
            "Navigation",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Up/k", Style::default().fg(Color::Yellow)),
            Span::raw(" - Move up"),
        ]),
        Line::from(vec![
            Span::styled("Down/j", Style::default().fg(Color::Yellow)),
            Span::raw(" - Move down"),
        ]),
        Line::from(vec![
            Span::styled("g", Style::default().fg(Color::Yellow)),
            Span::raw(" - Jump to first result"),
        ]),
        Line::from(vec![
            Span::styled("G", Style::default().fg(Color::Yellow)),
            Span::raw(" - Jump to last result"),
        ]),
        Line::from(vec![
            Span::styled("PgUp", Style::default().fg(Color::Yellow)),
            Span::raw(" - Page up"),
        ]),
        Line::from(vec![
            Span::styled("PgDn", Style::default().fg(Color::Yellow)),
            Span::raw(" - Page down"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Query Mode",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled(":", Style::default().fg(Color::Yellow)),
            Span::raw(" - Enter query mode"),
        ]),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" - Execute query"),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" - Exit query mode"),
        ]),
        Line::from(vec![
            Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
            Span::raw(" - Navigate query history"),
        ]),
        Line::from(vec![
            Span::styled("Tab/Shift+Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" - Cycle completion suggestions"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Search",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(" - Search within results or tree"),
        ]),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" - Confirm search"),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" - Cancel search"),
        ]),
        Line::from(vec![
            Span::styled("n/N", Style::default().fg(Color::Yellow)),
            Span::raw(" - Repeat search forward/backward"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Favorite Queries",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("S", Style::default().fg(Color::Yellow)),
            Span::raw(" - Save current query"),
        ]),
        Line::from(vec![
            Span::styled("F", Style::default().fg(Color::Yellow)),
            Span::raw(" - Browse saved queries"),
        ]),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" - Run selected query"),
        ]),
        Line::from(vec![
            Span::styled("d", Style::default().fg(Color::Yellow)),
            Span::raw(" - Delete selected query"),
        ]),
    ];

    let right_column = vec![
        Line::from(vec![Span::styled(
            "Other Commands",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("d", Style::default().fg(Color::Yellow)),
            Span::raw(" - Toggle detail view"),
        ]),
        Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Yellow)),
            Span::raw(" - Copy result to clipboard"),
        ]),
        Line::from(vec![
            Span::styled("Y", Style::default().fg(Color::Yellow)),
            Span::raw(" - Copy selected row to clipboard"),
        ]),
        Line::from(vec![
            Span::styled("q/Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" - Quit application"),
        ]),
        Line::from(vec![
            Span::styled("?", Style::default().fg(Color::Yellow)),
            Span::raw(" - Show this help"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+l", Style::default().fg(Color::Yellow)),
            Span::raw(" - Clear query"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+k", Style::default().fg(Color::Yellow)),
            Span::raw(" - Open command palette"),
        ]),
        Line::from(vec![
            Span::styled("</>", Style::default().fg(Color::Yellow)),
            Span::raw(" - Resize sidebar/detail/preview split"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Tabs / Files",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Left/Right", Style::default().fg(Color::Yellow)),
            Span::raw(" - Switch tabs"),
        ]),
        Line::from(vec![
            Span::styled("Tab/Shift+Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" - Switch tabs"),
        ]),
        Line::from(vec![
            Span::styled("o", Style::default().fg(Color::Yellow)),
            Span::raw(" - Open a file as a new tab"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Tree View Mode",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("t", Style::default().fg(Color::Yellow)),
            Span::raw(" - Toggle tree view"),
        ]),
        Line::from(vec![
            Span::styled("s", Style::default().fg(Color::Yellow)),
            Span::raw(" - Toggle sidebar (headers)"),
        ]),
        Line::from(vec![
            Span::styled("Up/k", Style::default().fg(Color::Yellow)),
            Span::raw(" - Move up in tree"),
        ]),
        Line::from(vec![
            Span::styled("Down/j", Style::default().fg(Color::Yellow)),
            Span::raw(" - Move down in tree"),
        ]),
        Line::from(vec![
            Span::styled("g", Style::default().fg(Color::Yellow)),
            Span::raw(" - Jump to first node"),
        ]),
        Line::from(vec![
            Span::styled("G", Style::default().fg(Color::Yellow)),
            Span::raw(" - Jump to last node"),
        ]),
        Line::from(vec![
            Span::styled("Enter/Space", Style::default().fg(Color::Yellow)),
            Span::raw(" - Expand/collapse node"),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" - Exit tree view"),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Preview Mode",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::UNDERLINED),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("p", Style::default().fg(Color::Yellow)),
            Span::raw(" - Toggle rendered preview"),
        ]),
        Line::from(vec![
            Span::styled("Up/k Down/j", Style::default().fg(Color::Yellow)),
            Span::raw(" - Scroll preview"),
        ]),
        Line::from(vec![
            Span::styled("u/d", Style::default().fg(Color::Yellow)),
            Span::raw(" - Scroll half-page up/down"),
        ]),
        Line::from(vec![
            Span::styled("b/f/Space", Style::default().fg(Color::Yellow)),
            Span::raw(" - Scroll full page up/down"),
        ]),
        Line::from(vec![
            Span::styled("g/G", Style::default().fg(Color::Yellow)),
            Span::raw(" - Jump to top/bottom"),
        ]),
        Line::from(vec![
            Span::styled("s", Style::default().fg(Color::Yellow)),
            Span::raw(" - Toggle split with raw source"),
        ]),
        Line::from(vec![
            Span::styled("Esc/p", Style::default().fg(Color::Yellow)),
            Span::raw(" - Exit preview"),
        ]),
    ];

    const COLUMN_GAP: u16 = 4;

    let left_width = max_line_width(&left_column);
    let right_width = max_line_width(&right_column);

    let content_lines = left_column.len().max(right_column.len());
    let width = (left_width + COLUMN_GAP + right_width + 2).clamp(30, area.width.max(30));
    let height = (content_lines as u16 + 2).clamp(15, area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let help_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, help_area);
    let inner = help_block.inner(help_area);
    frame.render_widget(help_block, help_area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left_width),
            Constraint::Length(COLUMN_GAP),
            Constraint::Length(right_width),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(left_column).alignment(Alignment::Left),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(right_column).alignment(Alignment::Left),
        columns[2],
    );

    let footer = format!(
        "Theme: {:?} | Hint bar: {} (config.toml)",
        app.theme_name(),
        if app.show_hint_bar() { "on" } else { "off" }
    );
    let footer_area = Rect::new(
        help_area.x + 1,
        help_area.y + help_area.height - 1,
        help_area.width.saturating_sub(2),
        1,
    );
    frame.render_widget(
        Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
        footer_area,
    );
}

fn draw_error_popup(frame: &mut Frame, error: &str) {
    let frame_size = frame.area();

    let width = frame_size.width.clamp(20, 60);
    let height = 3;

    let x = (frame_size.width.saturating_sub(width)) / 2;
    let y = (frame_size.height.saturating_sub(height)) / 2;

    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let error_block = Block::default()
        .title("Error")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Red).fg(Color::White));

    let error_text = Paragraph::new(error)
        .wrap(Wrap { trim: true })
        .style(Style::default().bg(Color::Red).fg(Color::White))
        .block(error_block);

    frame.render_widget(error_text, popup_area);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use itertools::Itertools;
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn create_test_app() -> App {
        let mut app = App::new("".to_string());
        app.set_query("test query".to_string());
        app
    }

    #[test]
    fn test_highlight_query_classifies_tokens() {
        let theme = Theme::dark();
        let spans = highlight_query(r#".h | select(.depth == 1) # note"#, &theme);
        let plain: Vec<String> = spans.iter().map(|s| s.content.to_string()).collect();

        assert!(plain.contains(&".h".to_string()));
        assert!(plain.contains(&"select".to_string()));
        assert!(plain.contains(&".depth".to_string()));
        assert!(plain.contains(&"|".to_string()));
        assert!(plain.contains(&"1".to_string()));
        assert!(plain.iter().any(|s| s.starts_with('#')));

        let selector_span = spans.iter().find(|s| s.content == ".h").unwrap();
        assert_eq!(selector_span.style.fg, Some(theme.selector));

        let function_span = spans.iter().find(|s| s.content == "select").unwrap();
        assert_eq!(function_span.style.fg, Some(theme.function));
    }

    #[test]
    fn test_highlight_query_string_literal_is_single_span() {
        let theme = Theme::dark();
        let spans = highlight_query(r#"select(.text == "hello world")"#, &theme);
        assert!(spans.iter().any(|s| s.content == "\"hello world\""));
    }

    fn create_app_with_results() -> App {
        let mut app = App::new("test.md".to_string());
        let results = vec![
            mq_markdown::Node::Heading(mq_markdown::Heading {
                depth: 1,
                position: None,
                values: vec![mq_markdown::Node::Text(mq_markdown::Text {
                    value: "Test Heading".to_string(),
                    position: None,
                })],
            }),
            mq_markdown::Node::Text(mq_markdown::Text {
                value: "Test paragraph content".to_string(),
                position: None,
            }),
            mq_markdown::Node::Code(mq_markdown::Code {
                meta: None,
                fence: true,
                lang: Some("rust".to_string()),
                value: "fn main() {}".to_string(),
                position: None,
            }),
        ];
        app.set_results(results);
        app.set_last_exec_time(Duration::from_millis(150));
        app
    }

    #[test]
    fn test_draw_ui_normal_mode() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = create_test_app();

        terminal
            .draw(|frame| {
                draw_ui(frame, &app);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        // Check for sidebar hint or other UI elements
        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("sidebar")
        );
    }

    #[test]
    fn test_draw_ui_query_mode() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut app = create_test_app();
        app.set_mode(Mode::Query);

        terminal
            .draw(|frame| {
                draw_ui(frame, &app);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Query")
        );
    }

    #[test]
    fn test_draw_ui_help_mode() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut app = create_test_app();
        app.set_mode(Mode::Help);

        terminal
            .draw(|frame| {
                draw_ui(frame, &app);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Keyboard Controls")
        );
    }

    #[test]
    fn test_draw_ui_with_error() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut app = create_test_app();
        app.set_error_msg("Test error message".to_string());

        terminal
            .draw(|frame| {
                draw_ui(frame, &app);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Error")
        );
    }

    #[test]
    fn test_draw_results_list_with_data() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = create_app_with_results();

        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_results_list(frame, &app, area);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Results")
        );
    }

    #[test]
    fn test_wrap_to_width_splits_long_line() {
        let rows = wrap_to_width("abcdefghij", 4);
        assert_eq!(rows, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn test_wrap_to_width_short_line_unchanged() {
        let rows = wrap_to_width("short", 40);
        assert_eq!(rows, vec!["short"]);
    }

    #[test]
    fn test_wrap_to_width_preserves_whitespace() {
        // A hard, position-preserving wrap must never collapse or drop
        // interior spaces (this would corrupt code/table indentation).
        let rows = wrap_to_width("a    b", 3);
        assert_eq!(rows.concat(), "a    b");
    }

    #[test]
    fn test_wrap_to_width_empty_line() {
        assert_eq!(wrap_to_width("", 10), vec![""]);
    }

    #[test]
    fn test_draw_results_list_wraps_long_lines_instead_of_clipping() {
        let mut terminal = Terminal::new(TestBackend::new(20, 24)).unwrap();
        let mut app = App::new("".to_string());
        app.set_results(vec![mq_markdown::Node::Text(mq_markdown::Text {
            value: "This line is much longer than the twenty column width of the terminal"
                .to_string(),
            position: None,
        })]);

        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_results_list(frame, &app, area);
            })
            .unwrap();

        let backend = terminal.backend();
        let content = backend
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .join("");

        // The tail of the line must still be visible somewhere (wrapped to
        // a later row), not silently dropped because it didn't fit on one row.
        assert!(content.contains("terminal"));
    }

    #[test]
    fn test_draw_title_bar_without_filename() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = create_test_app();

        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_title_bar(frame, &app, area);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        // Check for the new sidebar hint in title bar
        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("sidebar")
        );
    }

    #[test]
    fn test_draw_detail_view_empty_results() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = create_test_app();

        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_detail_view(frame, &app, area);
            })
            .unwrap();
    }

    #[test]
    fn test_draw_detail_view_with_selection() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = create_app_with_results();

        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_detail_view(frame, &app, area);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Detail View")
        );
    }

    #[test]
    fn test_draw_help_screen_content() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = create_test_app();

        terminal
            .draw(|frame| {
                draw_help_screen(frame, &app);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Navigation")
        );

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Query Mode")
        );

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("Other Commands")
        );
    }

    #[test]
    fn test_draw_error_popup_content() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let error_msg = "Test error message";

        terminal
            .draw(|frame| {
                draw_error_popup(frame, error_msg);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();
        let content = buffer.content().iter().map(|c| c.symbol()).join("");

        assert!(content.contains("Error"));
        assert!(content.contains(error_msg));
    }

    #[test]
    fn test_draw_query_input_cursor_position() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_cursor_position(5);

        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_query_input(frame, &app, area);
            })
            .unwrap();
    }

    #[test]
    fn test_ui_layout_constraints() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = create_test_app();

        terminal
            .draw(|frame| {
                draw_ui(frame, &app);
            })
            .unwrap();

        let mut small_terminal = Terminal::new(TestBackend::new(20, 10)).unwrap();
        small_terminal
            .draw(|frame| {
                draw_ui(frame, &app);
            })
            .unwrap();
    }

    #[test]
    fn test_draw_ui_tree_view_mode() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut app = create_test_app();
        app.set_mode(Mode::TreeView);

        terminal
            .draw(|frame| {
                draw_ui(frame, &app);
            })
            .unwrap();

        let backend = terminal.backend();
        let buffer = backend.buffer();

        assert!(
            buffer
                .content()
                .iter()
                .map(|c| c.symbol())
                .join("")
                .contains("TREE VIEW")
        );
    }

    #[test]
    fn test_title_bar_mode_indicators() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        // Test Normal mode
        let app_normal = create_test_app();
        terminal
            .draw(|frame| draw_title_bar(frame, &app_normal, frame.area()))
            .unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .join("");
        assert!(content.contains("NORMAL"));

        // Test TreeView mode
        let mut app_tree = create_test_app();
        app_tree.set_mode(Mode::TreeView);
        terminal
            .draw(|frame| draw_title_bar(frame, &app_tree, frame.area()))
            .unwrap();
        let content = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .join("");
        assert!(content.contains("TREE VIEW"));
    }

    #[test]
    fn test_is_markdown_header() {
        // Valid headers
        assert!(is_markdown_header("# Header 1"));
        assert!(is_markdown_header("## Header 2"));
        assert!(is_markdown_header("### Header 3"));
        assert!(is_markdown_header("#### Header 4"));
        assert!(is_markdown_header("##### Header 5"));
        assert!(is_markdown_header("###### Header 6"));
        assert!(is_markdown_header("  # Indented header"));
        assert!(is_markdown_header("\t## Tabbed header"));

        // Invalid headers
        assert!(!is_markdown_header("#NoSpace"));
        assert!(!is_markdown_header("No header here"));
        assert!(!is_markdown_header(""));
        assert!(!is_markdown_header("   "));
        assert!(!is_markdown_header("Regular text # with hash"));
    }
}
