use arboard::Clipboard;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use miette::IntoDiagnostic;
use mq_lang::{BUILTIN_FUNCTION_DOC, BUILTIN_SELECTOR_DOC, Engine};
use mq_markdown::Markdown;
use ratatui::prelude::*;
use std::{
    cell::Cell,
    fmt::Display,
    io::Stdout,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    config::{Config, ThemeName},
    event::{EventHandler, EventHandlerExt},
    theme::Theme,
    ui::{draw_ui, preview, treeview::TreeView},
    util,
    watcher::{self, FileWatcher},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Query,
    Help,
    TreeView,
    OpenFile,
    Preview,
    /// Incremental search ('/'), entered from Normal or TreeView mode.
    Search,
    /// Prompting for a name under which to save the current query.
    SaveQuery,
    /// Browsing/running/deleting saved (favorite) queries.
    Favorites,
    /// Fuzzy-searchable list of app actions (opened with Ctrl+K).
    CommandPalette,
}

impl Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Normal => write!(f, "NORMAL"),
            Mode::Query => write!(f, "QUERY"),
            Mode::Help => write!(f, "HELP"),
            Mode::TreeView => write!(f, "TREE VIEW"),
            Mode::OpenFile => write!(f, "OPEN FILE"),
            Mode::Preview => write!(f, "PREVIEW"),
            Mode::Search => write!(f, "SEARCH"),
            Mode::SaveQuery => write!(f, "SAVE QUERY"),
            Mode::Favorites => write!(f, "FAVORITES"),
            Mode::CommandPalette => write!(f, "COMMAND PALETTE"),
        }
    }
}

/// A command palette entry, backed by the keypress that already implements it in Normal mode.
pub struct PaletteAction {
    pub label: &'static str,
    pub description: &'static str,
    key: KeyCode,
    modifiers: KeyModifiers,
}

const PALETTE_ACTIONS: &[PaletteAction] = &[
    PaletteAction {
        label: "Query",
        description: "Enter query mode",
        key: KeyCode::Char(':'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Tree view",
        description: "Toggle tree view mode",
        key: KeyCode::Char('t'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Preview",
        description: "Toggle rendered preview mode",
        key: KeyCode::Char('p'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Sidebar",
        description: "Toggle sidebar (headers)",
        key: KeyCode::Char('s'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Detail view",
        description: "Toggle detail view for the selected item",
        key: KeyCode::Char('d'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Copy results",
        description: "Copy all results to the clipboard",
        key: KeyCode::Char('y'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Copy selected row",
        description: "Copy the selected row to the clipboard",
        key: KeyCode::Char('Y'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Search",
        description: "Incremental search within results",
        key: KeyCode::Char('/'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Save query",
        description: "Save the current query as a favorite",
        key: KeyCode::Char('S'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Favorites",
        description: "Browse saved (favorite) queries",
        key: KeyCode::Char('F'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Open file",
        description: "Open a file as a new tab",
        key: KeyCode::Char('o'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Clear query",
        description: "Clear the current query",
        key: KeyCode::Char('l'),
        modifiers: KeyModifiers::CONTROL,
    },
    PaletteAction {
        label: "Help",
        description: "Show the keyboard shortcuts help screen",
        key: KeyCode::Char('?'),
        modifiers: KeyModifiers::NONE,
    },
    PaletteAction {
        label: "Quit",
        description: "Quit the application",
        key: KeyCode::Char('q'),
        modifiers: KeyModifiers::NONE,
    },
];

/// Case-insensitive substring match.
fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    needle.is_empty() || haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// A named query saved to disk so it can be reused across sessions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedQuery {
    pub name: String,
    pub query: String,
}

/// A single open Markdown document (tab).
struct Document {
    /// The Markdown content to process
    content: String,
    /// Filename (if loaded from a file)
    filename: Option<String>,
    /// The current results from the query
    results: Vec<mq_markdown::Node>,
    /// Currently selected result index
    selected_idx: usize,
    /// Error message if the query/parse fails for this document
    error_msg: Option<String>,
    /// All parsed markdown nodes (for section extraction)
    all_nodes: Vec<mq_markdown::Node>,
    /// Sidebar tree view (headers only)
    sidebar_tree_view: Option<TreeView>,
    /// Path on disk this document was loaded from (used for `--watch`)
    path: Option<PathBuf>,
}

impl Document {
    fn new(content: String, filename: Option<String>) -> Self {
        let mut doc = Self {
            content,
            filename,
            results: Vec::new(),
            selected_idx: 0,
            error_msg: None,
            all_nodes: Vec::new(),
            sidebar_tree_view: None,
            path: None,
        };
        doc.init_sidebar_tree_view();
        doc
    }

    fn with_path(content: String, filename: Option<String>, path: PathBuf) -> Self {
        let mut doc = Self {
            content,
            filename,
            results: Vec::new(),
            selected_idx: 0,
            error_msg: None,
            all_nodes: Vec::new(),
            sidebar_tree_view: None,
            path: Some(path),
        };
        doc.init_sidebar_tree_view();
        doc
    }

    fn display_name(&self) -> &str {
        self.filename.as_deref().unwrap_or("untitled")
    }

    fn init_sidebar_tree_view(&mut self) {
        let markdown_result = Markdown::from_markdown_str(&self.content);
        match markdown_result {
            Ok(markdown) => {
                // Store all nodes for section extraction
                self.all_nodes = markdown.nodes.clone();

                // Extract only heading nodes for the sidebar
                let headers: Vec<mq_markdown::Node> = markdown
                    .nodes
                    .into_iter()
                    .filter(|node| matches!(node, mq_markdown::Node::Heading(_)))
                    .collect();

                if !headers.is_empty() {
                    let mut tree_view = TreeView::new(headers);
                    tree_view.rebuild_items_with_all_documents(true);
                    self.sidebar_tree_view = Some(tree_view);
                }
            }
            Err(_) => {
                // Silently fail for sidebar initialization
            }
        }
    }

    /// Extract section content from heading to the next same-level heading
    fn extract_section_content(&self, heading: &mq_markdown::Heading) -> Vec<mq_markdown::Node> {
        let mut section_nodes = Vec::new();
        let mut found_heading = false;
        let target_depth = heading.depth;

        for node in &self.all_nodes {
            if !found_heading {
                // Check if this is our target heading
                if let mq_markdown::Node::Heading(h) = node
                    && h.depth == heading.depth
                {
                    let h_text: String = h.values.iter().map(|n| n.value()).collect();
                    let target_text: String = heading.values.iter().map(|n| n.value()).collect();
                    if h_text == target_text {
                        found_heading = true;
                        section_nodes.push(node.clone());
                    }
                }
            } else {
                // After finding the heading, collect nodes until next same-level heading
                if let mq_markdown::Node::Heading(h) = node
                    && h.depth <= target_depth
                {
                    // Found next same-level or higher-level heading, stop
                    break;
                }
                section_nodes.push(node.clone());
            }
        }

        section_nodes
    }
}

/// What the sidebar selection resolved to, used to update the active document's results.
enum SidebarSelection {
    AllDocuments,
    Heading(mq_markdown::Heading, String),
}

pub struct App {
    /// Open documents (tabs)
    documents: Vec<Document>,
    /// Index of the currently active document
    active_doc: usize,
    /// The query to run on the Markdown content (shared across all open documents)
    query: String,
    /// Last query execution time (total, across all open documents)
    last_exec_time: Duration,
    /// Last query execution timestamp
    last_exec: Instant,
    /// Should the application exit
    should_quit: bool,
    /// Transient error message not tied to a specific document (e.g. clipboard failures)
    transient_error: Option<String>,
    /// Current app mode
    mode: Mode,
    /// Show detailed view of selected item
    show_detail: bool,
    /// History of executed queries
    query_history: Vec<String>,
    /// Current position in query history
    history_position: Option<usize>,
    /// Current cursor position in query string
    cursor_position: usize,
    /// Tree view component for the active document (Mode::TreeView)
    tree_view: Option<TreeView>,
    /// Show tree sidebar in Normal mode
    show_tree_sidebar: bool,
    /// Whether a query execution is pending (for debouncing)
    query_pending: bool,
    /// Debounce duration for query execution
    debounce_duration: Duration,
    /// Path currently being typed in Mode::OpenFile
    open_file_path: String,
    /// Cursor position within open_file_path
    open_file_cursor: usize,
    /// Current scroll offset (in lines) of the rendered preview (Mode::Preview)
    preview_scroll: u16,
    /// Height of the preview viewport from the last draw, used to clamp scrolling
    preview_viewport_height: Cell<u16>,
    /// Whether watched files should be reloaded automatically when changed on disk
    watch: bool,
    /// Event-driven watcher (OS file system notifications) backing `watch`
    file_watcher: Option<FileWatcher>,
    /// Current incremental search text (Mode::Search)
    search_query: String,
    /// Cursor position within `search_query`
    search_cursor: usize,
    /// Mode to return to when leaving Mode::Search (Normal or TreeView)
    search_return_mode: Mode,
    /// Mode to return to when leaving Mode::Query (Normal, TreeView, or
    /// Preview) - lets the user type a query while watching the tree/preview
    /// update live instead of always dropping back to the results list.
    query_return_mode: Mode,
    /// Selected index (in the active document's results, or the tree view)
    /// recorded when entering Mode::Search, so Esc can restore it
    search_origin_idx: usize,
    /// Active tab recorded when entering Mode::Search from Normal mode, so
    /// Esc can restore it after a cross-tab search jump
    search_origin_doc: usize,
    /// Last committed search term, used by `n` / `N` to repeat the search
    last_search: Option<String>,
    /// Name being typed for the query currently being saved (Mode::SaveQuery)
    save_query_name: String,
    /// Cursor position within `save_query_name`
    save_query_cursor: usize,
    /// Queries saved across sessions (loaded from disk)
    saved_queries: Vec<SavedQuery>,
    /// Currently selected index in Mode::Favorites
    favorites_selected: usize,
    /// Show raw source side-by-side with the rendered preview (Mode::Preview)
    preview_split: bool,
    /// Active Tab-completion cycle, if the user is currently cycling through candidates
    completion_cycle: Option<CompletionCycle>,
    /// Persisted UI settings (theme, hint bar visibility)
    config: Config,
    /// Split ratio (left pane percent) for the results/detail split view
    detail_split_percent: u16,
    /// Split ratio (source pane percent) for the preview/source split view
    preview_split_percent: u16,
    sidebar_split_percent: u16,
    /// Current filter text typed in Mode::CommandPalette
    palette_query: String,
    /// Cursor position within `palette_query`
    palette_cursor: usize,
    /// Currently selected index among the filtered palette actions
    palette_selected: usize,
}

/// Tracks an in-progress Tab-completion cycle so repeated Tab presses step
/// through the same candidate list instead of recomputing it against
/// whatever text the previous candidate left behind.
struct CompletionCycle {
    start: usize,
    matches: Vec<(String, String)>,
    index: usize,
}

impl App {
    pub fn new(content: String) -> Self {
        Self::from_documents(vec![Document::new(content, None)])
    }

    pub fn with_file(content: String, filename: String) -> Self {
        Self::from_documents(vec![Document::new(content, Some(filename))])
    }

    /// Create an App with multiple open documents (tabs), each as (content, filename).
    pub fn with_files(files: Vec<(String, String)>) -> Self {
        let documents = files
            .into_iter()
            .map(|(content, filename)| Document::new(content, Some(filename)))
            .collect();
        Self::from_documents(documents)
    }

    /// Create an App with multiple open documents (tabs), each as (content, filename, path).
    /// The path is used to support `--watch`.
    pub fn with_files_with_paths(files: Vec<(String, String, PathBuf)>) -> Self {
        let documents = files
            .into_iter()
            .map(|(content, filename, path)| Document::with_path(content, Some(filename), path))
            .collect();
        Self::from_documents(documents)
    }

    fn from_documents(documents: Vec<Document>) -> Self {
        Self {
            documents,
            active_doc: 0,
            query: ".".to_string(),
            last_exec_time: Duration::from_millis(0),
            last_exec: Instant::now(),
            should_quit: false,
            transient_error: None,
            mode: Mode::Normal,
            show_detail: false,
            query_history: Vec::new(),
            history_position: None,
            cursor_position: 0,
            tree_view: None,
            show_tree_sidebar: false,
            query_pending: false,
            debounce_duration: Duration::from_millis(300),
            open_file_path: String::new(),
            open_file_cursor: 0,
            preview_scroll: 0,
            preview_viewport_height: Cell::new(0),
            watch: false,
            file_watcher: None,
            search_query: String::new(),
            search_cursor: 0,
            search_return_mode: Mode::Normal,
            query_return_mode: Mode::Normal,
            search_origin_idx: 0,
            search_origin_doc: 0,
            last_search: None,
            save_query_name: String::new(),
            save_query_cursor: 0,
            saved_queries: crate::favorites::load(),
            favorites_selected: 0,
            preview_split: false,
            completion_cycle: None,
            config: crate::config::load(),
            detail_split_percent: 40,
            preview_split_percent: 50,
            sidebar_split_percent: 20,
            palette_query: String::new(),
            palette_cursor: 0,
            palette_selected: 0,
        }
    }

    fn active_doc(&self) -> &Document {
        &self.documents[self.active_doc]
    }

    fn active_doc_mut(&mut self) -> &mut Document {
        &mut self.documents[self.active_doc]
    }

    pub fn run(&mut self) -> miette::Result<()> {
        let mut terminal = util::setup_terminal()?;
        let events = EventHandler::new(Duration::from_millis(100));

        self.exec_query();

        while !self.should_quit {
            self.draw(&mut terminal)?;

            if let Some(event) = events.next()? {
                self.handle_event(event)?;
            }

            // Check if we should execute a pending query (debounce)
            if self.query_pending && self.last_exec.elapsed() >= self.debounce_duration {
                self.exec_query();
            }

            // Cheap, non-blocking: just drains any pending OS file system
            // notifications, no polling involved.
            self.check_for_file_changes();
        }

        util::restore_terminal()?;

        Ok(())
    }

    /// Enable or disable watching opened files for changes on disk. Uses the
    /// OS's native file system notifications (inotify / FSEvents /
    /// ReadDirectoryChangesW) rather than polling.
    pub fn set_watch(&mut self, watch: bool) {
        self.watch = watch;

        if !watch {
            self.file_watcher = None;
            return;
        }

        if self.file_watcher.is_none() {
            match FileWatcher::new() {
                Ok(mut watcher) => {
                    for doc in &self.documents {
                        if let Some(path) = &doc.path {
                            watcher.watch_file(path);
                        }
                    }
                    self.file_watcher = Some(watcher);
                }
                Err(err) => {
                    self.transient_error = Some(format!("Could not watch files: {err}"));
                    self.watch = false;
                }
            }
        }
    }

    /// Whether watch mode is enabled.
    pub fn watch(&self) -> bool {
        self.watch
    }

    /// Reload any document whose source file was reported as changed by the
    /// file watcher, re-running the current query if anything changed.
    fn check_for_file_changes(&mut self) {
        let Some(watcher) = &self.file_watcher else {
            return;
        };
        let changed_paths = watcher.drain_changed_paths();
        if changed_paths.is_empty() {
            return;
        }

        let mut changed = false;
        for doc in self.documents.iter_mut() {
            let Some(path) = doc.path.clone() else {
                continue;
            };
            if !changed_paths
                .iter()
                .any(|p| watcher::paths_refer_to_same_file(p, &path))
            {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path)
                && content != doc.content
            {
                doc.content = content;
                doc.init_sidebar_tree_view();
                changed = true;
            }
        }

        if changed {
            self.exec_query();
            if self.mode == Mode::TreeView {
                self.init_tree_view();
            }
        }
    }

    fn draw(&self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> miette::Result<()> {
        terminal
            .draw(|frame| draw_ui(frame, self))
            .into_diagnostic()?;
        Ok(())
    }

    pub fn handle_event(&mut self, event: Event) -> miette::Result<()> {
        self.transient_error = None;
        self.active_doc_mut().error_msg = None;
        match self.mode {
            Mode::Normal => self.handle_normal_mode_event(event),
            Mode::Query => self.handle_query_mode_event(event),
            Mode::Help => self.handle_help_mode_event(event),
            Mode::TreeView => self.handle_tree_view_mode_event(event),
            Mode::OpenFile => self.handle_open_file_mode_event(event),
            Mode::Preview => self.handle_preview_mode_event(event),
            Mode::Search => self.handle_search_mode_event(event),
            Mode::SaveQuery => self.handle_save_query_mode_event(event),
            Mode::Favorites => self.handle_favorites_mode_event(event),
            Mode::CommandPalette => self.handle_command_palette_mode_event(event),
        }
    }

    fn handle_normal_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Mouse(mouse_event) = event {
            match mouse_event.kind {
                MouseEventKind::ScrollDown if !self.active_doc().results.is_empty() => {
                    let idx = self.next_visible_from_current(true);
                    self.active_doc_mut().selected_idx = idx;
                }
                MouseEventKind::ScrollUp if !self.active_doc().results.is_empty() => {
                    let idx = self.next_visible_from_current(false);
                    self.active_doc_mut().selected_idx = idx;
                }
                _ => {}
            }
            return Ok(());
        }

        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                // Quit on Escape or q
                (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                    self.should_quit = true;
                }
                // Toggle detailed view
                (KeyCode::Char('d'), _) => {
                    self.show_detail = !self.show_detail;
                }
                // Enter query mode
                (KeyCode::Char(':'), _) => {
                    self.query_return_mode = Mode::Normal;
                    self.mode = Mode::Query;
                    self.cursor_position = self.query.len();
                }
                // Show help
                (KeyCode::Char('?'), _) | (KeyCode::F(1), _) => {
                    self.mode = Mode::Help;
                }
                // Toggle tree view
                (KeyCode::Char('t'), _) => {
                    self.mode = Mode::TreeView;
                    self.init_tree_view();
                }
                // Toggle rendered preview
                (KeyCode::Char('p'), _) => {
                    self.mode = Mode::Preview;
                    self.preview_scroll = 0;
                }
                // Toggle tree sidebar
                (KeyCode::Char('s'), _) => {
                    self.toggle_tree_sidebar();
                    // Update selection when sidebar is shown
                    if self.show_tree_sidebar {
                        self.update_sidebar_selection();
                    }
                }
                // Open a new file as an additional tab
                (KeyCode::Char('o'), _) => {
                    self.mode = Mode::OpenFile;
                    self.open_file_path.clear();
                    self.open_file_cursor = 0;
                }
                // Incremental search within the current results
                (KeyCode::Char('/'), _) => {
                    self.search_return_mode = Mode::Normal;
                    self.search_origin_idx = self.active_doc().selected_idx;
                    self.search_origin_doc = self.active_doc;
                    self.search_query.clear();
                    self.search_cursor = 0;
                    self.mode = Mode::Search;
                }
                // Repeat last search
                (KeyCode::Char('n'), _) if self.last_search.is_some() => {
                    self.repeat_search(true);
                }
                (KeyCode::Char('N'), _) if self.last_search.is_some() => {
                    self.repeat_search(false);
                }
                // Save the current query under a name
                (KeyCode::Char('S'), _) => {
                    self.mode = Mode::SaveQuery;
                    self.save_query_name.clear();
                    self.save_query_cursor = 0;
                }
                // Browse saved (favorite) queries
                (KeyCode::Char('F'), _) => {
                    self.favorites_selected = 0;
                    self.mode = Mode::Favorites;
                }
                // Open the command palette
                (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                    self.palette_query.clear();
                    self.palette_cursor = 0;
                    self.palette_selected = 0;
                    self.mode = Mode::CommandPalette;
                }
                (KeyCode::Char('<'), _) if self.show_tree_sidebar => {
                    self.adjust_sidebar_split(-5);
                }
                (KeyCode::Char('>'), _) if self.show_tree_sidebar => {
                    self.adjust_sidebar_split(5);
                }
                (KeyCode::Char('<'), _) if self.show_detail => {
                    self.adjust_detail_split(-5);
                }
                (KeyCode::Char('>'), _) if self.show_detail => {
                    self.adjust_detail_split(5);
                }
                // Switch tabs
                (KeyCode::Right, _) | (KeyCode::Tab, _) => {
                    self.next_tab();
                }
                (KeyCode::Left, _) | (KeyCode::BackTab, _) => {
                    self.prev_tab();
                }
                // Navigate results or sidebar
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if self.show_tree_sidebar && self.active_doc().sidebar_tree_view.is_some() {
                        if let Some(sidebar) = &mut self.active_doc_mut().sidebar_tree_view {
                            sidebar.move_down();
                        }
                        self.update_sidebar_selection();
                    } else if !self.active_doc().results.is_empty() {
                        let idx = self.next_visible_from_current(true);
                        self.active_doc_mut().selected_idx = idx;
                    }
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    if self.show_tree_sidebar && self.active_doc().sidebar_tree_view.is_some() {
                        if let Some(sidebar) = &mut self.active_doc_mut().sidebar_tree_view {
                            sidebar.move_up();
                        }
                        self.update_sidebar_selection();
                    } else if !self.active_doc().results.is_empty() {
                        let idx = self.next_visible_from_current(false);
                        self.active_doc_mut().selected_idx = idx;
                    }
                }
                // Select header in sidebar (Enter key - content already updated by navigation)
                (KeyCode::Enter, _) => {
                    // Content is already updated by update_sidebar_selection()
                    // Enter key can be used for future actions if needed
                }
                (KeyCode::PageDown, _) if !self.active_doc().results.is_empty() => {
                    let next = (self.active_doc().selected_idx + 10)
                        .min(self.active_doc().results.len() - 1);
                    let idx = self.next_visible(next, true);
                    self.active_doc_mut().selected_idx = idx;
                }
                (KeyCode::PageUp, _) if !self.active_doc().results.is_empty() => {
                    let prev = self.active_doc().selected_idx.saturating_sub(10);
                    let idx = self.next_visible(prev, false);
                    self.active_doc_mut().selected_idx = idx;
                }
                (KeyCode::Home, _) if !self.active_doc().results.is_empty() => {
                    let idx = self.next_visible(0, true);
                    self.active_doc_mut().selected_idx = idx;
                }
                (KeyCode::End, _) if !self.active_doc().results.is_empty() => {
                    let last = self.active_doc().results.len() - 1;
                    let idx = self.next_visible(last, false);
                    self.active_doc_mut().selected_idx = idx;
                }
                // Jump to first/last result (vim-style)
                (KeyCode::Char('g'), _) if !self.active_doc().results.is_empty() => {
                    let idx = self.next_visible(0, true);
                    self.active_doc_mut().selected_idx = idx;
                }
                (KeyCode::Char('G'), _) if !self.active_doc().results.is_empty() => {
                    let last = self.active_doc().results.len() - 1;
                    let idx = self.next_visible(last, false);
                    self.active_doc_mut().selected_idx = idx;
                }
                // Clear query with Ctrl+L
                (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.cursor_position = 0;
                    self.exec_query();
                }
                (KeyCode::Char('y'), _) if !self.active_doc().results.is_empty() => {
                    let result_text =
                        mq_markdown::Markdown::new(self.active_doc().results.clone()).to_string();
                    if let Ok(mut clipboard) = Clipboard::new() {
                        if clipboard.set_text(result_text).is_ok() {
                        } else {
                            self.transient_error =
                                Some("Error: Could not copy to clipboard".to_string());
                        }
                    } else {
                        self.transient_error =
                            Some("Error: Could not access clipboard".to_string());
                    }
                }
                (KeyCode::Char('Y'), _) if !self.active_doc().results.is_empty() => {
                    let current_text = self
                        .active_doc()
                        .results
                        .get(self.active_doc().selected_idx)
                        .map(|node| node.to_string())
                        .unwrap_or_default();
                    if let Ok(mut clipboard) = Clipboard::new() {
                        if clipboard.set_text(current_text).is_ok() {
                        } else {
                            self.transient_error =
                                Some("Error: Could not copy to clipboard".to_string());
                        }
                    } else {
                        self.transient_error =
                            Some("Error: Could not access clipboard".to_string());
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_query_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                // Exit query mode on Escape
                (KeyCode::Esc, _) => {
                    self.mode = self.query_return_mode;
                    self.history_position = None;
                }
                // Execute query on Enter
                (KeyCode::Enter, _) => {
                    self.mode = self.query_return_mode;
                    if !self.query.is_empty() {
                        // Add query to history if it's not a duplicate
                        if self.query_history.is_empty()
                            || self.query_history.last() != Some(&self.query)
                        {
                            self.query_history.push(self.query.clone());
                        }
                    }
                    self.history_position = None;
                    self.query_pending = false;
                    self.exec_query();
                }
                // Tab/Shift+Tab step forward/backward through completion
                // candidates for the current word; falls back to switching
                // tabs when there's nothing to complete.
                (KeyCode::Tab, _) => self.cycle_completion(true),
                (KeyCode::BackTab, _) => self.cycle_completion(false),
                // Edit query
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    self.completion_cycle = None;
                    self.query.insert(self.cursor_position, c);
                    self.cursor_position += 1;
                    self.last_exec = Instant::now();
                    self.query_pending = true;
                }
                (KeyCode::Backspace, _) if self.cursor_position > 0 => {
                    self.completion_cycle = None;
                    self.query.remove(self.cursor_position - 1);
                    self.cursor_position -= 1;
                    self.last_exec = Instant::now();
                    self.query_pending = true;
                }
                (KeyCode::Delete, _) if self.cursor_position < self.query.len() => {
                    self.completion_cycle = None;
                    self.query.remove(self.cursor_position);
                    self.last_exec = Instant::now();
                    self.query_pending = true;
                }
                // Move cursor
                (KeyCode::Left, _) if self.cursor_position > 0 => {
                    self.completion_cycle = None;
                    self.cursor_position -= 1;
                }
                (KeyCode::Right, _) if self.cursor_position < self.query.len() => {
                    self.completion_cycle = None;
                    self.cursor_position += 1;
                }
                (KeyCode::Home, _) => {
                    self.completion_cycle = None;
                    self.cursor_position = 0;
                }
                (KeyCode::End, _) => {
                    self.completion_cycle = None;
                    self.cursor_position = self.query.len();
                }
                // Navigate history
                (KeyCode::Up, _) if !self.query_history.is_empty() => {
                    self.completion_cycle = None;
                    match self.history_position {
                        None => {
                            self.history_position = Some(self.query_history.len() - 1);
                            self.query = self.query_history[self.history_position.unwrap()].clone();
                        }
                        Some(pos) if pos > 0 => {
                            self.history_position = Some(pos - 1);
                            self.query = self.query_history[self.history_position.unwrap()].clone();
                        }
                        _ => {}
                    }
                    self.cursor_position = self.query.len();
                }
                (KeyCode::Down, _) => {
                    self.completion_cycle = None;
                    if let Some(pos) = self.history_position {
                        if pos < self.query_history.len() - 1 {
                            self.history_position = Some(pos + 1);
                            self.query = self.query_history[self.history_position.unwrap()].clone();
                        } else {
                            self.history_position = None;
                            self.query.clear();
                        }
                        self.cursor_position = self.query.len();
                    }
                }

                _ => {}
            }
        }

        Ok(())
    }

    /// Advance (or, if `forward` is false, step back) the Tab-completion
    /// cycle for the word at the cursor, starting a new cycle if none is
    /// active. Falls back to switching tabs if there's nothing to complete.
    fn cycle_completion(&mut self, forward: bool) {
        if self.completion_cycle.is_none() {
            let matches = self.query_completions();
            if matches.is_empty() {
                if forward {
                    self.next_tab();
                } else {
                    self.prev_tab();
                }
                return;
            }
            let (start, _) = Self::word_range_at_cursor(&self.query, self.cursor_position);
            self.completion_cycle = Some(CompletionCycle {
                start,
                matches,
                index: 0,
            });
        } else if let Some(cycle) = &mut self.completion_cycle {
            let len = cycle.matches.len();
            cycle.index = if forward {
                (cycle.index + 1) % len
            } else if cycle.index == 0 {
                len - 1
            } else {
                cycle.index - 1
            };
        }

        let cycle = self.completion_cycle.as_ref().unwrap();
        let name = cycle.matches[cycle.index].0.clone();
        let start = cycle.start;
        self.query.replace_range(start..self.cursor_position, &name);
        self.cursor_position = start + name.len();
        self.last_exec = Instant::now();
        self.query_pending = true;
    }

    fn handle_help_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Key(KeyEvent {
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            self.mode = Mode::Normal;
        }

        Ok(())
    }

    /// Palette actions whose label or description fuzzy-matches `palette_query`.
    pub fn filtered_palette_actions(&self) -> Vec<&'static PaletteAction> {
        PALETTE_ACTIONS
            .iter()
            .filter(|action| {
                fuzzy_match(action.label, &self.palette_query)
                    || fuzzy_match(action.description, &self.palette_query)
            })
            .collect()
    }

    pub fn palette_query(&self) -> &str {
        &self.palette_query
    }

    pub fn palette_cursor(&self) -> usize {
        self.palette_cursor
    }

    pub fn palette_selected(&self) -> usize {
        self.palette_selected
    }

    /// Replays the action's keypress through the normal-mode handler, so
    /// behavior isn't duplicated between the palette and key bindings.
    fn run_palette_action(&mut self, action: &PaletteAction) -> miette::Result<()> {
        self.mode = Mode::Normal;
        self.handle_normal_mode_event(Event::Key(KeyEvent::new(action.key, action.modifiers)))
    }

    fn handle_command_palette_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                (KeyCode::Esc, _) => {
                    self.mode = Mode::Normal;
                }
                (KeyCode::Enter, _) => {
                    let matches = self.filtered_palette_actions();
                    if let Some(action) = matches.get(self.palette_selected).copied() {
                        self.run_palette_action(action)?;
                    } else {
                        self.mode = Mode::Normal;
                    }
                }
                (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                    let len = self.filtered_palette_actions().len();
                    if len > 0 {
                        self.palette_selected = (self.palette_selected + 1) % len;
                    }
                }
                (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                    let len = self.filtered_palette_actions().len();
                    if len > 0 {
                        self.palette_selected = if self.palette_selected == 0 {
                            len - 1
                        } else {
                            self.palette_selected - 1
                        };
                    }
                }
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    self.palette_query.insert(self.palette_cursor, c);
                    self.palette_cursor += 1;
                    self.palette_selected = 0;
                }
                (KeyCode::Backspace, _) if self.palette_cursor > 0 => {
                    self.palette_query.remove(self.palette_cursor - 1);
                    self.palette_cursor -= 1;
                    self.palette_selected = 0;
                }
                (KeyCode::Delete, _) if self.palette_cursor < self.palette_query.len() => {
                    self.palette_query.remove(self.palette_cursor);
                    self.palette_selected = 0;
                }
                (KeyCode::Left, _) if self.palette_cursor > 0 => {
                    self.palette_cursor -= 1;
                }
                (KeyCode::Right, _) if self.palette_cursor < self.palette_query.len() => {
                    self.palette_cursor += 1;
                }
                (KeyCode::Home, _) => {
                    self.palette_cursor = 0;
                }
                (KeyCode::End, _) => {
                    self.palette_cursor = self.palette_query.len();
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_tree_view_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Mouse(mouse_event) = event {
            match mouse_event.kind {
                MouseEventKind::ScrollDown => {
                    if let Some(tree_view) = &mut self.tree_view {
                        tree_view.move_down();
                    }
                }
                MouseEventKind::ScrollUp => {
                    if let Some(tree_view) = &mut self.tree_view {
                        tree_view.move_up();
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                // Exit tree view mode
                (KeyCode::Esc, _) | (KeyCode::Char('t'), _) => {
                    self.mode = Mode::Normal;
                }
                // Quit
                (KeyCode::Char('q'), _) => {
                    self.should_quit = true;
                }
                // Switch tabs
                (KeyCode::Right, _) | (KeyCode::Tab, _) => {
                    self.next_tab();
                    self.init_tree_view();
                }
                (KeyCode::Left, _) | (KeyCode::BackTab, _) => {
                    self.prev_tab();
                    self.init_tree_view();
                }
                // Navigation
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if let Some(tree_view) = &mut self.tree_view {
                        tree_view.move_down();
                    }
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    if let Some(tree_view) = &mut self.tree_view {
                        tree_view.move_up();
                    }
                }
                // Toggle expand/collapse
                (KeyCode::Enter, _) | (KeyCode::Char(' '), _) => {
                    if let Some(tree_view) = &mut self.tree_view {
                        tree_view.toggle_expand();
                    }
                }
                // Jump to first/last item (vim-style)
                (KeyCode::Char('g'), _) => {
                    if let Some(tree_view) = &mut self.tree_view {
                        tree_view.move_to_first();
                    }
                }
                (KeyCode::Char('G'), _) => {
                    if let Some(tree_view) = &mut self.tree_view {
                        tree_view.move_to_last();
                    }
                }
                // Show help
                (KeyCode::Char('?'), _) | (KeyCode::F(1), _) => {
                    self.mode = Mode::Help;
                }
                // Incremental search within the tree
                (KeyCode::Char('/'), _) => {
                    self.search_return_mode = Mode::TreeView;
                    self.search_origin_idx = self
                        .tree_view
                        .as_ref()
                        .map(|t| t.selected_index())
                        .unwrap_or(0);
                    self.search_query.clear();
                    self.search_cursor = 0;
                    self.mode = Mode::Search;
                }
                (KeyCode::Char('n'), _) if self.last_search.is_some() => {
                    self.repeat_search(true);
                }
                (KeyCode::Char('N'), _) if self.last_search.is_some() => {
                    self.repeat_search(false);
                }
                // Enter query mode without leaving the tree: matching nodes
                // highlight live in the tree as the query is typed.
                (KeyCode::Char(':'), _) => {
                    self.query_return_mode = Mode::TreeView;
                    self.mode = Mode::Query;
                    self.cursor_position = self.query.len();
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_open_file_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                // Cancel
                (KeyCode::Esc, _) => {
                    self.mode = Mode::Normal;
                    self.open_file_path.clear();
                    self.open_file_cursor = 0;
                }
                // Open the file
                (KeyCode::Enter, _) => {
                    self.open_file();
                }
                // Edit path
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    self.open_file_path.insert(self.open_file_cursor, c);
                    self.open_file_cursor += 1;
                }
                (KeyCode::Backspace, _) if self.open_file_cursor > 0 => {
                    self.open_file_path.remove(self.open_file_cursor - 1);
                    self.open_file_cursor -= 1;
                }
                (KeyCode::Delete, _) if self.open_file_cursor < self.open_file_path.len() => {
                    self.open_file_path.remove(self.open_file_cursor);
                }
                (KeyCode::Left, _) if self.open_file_cursor > 0 => {
                    self.open_file_cursor -= 1;
                }
                (KeyCode::Right, _) if self.open_file_cursor < self.open_file_path.len() => {
                    self.open_file_cursor += 1;
                }
                (KeyCode::Home, _) => {
                    self.open_file_cursor = 0;
                }
                (KeyCode::End, _) => {
                    self.open_file_cursor = self.open_file_path.len();
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Read the typed path, open it as a new tab, and switch to it.
    fn open_file(&mut self) {
        let path = self.open_file_path.trim().to_string();
        self.mode = Mode::Normal;

        if path.is_empty() {
            return;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let filename = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&path)
                    .to_string();
                let path_buf = PathBuf::from(&path);
                if let Some(watcher) = &mut self.file_watcher {
                    watcher.watch_file(&path_buf);
                }
                self.documents
                    .push(Document::with_path(content, Some(filename), path_buf));
                self.active_doc = self.documents.len() - 1;
                self.open_file_path.clear();
                self.open_file_cursor = 0;
                self.exec_query();
            }
            Err(err) => {
                self.transient_error = Some(format!("Could not open '{}': {}", path, err));
            }
        }
    }

    fn init_tree_view(&mut self) {
        let markdown_result = Markdown::from_markdown_str(&self.active_doc().content);
        match markdown_result {
            Ok(markdown) => {
                self.tree_view = Some(TreeView::new(markdown.nodes));
            }
            Err(_) => {
                self.transient_error = Some("Failed to parse markdown for tree view".to_string());
            }
        }
    }

    fn handle_preview_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Mouse(mouse_event) = event {
            match mouse_event.kind {
                MouseEventKind::ScrollDown => {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                MouseEventKind::ScrollUp => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                _ => {}
            }
            self.clamp_preview_scroll();
            return Ok(());
        }

        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                // Exit preview mode
                (KeyCode::Esc, _) | (KeyCode::Char('p'), _) => {
                    self.mode = Mode::Normal;
                }
                // Quit
                (KeyCode::Char('q'), _) => {
                    self.should_quit = true;
                }
                // Switch tabs
                (KeyCode::Right, _) | (KeyCode::Tab, _) => {
                    self.next_tab();
                    self.preview_scroll = 0;
                }
                (KeyCode::Left, _) | (KeyCode::BackTab, _) => {
                    self.prev_tab();
                    self.preview_scroll = 0;
                }
                // Scroll
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    self.preview_scroll = self.preview_scroll.saturating_add(1);
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(1);
                }
                // Full page (less/vim-pager style: f/Space forward, b back)
                (KeyCode::PageDown, _) | (KeyCode::Char('f'), _) | (KeyCode::Char(' '), _) => {
                    let page = self.preview_page_lines();
                    self.preview_scroll = self.preview_scroll.saturating_add(page);
                }
                (KeyCode::PageUp, _) | (KeyCode::Char('b'), _) => {
                    let page = self.preview_page_lines();
                    self.preview_scroll = self.preview_scroll.saturating_sub(page);
                }
                // Half page (less/vim-pager style: d down, u up)
                (KeyCode::Char('d'), _) => {
                    let half = (self.preview_page_lines() / 2).max(1);
                    self.preview_scroll = self.preview_scroll.saturating_add(half);
                }
                (KeyCode::Char('u'), _) => {
                    let half = (self.preview_page_lines() / 2).max(1);
                    self.preview_scroll = self.preview_scroll.saturating_sub(half);
                }
                // Jump to top/bottom (vim-style)
                (KeyCode::Char('g'), _) => {
                    self.preview_scroll = 0;
                }
                (KeyCode::Char('G'), _) => {
                    self.preview_scroll = u16::MAX;
                }
                // Show help
                (KeyCode::Char('?'), _) | (KeyCode::F(1), _) => {
                    self.mode = Mode::Help;
                }
                // Toggle source/preview split
                (KeyCode::Char('s'), _) => {
                    self.preview_split = !self.preview_split;
                }
                // Adjust the source/preview split ratio
                (KeyCode::Char('<'), _) if self.preview_split => {
                    self.adjust_preview_split(-5);
                }
                (KeyCode::Char('>'), _) if self.preview_split => {
                    self.adjust_preview_split(5);
                }
                // Enter query mode without leaving the preview
                (KeyCode::Char(':'), _) => {
                    self.query_return_mode = Mode::Preview;
                    self.mode = Mode::Query;
                    self.cursor_position = self.query.len();
                }
                _ => {}
            }
            self.clamp_preview_scroll();
        }

        Ok(())
    }

    /// Whether the preview shows raw source side-by-side with the rendered output
    pub fn preview_split(&self) -> bool {
        self.preview_split
    }

    /// The active color theme, derived from persisted config.
    pub fn theme(&self) -> Theme {
        Theme::from_name(self.config.theme)
    }

    pub fn theme_name(&self) -> ThemeName {
        self.config.theme
    }

    /// Session-only theme override (e.g. `--theme`); does not touch config.toml.
    pub fn set_theme_name(&mut self, name: ThemeName) {
        self.config.theme = name;
    }

    /// Whether the persistent, mode-specific key-hint bar is shown above the status line.
    pub fn show_hint_bar(&self) -> bool {
        self.config.show_hint_bar
    }

    /// Session-only hint-bar override (e.g. `--no-hints`); does not touch config.toml.
    pub fn set_show_hint_bar(&mut self, show: bool) {
        self.config.show_hint_bar = show;
    }

    /// Percentage width of the results list in the results/detail split view.
    pub fn detail_split_percent(&self) -> u16 {
        self.detail_split_percent
    }

    /// Percentage width of the source pane in the preview/source split view.
    pub fn preview_split_percent(&self) -> u16 {
        self.preview_split_percent
    }

    pub fn sidebar_split_percent(&self) -> u16 {
        self.sidebar_split_percent
    }

    /// Widen/narrow the results pane relative to the detail pane, in 5% steps.
    fn adjust_detail_split(&mut self, delta: i16) {
        self.detail_split_percent = self
            .detail_split_percent
            .saturating_add_signed(delta)
            .clamp(15, 85);
    }

    /// Widen/narrow the source pane relative to the rendered preview, in 5% steps.
    fn adjust_preview_split(&mut self, delta: i16) {
        self.preview_split_percent = self
            .preview_split_percent
            .saturating_add_signed(delta)
            .clamp(15, 85);
    }

    fn adjust_sidebar_split(&mut self, delta: i16) {
        self.sidebar_split_percent = self
            .sidebar_split_percent
            .saturating_add_signed(delta)
            .clamp(10, 50);
    }

    /// Get the current preview scroll offset (in rendered lines)
    pub fn preview_scroll(&self) -> u16 {
        self.preview_scroll
    }

    /// Record the height of the preview viewport from the last draw, so that
    /// scrolling can be clamped to the document's actual rendered length.
    pub fn set_preview_viewport_height(&self, height: u16) {
        self.preview_viewport_height.set(height);
    }

    /// Get the content of the active document (used to render the preview)
    pub fn active_doc_content(&self) -> &str {
        &self.active_doc().content
    }

    fn preview_total_lines(&self) -> usize {
        preview::render_preview(&self.active_doc().content, &self.theme()).len()
    }

    fn clamp_preview_scroll(&mut self) {
        let total = self.preview_total_lines();
        let viewport = self.preview_viewport_height.get().saturating_sub(2).max(1) as usize;
        let max_scroll = total.saturating_sub(viewport) as u16;
        self.preview_scroll = self.preview_scroll.min(max_scroll);
    }

    /// Lines per page/half-page scroll, based on the last-drawn viewport height.
    fn preview_page_lines(&self) -> u16 {
        self.preview_viewport_height.get().saturating_sub(2).max(1)
    }

    /// Update results based on current sidebar selection
    fn update_sidebar_selection(&mut self) {
        if !self.show_tree_sidebar {
            return;
        }

        let active = self.active_doc;
        let selection = self.documents[active]
            .sidebar_tree_view
            .as_ref()
            .and_then(|sidebar| {
                let selected_item = sidebar.items().get(sidebar.selected_index())?;
                if selected_item.is_all_documents {
                    return Some(SidebarSelection::AllDocuments);
                }
                let selected_node = sidebar.get_selected_node()?;
                if let mq_markdown::Node::Heading(heading) = selected_node {
                    Some(SidebarSelection::Heading(
                        heading.clone(),
                        selected_node.value(),
                    ))
                } else {
                    None
                }
            });

        match selection {
            Some(SidebarSelection::AllDocuments) => {
                self.query.clear();
                self.cursor_position = 0;
                self.exec_query();
            }
            Some(SidebarSelection::Heading(heading, value)) => {
                let section_content = self.documents[active].extract_section_content(&heading);
                self.query = format!(
                    r#"import "section" | nodes | section::split({}) | section::title_contains("{}") | section::collect()"#,
                    heading.depth, value
                );
                self.documents[active].results = section_content;
                self.documents[active].selected_idx = 0;
                self.cursor_position = self.query.len();
            }
            None => {}
        }
    }

    /// Execute the current query against every open document.
    pub fn exec_query(&mut self) {
        self.query_pending = false;
        let start = Instant::now();

        for doc in self.documents.iter_mut() {
            let mut engine: Engine = Engine::default();
            engine.load_builtin_module();
            let markdown_result = Markdown::from_markdown_str(&doc.content);
            match markdown_result {
                Ok(markdown) => {
                    if !self.query.is_empty() {
                        let md_nodes = markdown
                            .nodes
                            .into_iter()
                            .map(mq_lang::RuntimeValue::from)
                            .collect::<Vec<_>>();

                        match engine.eval(&self.query, md_nodes.into_iter()) {
                            Ok(results) => {
                                doc.results = results
                                    .into_iter()
                                    .map(|runtime_value| match runtime_value {
                                        mq_lang::RuntimeValue::Markdown(node, _) => (*node).clone(),
                                        _ => runtime_value.to_string().into(),
                                    })
                                    .collect();
                                doc.error_msg = None;
                            }
                            Err(err) => {
                                doc.error_msg = Some(format!("Query error: {}", err));
                                // Keep previous results
                            }
                        }
                    } else {
                        // Show all nodes when query is empty
                        doc.results = markdown.nodes;
                        doc.error_msg = None;
                    }
                }
                Err(err) => {
                    doc.error_msg = Some(format!("Markdown parse error: {}", err));
                    doc.results = Vec::new();
                }
            }

            // Reset selected index if it's now out of bounds
            if doc.selected_idx >= doc.results.len() {
                doc.selected_idx = doc.results.len().saturating_sub(1);
            }

            // Advance past invisible nodes (nodes that render to nothing in combined output)
            if !doc.results.is_empty() {
                doc.selected_idx = Self::next_visible_in(&doc.results, doc.selected_idx, true);
            }
        }

        self.update_tree_view_matches();

        self.last_exec_time = start.elapsed();
        self.last_exec = Instant::now();
    }

    /// Highlight, in the tree view, whichever of its nodes the current query
    /// selected - mirroring jid's real-time match highlighting - and jump the
    /// tree's selection to the first match while it's being watched live.
    fn update_tree_view_matches(&mut self) {
        let Some(tree_view) = self.tree_view.as_mut() else {
            return;
        };

        if self.query.trim().is_empty() {
            tree_view.clear_matched_nodes();
            return;
        }

        tree_view.set_matched_nodes(self.documents[self.active_doc].results.clone());

        let watching_live = self.mode == Mode::TreeView
            || (self.mode == Mode::Query && self.query_return_mode == Mode::TreeView);
        if watching_live && let Some(first) = tree_view.first_matched_index() {
            tree_view.set_selected_index(first);
        }
    }

    /// Get the current query string
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Get the current results (for the active document)
    pub fn results(&self) -> &[mq_markdown::Node] {
        &self.active_doc().results
    }

    /// Get the currently selected result index (for the active document)
    pub fn selected_idx(&self) -> usize {
        self.active_doc().selected_idx
    }

    /// Get the last execution time
    pub fn last_exec_time(&self) -> Duration {
        self.last_exec_time
    }

    /// Get the current error message, if any (for the active document)
    pub fn error_msg(&self) -> Option<&str> {
        self.transient_error
            .as_deref()
            .or_else(|| self.active_doc().error_msg.as_deref())
    }

    /// Get the current app mode
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Check if detailed view is enabled
    pub fn show_detail(&self) -> bool {
        self.show_detail
    }

    /// Get the cursor position in the query
    pub fn cursor_position(&self) -> usize {
        self.cursor_position
    }

    /// Get the filename of the active document, if any
    pub fn filename(&self) -> Option<&str> {
        self.active_doc().filename.as_deref()
    }

    /// Get the query history
    pub fn query_history(&self) -> &[String] {
        &self.query_history
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.cursor_position = self.query.len();
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    #[cfg(test)]
    pub fn set_results(&mut self, results: Vec<mq_markdown::Node>) {
        self.active_doc_mut().results = results;
    }

    #[cfg(test)]
    pub fn set_last_exec_time(&mut self, duration: Duration) {
        self.last_exec_time = duration;
    }

    #[cfg(test)]
    pub fn set_error_msg(&mut self, msg: String) {
        self.transient_error = Some(msg);
    }

    #[cfg(test)]
    pub fn set_cursor_position(&mut self, position: usize) {
        self.cursor_position = position;
    }

    /// Seed saved queries directly, bypassing disk load/save, for tests.
    #[cfg(test)]
    pub fn set_saved_queries(&mut self, queries: Vec<SavedQuery>) {
        self.saved_queries = queries;
    }

    /// Get the tree view, if available
    pub fn tree_view(&self) -> Option<&TreeView> {
        self.tree_view.as_ref()
    }

    /// Check if tree sidebar is shown
    pub fn show_tree_sidebar(&self) -> bool {
        self.show_tree_sidebar
    }

    /// Get the sidebar tree view of the active document, if available
    pub fn sidebar_tree_view(&self) -> Option<&TreeView> {
        self.active_doc().sidebar_tree_view.as_ref()
    }

    /// Get mutable reference to the active document's sidebar tree view
    pub fn sidebar_tree_view_mut(&mut self) -> Option<&mut TreeView> {
        self.active_doc_mut().sidebar_tree_view.as_mut()
    }

    /// Toggle tree sidebar visibility
    pub fn toggle_tree_sidebar(&mut self) {
        self.show_tree_sidebar = !self.show_tree_sidebar;
    }

    /// Number of currently open documents (tabs)
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Index of the currently active document (tab)
    pub fn active_doc_index(&self) -> usize {
        self.active_doc
    }

    /// Display names (filenames) for all open documents, in order
    pub fn document_names(&self) -> Vec<&str> {
        self.documents.iter().map(|d| d.display_name()).collect()
    }

    /// Path currently being typed in Mode::OpenFile
    pub fn open_file_path(&self) -> &str {
        &self.open_file_path
    }

    /// Cursor position within the open-file path input
    pub fn open_file_cursor(&self) -> usize {
        self.open_file_cursor
    }

    /// Switch to the next tab (document), wrapping around
    pub fn next_tab(&mut self) {
        if self.documents.len() <= 1 {
            return;
        }
        self.active_doc = (self.active_doc + 1) % self.documents.len();
    }

    /// Switch to the previous tab (document), wrapping around
    pub fn prev_tab(&mut self) {
        if self.documents.len() <= 1 {
            return;
        }
        self.active_doc = if self.active_doc == 0 {
            self.documents.len() - 1
        } else {
            self.active_doc - 1
        };
    }

    /// Move to next/previous visible node from current position.
    /// This skips invisible nodes until finding a visible one.
    fn next_visible_from_current(&self, forward: bool) -> usize {
        Self::next_visible_from_current_in(
            &self.active_doc().results,
            self.active_doc().selected_idx,
            forward,
        )
    }

    fn next_visible_from_current_in(
        results: &[mq_markdown::Node],
        start_idx: usize,
        forward: bool,
    ) -> usize {
        let len = results.len();
        if len == 0 {
            return 0;
        }

        // Calculate the current rendered line position
        let current_line = if start_idx == 0 {
            0
        } else {
            Markdown::new(results[..start_idx + 1].to_vec())
                .to_string()
                .lines()
                .count()
                .saturating_sub(
                    Markdown::new(vec![results[start_idx].clone()])
                        .to_string()
                        .lines()
                        .count()
                        .max(1),
                )
        };

        // Find next node that renders to a different line position
        let mut idx = start_idx;
        for _ in 0..len {
            // Move to next/previous position
            if forward {
                idx = (idx + 1) % len;
            } else {
                idx = if idx == 0 { len - 1 } else { idx - 1 };
            }

            // Check if this node is visible (renders to non-empty content)
            let rendered = Markdown::new(vec![results[idx].clone()]).to_string();
            if !rendered.trim().is_empty() {
                // Calculate the line position of this node
                let node_line = if idx == 0 {
                    0
                } else {
                    Markdown::new(results[..idx + 1].to_vec())
                        .to_string()
                        .lines()
                        .count()
                        .saturating_sub(rendered.lines().count().max(1))
                };

                // Return this node if it's at a different line position
                if node_line != current_line {
                    return idx;
                }
            }
        }

        start_idx // No different position found, stay put
    }

    /// Find the nearest visible result index from `start`, searching in `forward` direction.
    /// A node is invisible when `render_with_theme` skips it (renders to "" or only whitespace).
    /// Returns `start` unchanged if all results are invisible.
    fn next_visible(&self, start: usize, forward: bool) -> usize {
        Self::next_visible_in(&self.active_doc().results, start, forward)
    }

    fn next_visible_in(results: &[mq_markdown::Node], start: usize, forward: bool) -> usize {
        let len = results.len();
        if len == 0 {
            return 0;
        }
        let mut idx = start;
        let mut checked = 0;

        while checked < len {
            let rendered = Markdown::new(vec![results[idx].clone()]).to_string();

            // A node is visible if it renders to non-empty, non-whitespace content
            // We need to check both the raw length and trimmed length
            let is_visible = !rendered.trim().is_empty();

            if is_visible {
                return idx;
            }

            // Move to next position
            if forward {
                idx = (idx + 1) % len;
            } else {
                idx = if idx == 0 { len - 1 } else { idx - 1 };
            }
            checked += 1;
        }

        start // all invisible: stay put
    }

    /// Range of the identifier-like word (`[A-Za-z0-9_.]`) ending at `cursor`.
    fn word_range_at_cursor(query: &str, cursor: usize) -> (usize, usize) {
        let cursor = cursor.min(query.len());
        let mut start = cursor;
        while start > 0 {
            let c = query[..start].chars().last().unwrap();
            if c.is_alphanumeric() || c == '_' || c == '.' {
                start -= c.len_utf8();
            } else {
                break;
            }
        }
        (start, cursor)
    }

    /// The word at the cursor that completion candidates are matched against,
    /// so the UI can bold the matched prefix within each candidate's name.
    pub fn completion_prefix(&self) -> &str {
        let (start, end) = Self::word_range_at_cursor(&self.query, self.cursor_position);
        &self.query[start..end]
    }

    /// Completion candidates (name, description) for the word at the cursor:
    /// selectors (`.h`, ...) if it starts with `.`, else builtin functions.
    pub fn query_completions(&self) -> Vec<(String, String)> {
        let (start, end) = Self::word_range_at_cursor(&self.query, self.cursor_position);
        let word = &self.query[start..end];
        if word.is_empty() {
            return Vec::new();
        }

        let mut matches: Vec<(String, String)> = if word.starts_with('.') {
            BUILTIN_SELECTOR_DOC
                .iter()
                .filter(|(name, _)| name.starts_with(word))
                .map(|(name, doc)| (name.to_string(), doc.description.to_string()))
                .collect()
        } else {
            BUILTIN_FUNCTION_DOC
                .iter()
                .filter(|(name, _)| name.starts_with(word))
                .map(|(name, doc)| (name.to_string(), doc.description.to_string()))
                .collect()
        };
        matches.sort();
        matches.truncate(8);
        matches
    }

    /// Completion candidates to display: the active cycle's list while the
    /// user is Tab-cycling, otherwise a fresh match against the cursor word.
    pub fn active_completions(&self) -> Vec<(String, String)> {
        match &self.completion_cycle {
            Some(cycle) => cycle.matches.clone(),
            None => self.query_completions(),
        }
    }

    /// Index within `active_completions()` that is currently selected.
    pub fn completion_selected_index(&self) -> usize {
        self.completion_cycle.as_ref().map_or(0, |c| c.index)
    }

    /// Remaining characters of the top completion candidate, for inline
    /// "ghost text". `None` unless the cursor is at the end of the query
    /// (otherwise the suggestion would overlap text after the cursor).
    pub fn ghost_suffix(&self) -> Option<String> {
        if self.completion_cycle.is_some() || self.cursor_position != self.query.len() {
            return None;
        }
        let (start, end) = Self::word_range_at_cursor(&self.query, self.cursor_position);
        let word = &self.query[start..end];
        if word.is_empty() {
            return None;
        }
        let top = self.query_completions().into_iter().next()?;
        if top.0.len() > word.len() {
            Some(top.0[word.len()..].to_string())
        } else {
            None
        }
    }

    fn handle_search_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                (KeyCode::Esc, _) => {
                    self.restore_search_origin();
                    self.mode = self.search_return_mode;
                }
                (KeyCode::Enter, _) => {
                    if !self.search_query.is_empty() {
                        self.last_search = Some(self.search_query.clone());
                    }
                    self.mode = self.search_return_mode;
                }
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    self.search_query.insert(self.search_cursor, c);
                    self.search_cursor += 1;
                    self.apply_incremental_search();
                }
                (KeyCode::Backspace, _) if self.search_cursor > 0 => {
                    self.search_query.remove(self.search_cursor - 1);
                    self.search_cursor -= 1;
                    self.apply_incremental_search();
                }
                (KeyCode::Delete, _) if self.search_cursor < self.search_query.len() => {
                    self.search_query.remove(self.search_cursor);
                    self.apply_incremental_search();
                }
                (KeyCode::Left, _) if self.search_cursor > 0 => {
                    self.search_cursor -= 1;
                }
                (KeyCode::Right, _) if self.search_cursor < self.search_query.len() => {
                    self.search_cursor += 1;
                }
                (KeyCode::Home, _) => {
                    self.search_cursor = 0;
                }
                (KeyCode::End, _) => {
                    self.search_cursor = self.search_query.len();
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Re-search from the origin position as the search text changes.
    fn apply_incremental_search(&mut self) {
        if self.search_query.is_empty() {
            self.restore_search_origin();
            return;
        }

        match self.search_return_mode {
            Mode::TreeView => {
                if let Some(tree_view) = &mut self.tree_view
                    && let Some(idx) =
                        tree_view.find_match(self.search_origin_idx, true, &self.search_query)
                {
                    tree_view.set_selected_index(idx);
                }
            }
            _ => {
                if let Some((doc_idx, idx)) = Self::find_match_across_tabs(
                    &self.documents,
                    self.search_origin_doc,
                    self.search_origin_idx,
                    true,
                    &self.search_query,
                ) {
                    self.active_doc = doc_idx;
                    self.active_doc_mut().selected_idx = idx;
                }
            }
        }
    }

    /// Restore the selection to before Mode::Search was entered (Esc).
    fn restore_search_origin(&mut self) {
        match self.search_return_mode {
            Mode::TreeView => {
                if let Some(tree_view) = &mut self.tree_view {
                    tree_view.set_selected_index(self.search_origin_idx);
                }
            }
            _ => {
                self.active_doc = self.search_origin_doc;
                self.active_doc_mut().selected_idx = self.search_origin_idx;
            }
        }
    }

    /// `n` / `N`: jump to the next/previous match of `last_search`.
    fn repeat_search(&mut self, forward: bool) {
        let Some(term) = self.last_search.clone() else {
            return;
        };

        // Step past the current position so we don't re-select the same match.
        fn step(current: usize, len: usize, forward: bool) -> Option<usize> {
            if len == 0 {
                return None;
            }
            Some(if forward {
                (current + 1) % len
            } else if current == 0 {
                len - 1
            } else {
                current - 1
            })
        }

        match self.search_return_mode {
            Mode::TreeView => {
                if let Some(tree_view) = &mut self.tree_view
                    && let Some(start) =
                        step(tree_view.selected_index(), tree_view.items().len(), forward)
                    && let Some(idx) = tree_view.find_match(start, forward, &term)
                {
                    tree_view.set_selected_index(idx);
                }
            }
            _ => {
                let results = &self.active_doc().results;
                let start =
                    step(self.active_doc().selected_idx, results.len(), forward).unwrap_or(0);
                if let Some((doc_idx, idx)) = Self::find_match_across_tabs(
                    &self.documents,
                    self.active_doc,
                    start,
                    forward,
                    &term,
                ) {
                    self.active_doc = doc_idx;
                    self.active_doc_mut().selected_idx = idx;
                }
            }
        }
    }

    /// Nearest result containing `term` (case-insensitive) from `start`, wrapping.
    fn find_match_idx(
        results: &[mq_markdown::Node],
        start: usize,
        forward: bool,
        term: &str,
    ) -> Option<usize> {
        let len = results.len();
        if len == 0 || term.is_empty() {
            return None;
        }
        let term_lower = term.to_lowercase();
        let start = start.min(len - 1);
        let mut idx = start;

        for step in 0..len {
            if step > 0 {
                idx = if forward {
                    (idx + 1) % len
                } else if idx == 0 {
                    len - 1
                } else {
                    idx - 1
                };
            }
            let rendered = Markdown::new(vec![results[idx].clone()])
                .to_string()
                .to_lowercase();
            if rendered.contains(&term_lower) {
                return Some(idx);
            }
        }

        None
    }

    /// Like `find_match_idx`, but when `documents[start_doc]` has no match it
    /// continues into the following tabs (wrapping back to `start_doc`),
    /// returning the tab index alongside the match index.
    fn find_match_across_tabs(
        documents: &[Document],
        start_doc: usize,
        start_idx: usize,
        forward: bool,
        term: &str,
    ) -> Option<(usize, usize)> {
        let doc_count = documents.len();
        if doc_count == 0 || term.is_empty() {
            return None;
        }

        if let Some(idx) =
            Self::find_match_idx(&documents[start_doc].results, start_idx, forward, term)
        {
            return Some((start_doc, idx));
        }

        for step in 1..doc_count {
            let doc_idx = if forward {
                (start_doc + step) % doc_count
            } else {
                (start_doc + doc_count - step) % doc_count
            };
            let results = &documents[doc_idx].results;
            let start = if forward {
                0
            } else {
                results.len().saturating_sub(1)
            };
            if let Some(idx) = Self::find_match_idx(results, start, forward, term) {
                return Some((doc_idx, idx));
            }
        }

        None
    }

    /// Current incremental search text
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Cursor position within the search text
    pub fn search_cursor(&self) -> usize {
        self.search_cursor
    }

    /// The last committed search term, if any (used to highlight matches)
    pub fn last_search(&self) -> Option<&str> {
        self.last_search.as_deref()
    }

    fn handle_save_query_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match (code, modifiers) {
                (KeyCode::Esc, _) => {
                    self.mode = Mode::Normal;
                }
                (KeyCode::Enter, _) => {
                    let name = self.save_query_name.trim().to_string();
                    if !name.is_empty() {
                        self.saved_queries.retain(|q| q.name != name);
                        self.saved_queries.push(SavedQuery {
                            name,
                            query: self.query.clone(),
                        });
                        if let Err(err) = crate::favorites::save(&self.saved_queries) {
                            self.transient_error = Some(format!("Could not save query: {err}"));
                        }
                    }
                    self.mode = Mode::Normal;
                }
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    self.save_query_name.insert(self.save_query_cursor, c);
                    self.save_query_cursor += 1;
                }
                (KeyCode::Backspace, _) if self.save_query_cursor > 0 => {
                    self.save_query_name.remove(self.save_query_cursor - 1);
                    self.save_query_cursor -= 1;
                }
                (KeyCode::Delete, _) if self.save_query_cursor < self.save_query_name.len() => {
                    self.save_query_name.remove(self.save_query_cursor);
                }
                (KeyCode::Left, _) if self.save_query_cursor > 0 => {
                    self.save_query_cursor -= 1;
                }
                (KeyCode::Right, _) if self.save_query_cursor < self.save_query_name.len() => {
                    self.save_query_cursor += 1;
                }
                (KeyCode::Home, _) => {
                    self.save_query_cursor = 0;
                }
                (KeyCode::End, _) => {
                    self.save_query_cursor = self.save_query_name.len();
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_favorites_mode_event(&mut self, event: Event) -> miette::Result<()> {
        if let Event::Key(KeyEvent {
            code,
            modifiers: _,
            kind: KeyEventKind::Press,
            ..
        }) = event
        {
            match code {
                KeyCode::Esc | KeyCode::Char('F') => {
                    self.mode = Mode::Normal;
                }
                KeyCode::Char('q') => {
                    self.should_quit = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.favorites_selected + 1 < self.saved_queries.len() {
                        self.favorites_selected += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.favorites_selected = self.favorites_selected.saturating_sub(1);
                }
                KeyCode::Enter => {
                    if let Some(saved) = self.saved_queries.get(self.favorites_selected) {
                        self.query = saved.query.clone();
                        self.cursor_position = self.query.len();
                        self.mode = Mode::Normal;
                        self.exec_query();
                    }
                }
                KeyCode::Char('d') if self.favorites_selected < self.saved_queries.len() => {
                    self.saved_queries.remove(self.favorites_selected);
                    if let Err(err) = crate::favorites::save(&self.saved_queries) {
                        self.transient_error =
                            Some(format!("Could not update saved queries: {err}"));
                    }
                    self.favorites_selected = self
                        .favorites_selected
                        .min(self.saved_queries.len().saturating_sub(1));
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Text currently typed for the name of the query being saved
    pub fn save_query_name(&self) -> &str {
        &self.save_query_name
    }

    /// Cursor position within the save-query name input
    pub fn save_query_cursor(&self) -> usize {
        self.save_query_cursor
    }

    /// Saved (favorite) queries, in order
    pub fn saved_queries(&self) -> &[SavedQuery] {
        &self.saved_queries
    }

    /// Currently selected index in Mode::Favorites
    pub fn favorites_selected(&self) -> usize {
        self.favorites_selected
    }

    /// Mode to return to when leaving Mode::Search; also what it searches.
    pub fn search_return_mode(&self) -> Mode {
        self.search_return_mode
    }

    /// Mode to return to when leaving Mode::Query (Normal, TreeView, or Preview).
    pub fn query_return_mode(&self) -> Mode {
        self.query_return_mode
    }

    /// Search term to highlight/jump to: in-progress text, else last committed.
    pub fn active_search_term(&self) -> Option<&str> {
        if self.mode == Mode::Search {
            Some(&self.search_query)
        } else {
            self.last_search.as_deref()
        }
    }
}
#[cfg(test)]
mod tests {
    use mq_markdown::Node;

    use super::*;

    fn create_test_app() -> App {
        App::new("# Test\nSome content".to_string())
    }

    fn create_test_app_with_file() -> App {
        App::with_file("# Test\nSome content".to_string(), "test.md".to_string())
    }

    #[test]
    fn test_app_creation() {
        let app = create_test_app();
        assert_eq!(app.query(), ".");
        assert_eq!(app.selected_idx(), 0);
        assert_eq!(app.mode(), Mode::Normal);
        assert!(!app.show_detail());
        assert_eq!(app.cursor_position(), 0);
        assert!(app.filename().is_none());
        assert!(app.error_msg().is_none());
    }

    #[test]
    fn test_app_with_file() {
        let app = create_test_app_with_file();
        assert_eq!(app.filename(), Some("test.md"));
    }

    #[test]
    fn test_app_with_multiple_files() {
        let app = App::with_files(vec![
            ("# One".to_string(), "one.md".to_string()),
            ("# Two".to_string(), "two.md".to_string()),
        ]);
        assert_eq!(app.document_count(), 2);
        assert_eq!(app.active_doc_index(), 0);
        assert_eq!(app.filename(), Some("one.md"));
        assert_eq!(app.document_names(), vec!["one.md", "two.md"]);
    }

    #[test]
    fn test_tab_switching() {
        let mut app = App::with_files(vec![
            ("# One".to_string(), "one.md".to_string()),
            ("# Two".to_string(), "two.md".to_string()),
            ("# Three".to_string(), "three.md".to_string()),
        ]);

        assert_eq!(app.active_doc_index(), 0);
        app.next_tab();
        assert_eq!(app.active_doc_index(), 1);
        app.next_tab();
        assert_eq!(app.active_doc_index(), 2);
        // Wraps around
        app.next_tab();
        assert_eq!(app.active_doc_index(), 0);

        app.prev_tab();
        assert_eq!(app.active_doc_index(), 2);
    }

    #[test]
    fn test_tab_switching_keys() {
        let mut app = App::with_files(vec![
            ("# One".to_string(), "one.md".to_string()),
            ("# Two".to_string(), "two.md".to_string()),
        ]);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.active_doc_index(), 1);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Left,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.active_doc_index(), 0);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.active_doc_index(), 1);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.active_doc_index(), 0);
    }

    #[test]
    fn test_query_applies_to_all_documents() {
        let mut app = App::with_files(vec![
            ("# One".to_string(), "one.md".to_string()),
            (
                "# Two\n\nbody\n\n## Three".to_string(),
                "two.md".to_string(),
            ),
        ]);

        // Identity query, executed once, should populate results for every
        // open document independently (based on each document's own content),
        // not just the active one.
        app.set_query(".".to_string());
        app.exec_query();

        assert_eq!(app.results().len(), 1);
        assert!(app.error_msg().is_none());

        app.next_tab();
        assert_eq!(app.results().len(), 3);
        assert!(app.error_msg().is_none());
    }

    #[test]
    fn test_mode_switching() {
        let mut app = create_test_app();

        // Normal to Query mode
        app.set_mode(Mode::Query);
        assert_eq!(app.mode(), Mode::Query);

        // Query to Help mode
        app.set_mode(Mode::Help);
        assert_eq!(app.mode(), Mode::Help);

        // Help to Normal mode
        app.set_mode(Mode::Normal);
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_query_setting() {
        let mut app = create_test_app();
        let test_query = "select(.type == 'heading')";

        app.set_query(test_query.to_string());
        assert_eq!(app.query(), test_query);
        assert_eq!(app.cursor_position(), test_query.len());
    }

    #[test]
    fn test_normal_mode_navigation() {
        let mut app = create_test_app();
        let test_results = vec![
            Node::from("result1"),
            Node::from("result2"),
            Node::from("result3"),
        ];
        app.set_results(test_results);

        // Test down navigation
        let down_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(down_event).unwrap();
        assert_eq!(app.selected_idx(), 1);

        // Test up navigation
        let up_event = Event::Key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(up_event).unwrap();
        assert_eq!(app.selected_idx(), 0);
    }

    #[test]
    fn test_normal_mode_vim_navigation() {
        let mut app = create_test_app();
        let test_results = vec![Node::from("result1"), Node::from("result2")];
        app.set_results(test_results);

        // Test j (down)
        let j_event = Event::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(j_event).unwrap();
        assert_eq!(app.selected_idx(), 1);

        // Test k (up)
        let k_event = Event::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(k_event).unwrap();
        assert_eq!(app.selected_idx(), 0);
    }

    #[test]
    fn test_normal_mode_page_navigation() {
        let mut app = create_test_app();
        let test_results = (0..20)
            .map(|i| Node::from(format!("result{}", i)))
            .collect();
        app.set_results(test_results);

        // Test PageDown
        let page_down_event = Event::Key(KeyEvent {
            code: KeyCode::PageDown,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(page_down_event).unwrap();
        assert_eq!(app.selected_idx(), 10);

        // Test PageUp
        let page_up_event = Event::Key(KeyEvent {
            code: KeyCode::PageUp,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(page_up_event).unwrap();
        assert_eq!(app.selected_idx(), 0);
    }

    #[test]
    fn test_normal_mode_home_end_navigation() {
        let mut app = create_test_app();
        let test_results = vec![
            Node::from("result1"),
            Node::from("result2"),
            Node::from("result3"),
        ];
        app.set_results(test_results);
        app.active_doc_mut().selected_idx = 1;

        // Test End
        let end_event = Event::Key(KeyEvent {
            code: KeyCode::End,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(end_event).unwrap();
        assert_eq!(app.selected_idx(), 2);

        // Test Home
        let home_event = Event::Key(KeyEvent {
            code: KeyCode::Home,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(home_event).unwrap();
        assert_eq!(app.selected_idx(), 0);
    }

    #[test]
    fn test_normal_mode_toggle_detail() {
        let mut app = create_test_app();
        assert!(!app.show_detail());

        let detail_event = Event::Key(KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(detail_event.clone()).unwrap();
        assert!(app.show_detail());

        // Toggle again
        app.handle_event(detail_event).unwrap();
        assert!(!app.show_detail());
    }

    #[test]
    fn test_normal_mode_enter_query_mode() {
        let mut app = create_test_app();

        let colon_event = Event::Key(KeyEvent {
            code: KeyCode::Char(':'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(colon_event).unwrap();
        assert_eq!(app.mode(), Mode::Query);
    }

    #[test]
    fn test_normal_mode_enter_help_mode() {
        let mut app = create_test_app();

        let help_event = Event::Key(KeyEvent {
            code: KeyCode::Char('?'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(help_event).unwrap();
        assert_eq!(app.mode(), Mode::Help);

        // Test F1 as well
        app.set_mode(Mode::Normal);
        let f1_event = Event::Key(KeyEvent {
            code: KeyCode::F(1),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(f1_event).unwrap();
        assert_eq!(app.mode(), Mode::Help);
    }

    #[test]
    fn test_normal_mode_clear_query() {
        let mut app = create_test_app();
        app.set_query("test query".to_string());

        let clear_event = Event::Key(KeyEvent {
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(clear_event).unwrap();
        assert_eq!(app.query(), "");
        assert_eq!(app.cursor_position(), 0);
    }

    #[test]
    fn test_query_mode_text_input() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);

        let char_event = Event::Key(KeyEvent {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(char_event).unwrap();
        assert_eq!(app.query(), "t.");
        assert_eq!(app.cursor_position(), 1);
    }

    #[test]
    fn test_query_mode_backspace() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("test".to_string());

        let backspace_event = Event::Key(KeyEvent {
            code: KeyCode::Backspace,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(backspace_event).unwrap();
        assert_eq!(app.query(), "tes");
        assert_eq!(app.cursor_position(), 3);
    }

    #[test]
    fn test_query_mode_delete() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("test".to_string());
        app.set_cursor_position(2);

        let delete_event = Event::Key(KeyEvent {
            code: KeyCode::Delete,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(delete_event).unwrap();
        assert_eq!(app.query(), "tet");
        assert_eq!(app.cursor_position(), 2);
    }

    #[test]
    fn test_query_mode_cursor_movement() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("test".to_string());

        // Test left arrow
        let left_event = Event::Key(KeyEvent {
            code: KeyCode::Left,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(left_event).unwrap();
        assert_eq!(app.cursor_position(), 3);

        // Test right arrow
        let right_event = Event::Key(KeyEvent {
            code: KeyCode::Right,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(right_event).unwrap();
        assert_eq!(app.cursor_position(), 4);
    }

    #[test]
    fn test_query_mode_home_end() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("test".to_string());
        app.set_cursor_position(2);

        // Test Home
        let home_event = Event::Key(KeyEvent {
            code: KeyCode::Home,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(home_event).unwrap();
        assert_eq!(app.cursor_position(), 0);

        // Test End
        let end_event = Event::Key(KeyEvent {
            code: KeyCode::End,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(end_event).unwrap();
        assert_eq!(app.cursor_position(), 4);
    }

    #[test]
    fn test_query_mode_exit_on_escape() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);

        let escape_event = Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(escape_event).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_query_mode_execute_on_enter() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("test query".to_string());

        let enter_event = Event::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(enter_event).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
        assert!(app.query_history().contains(&"test query".to_string()));
    }

    #[test]
    fn test_help_mode_exit_on_any_key() {
        let mut app = create_test_app();
        app.set_mode(Mode::Help);

        let any_key_event = Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(any_key_event).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_quit_on_q_or_escape() {
        let mut app = create_test_app();

        let q_event = Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(q_event).unwrap();
        assert!(app.should_quit);

        // Reset and test escape
        app.should_quit = false;
        let escape_event = Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(escape_event).unwrap();
        assert!(app.should_quit);
    }

    #[test]
    fn test_error_message_cleared_on_event() {
        let mut app = create_test_app();
        app.set_error_msg("Test error".to_string());
        assert!(app.error_msg().is_some());

        let any_event = Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(any_event).unwrap();
        assert!(app.error_msg().is_none());
    }

    #[test]
    fn test_navigation_with_empty_results() {
        let mut app = create_test_app();
        app.set_results(vec![]);

        let down_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(down_event).unwrap();
        assert_eq!(app.selected_idx(), 0);
    }

    #[test]
    fn test_navigation_wraparound() {
        let mut app = create_test_app();
        let test_results = vec!["result1".into(), Node::from("result2")];
        app.set_results(test_results);
        app.active_doc_mut().selected_idx = 1;

        let down_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(down_event).unwrap();
        assert_eq!(app.selected_idx(), 0);

        let up_event = Event::Key(KeyEvent {
            code: KeyCode::Up,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(up_event).unwrap();
        assert_eq!(app.selected_idx(), 1);
    }

    #[test]
    fn test_execution_time_tracking() {
        let mut app = create_test_app();
        let test_duration = Duration::from_millis(100);
        app.set_last_exec_time(test_duration);
        assert_eq!(app.last_exec_time(), test_duration);
    }

    #[test]
    fn test_query_history_functionality() {
        let app = create_test_app();
        assert!(app.query_history().is_empty());
    }

    #[test]
    fn test_tree_view_mode() {
        let mut app = create_test_app();
        assert_eq!(app.mode(), Mode::Normal);
        assert!(app.tree_view().is_none());

        // Switch to tree view mode
        app.set_mode(Mode::TreeView);
        app.init_tree_view();

        assert_eq!(app.mode(), Mode::TreeView);
        assert!(app.tree_view().is_some());
    }

    #[test]
    fn test_tree_view_mode_navigation() {
        let mut app = create_test_app();
        app.set_mode(Mode::TreeView);
        app.init_tree_view();

        // Test tree view navigation keys
        let down_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(down_event).unwrap();
        assert_eq!(app.mode(), Mode::TreeView);

        // Test exiting tree view
        let escape_event = Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(escape_event).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_tree_view_toggle_from_normal_mode() {
        let mut app = create_test_app();

        let tree_toggle_event = Event::Key(KeyEvent {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(tree_toggle_event).unwrap();
        assert_eq!(app.mode(), Mode::TreeView);
        assert!(app.tree_view().is_some());
    }

    #[test]
    fn test_colon_enters_query_mode_from_tree_view_and_returns_to_it() {
        let mut app = create_test_app();
        app.handle_event(key(KeyCode::Char('t'))).unwrap();
        assert_eq!(app.mode(), Mode::TreeView);

        app.handle_event(key(KeyCode::Char(':'))).unwrap();
        assert_eq!(app.mode(), Mode::Query);
        assert_eq!(app.query_return_mode(), Mode::TreeView);

        app.handle_event(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.mode(), Mode::TreeView);
    }

    #[test]
    fn test_colon_enters_query_mode_from_preview_and_returns_to_it() {
        let mut app = create_test_app();
        app.handle_event(key(KeyCode::Char('p'))).unwrap();
        assert_eq!(app.mode(), Mode::Preview);

        app.handle_event(key(KeyCode::Char(':'))).unwrap();
        assert_eq!(app.mode(), Mode::Query);
        assert_eq!(app.query_return_mode(), Mode::Preview);

        app.handle_event(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode(), Mode::Preview);
    }

    #[test]
    fn test_live_query_highlights_matching_tree_nodes_and_jumps_to_first() {
        let mut app = App::new("# Alpha\n\nbody text\n\n# Beta\n".to_string());
        app.handle_event(key(KeyCode::Char('t'))).unwrap();
        app.handle_event(key(KeyCode::Char(':'))).unwrap();

        app.set_query(".h".to_string());
        app.exec_query();

        assert_eq!(app.mode(), Mode::Query);
        let tree_view = app.tree_view().unwrap();
        // Both headings should be the live query's matches...
        assert_eq!(tree_view.first_matched_index(), Some(0));
        // ...and since the tree is being watched live, selection jumped there.
        assert_eq!(tree_view.selected_index(), 0);
    }

    #[test]
    fn test_clearing_live_query_clears_tree_matches() {
        let mut app = App::new("# Alpha\n\nbody text\n".to_string());
        app.handle_event(key(KeyCode::Char('t'))).unwrap();
        app.handle_event(key(KeyCode::Char(':'))).unwrap();

        app.set_query(".h".to_string());
        app.exec_query();
        assert!(app.tree_view().unwrap().first_matched_index().is_some());

        app.set_query(String::new());
        app.exec_query();
        assert_eq!(app.tree_view().unwrap().first_matched_index(), None);
    }

    #[test]
    fn test_open_file_mode_enter_and_cancel() {
        let mut app = create_test_app();

        let o_event = Event::Key(KeyEvent {
            code: KeyCode::Char('o'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(o_event).unwrap();
        assert_eq!(app.mode(), Mode::OpenFile);

        let escape_event = Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(escape_event).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.document_count(), 1);
    }

    #[test]
    fn test_open_file_mode_opens_new_tab() {
        let mut app = create_test_app();

        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir.join(format!("mq_tui_test_{}.md", std::process::id()));
        std::fs::write(&file_path, "# Opened\n").unwrap();

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Char('o'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();

        for c in file_path.to_string_lossy().chars() {
            app.handle_event(Event::Key(KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: crossterm::event::KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            }))
            .unwrap();
        }

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();

        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.document_count(), 2);
        assert_eq!(app.active_doc_index(), 1);

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn test_normal_mode_ignores_release_events() {
        let mut app = create_test_app();
        let test_results = vec![
            Node::from("result1"),
            Node::from("result2"),
            Node::from("result3"),
        ];
        app.set_results(test_results);
        let initial_idx = app.selected_idx();

        // Send a Release event for Down key
        let release_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_event).unwrap();

        // Index should not change because Release events are ignored
        assert_eq!(app.selected_idx(), initial_idx);

        // Verify Press event still works
        let press_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(press_event).unwrap();
        assert_eq!(app.selected_idx(), initial_idx + 1);
    }

    #[test]
    fn test_normal_mode_ignores_repeat_events() {
        let mut app = create_test_app();
        app.mode = Mode::Normal;
        let initial_mode = app.mode();

        // Send a Repeat event for mode switch key
        let repeat_event = Event::Key(KeyEvent {
            code: KeyCode::Char(':'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Repeat,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(repeat_event).unwrap();

        // Mode should not change because Repeat events are ignored
        assert_eq!(app.mode(), initial_mode);
    }

    #[test]
    fn test_query_mode_ignores_release_events() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("test".to_string());
        let initial_query = app.query().to_string();

        // Send a Release event for backspace
        let release_event = Event::Key(KeyEvent {
            code: KeyCode::Backspace,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_event).unwrap();

        // Query should not change because Release events are ignored
        assert_eq!(app.query(), initial_query);

        // Verify Press event still works
        let press_event = Event::Key(KeyEvent {
            code: KeyCode::Backspace,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(press_event).unwrap();
        assert_eq!(app.query(), "tes");
    }

    #[test]
    fn test_query_mode_char_input_ignores_release() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("".to_string());

        // Send Release event for character input
        let release_event = Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_event).unwrap();

        // Query should remain empty because Release events are ignored
        assert_eq!(app.query(), "");

        // Verify Press event adds the character
        let press_event = Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(press_event).unwrap();
        assert_eq!(app.query(), "x");
    }

    #[test]
    fn test_help_mode_ignores_release_events() {
        let mut app = create_test_app();
        app.set_mode(Mode::Help);

        // Send a Release event
        let release_event = Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_event).unwrap();

        // Mode should not change because Release events are ignored
        assert_eq!(app.mode(), Mode::Help);

        // Verify Press event exits help mode
        let press_event = Event::Key(KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(press_event).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_tree_view_mode_ignores_release_events() {
        let mut app = create_test_app();
        app.set_mode(Mode::TreeView);
        app.init_tree_view();

        // Send a Release event for navigation
        let release_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_event).unwrap();

        // Should still be in tree view mode
        assert_eq!(app.mode(), Mode::TreeView);

        // Send Release event for Escape (exit tree view)
        let escape_release = Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(escape_release).unwrap();

        // Mode should not change because Release events are ignored
        assert_eq!(app.mode(), Mode::TreeView);

        // Verify Press event exits tree view
        let escape_press = Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(escape_press).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_windows_double_input_scenario() {
        // This test simulates the Windows environment where both Press and Release
        // events are generated for a single key press (issue #1)
        let mut app = create_test_app();
        let test_results = vec![Node::from("result1"), Node::from("result2")];
        app.set_results(test_results);
        assert_eq!(app.selected_idx(), 0);

        // Simulate Windows: Press event followed by Release event
        let press_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(press_event).unwrap();
        assert_eq!(app.selected_idx(), 1); // Should move to next item

        let release_event = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_event).unwrap();
        assert_eq!(app.selected_idx(), 1); // Should NOT move again

        // Verify the fix: only one navigation occurred instead of two
    }

    #[test]
    fn test_query_mode_windows_double_input_scenario() {
        // Test query mode with Windows-style Press/Release events
        let mut app = create_test_app();
        app.set_mode(Mode::Query);

        // Simulate typing 'j' with Press event
        let press_j = Event::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(press_j).unwrap();
        assert_eq!(app.query(), "j.");

        // Simulate Release event for 'j'
        let release_j = Event::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_j).unwrap();
        assert_eq!(app.query(), "j."); // Should still be "j", not "jj"

        // Simulate typing 'k' with Press event
        let press_k = Event::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(press_k).unwrap();
        assert_eq!(app.query(), "jk.");

        // Simulate Release event for 'k'
        let release_k = Event::Key(KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        app.handle_event(release_k).unwrap();
        assert_eq!(app.query(), "jk."); // Should still be "jk", not "jkk"

        // Verify the fix: query is "jk" not "jjkk"
    }

    #[test]
    fn test_preview_mode_toggle_and_exit() {
        let mut app = create_test_app();
        assert_eq!(app.mode(), Mode::Normal);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.mode(), Mode::Preview);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_preview_mode_scroll_navigation() {
        let mut app = App::new("# Title\n\nSome body text".to_string());
        app.set_mode(Mode::Preview);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.preview_scroll(), 1);

        app.handle_event(Event::Key(KeyEvent {
            code: KeyCode::Char('g'),
            modifiers: KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }))
        .unwrap();
        assert_eq!(app.preview_scroll(), 0);
    }

    #[test]
    fn test_preview_mode_page_and_half_page_navigation() {
        let content = (0..50)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = App::new(content);
        app.set_mode(Mode::Preview);
        app.set_preview_viewport_height(22); // inner height = 20

        let press = |code: KeyCode| {
            Event::Key(KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: crossterm::event::KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            })
        };

        app.handle_event(press(KeyCode::Char('d'))).unwrap();
        assert_eq!(app.preview_scroll(), 10);

        app.handle_event(press(KeyCode::Char('u'))).unwrap();
        assert_eq!(app.preview_scroll(), 0);

        app.handle_event(press(KeyCode::Char('f'))).unwrap();
        assert_eq!(app.preview_scroll(), 20);

        app.handle_event(press(KeyCode::Char('b'))).unwrap();
        assert_eq!(app.preview_scroll(), 0);
    }

    #[test]
    fn test_preview_renders_active_document_content() {
        let app = App::new("# Hello".to_string());
        assert_eq!(app.active_doc_content(), "# Hello");
    }

    #[test]
    fn test_watch_disabled_by_default() {
        let app = create_test_app();
        assert!(!app.watch());
    }

    #[test]
    fn test_watch_reloads_changed_file() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir.join(format!("mq_tui_watch_test_{}.md", std::process::id()));
        std::fs::write(&file_path, "# Original\n").unwrap();

        let mut app = App::with_files_with_paths(vec![(
            "# Original\n".to_string(),
            "watch.md".to_string(),
            file_path.clone(),
        )]);
        app.set_watch(true);

        // Give the OS watcher a moment to register before writing.
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(&file_path, "# Updated\n").unwrap();

        // The debouncer waits ~300ms before emitting; poll a bit longer than
        // that for the change to be picked up.
        let mut reloaded = false;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(50));
            app.check_for_file_changes();
            if app.active_doc_content() == "# Updated\n" {
                reloaded = true;
                break;
            }
        }

        std::fs::remove_file(&file_path).ok();
        assert!(
            reloaded,
            "expected the document to reload after the file changed on disk"
        );
    }

    #[test]
    fn test_watch_without_changes_keeps_content() {
        let tmp_dir = std::env::temp_dir();
        let file_path = tmp_dir.join(format!(
            "mq_tui_watch_test_nochange_{}.md",
            std::process::id()
        ));
        std::fs::write(&file_path, "# Original\n").unwrap();

        let mut app = App::with_files_with_paths(vec![(
            "# Original\n".to_string(),
            "watch.md".to_string(),
            file_path.clone(),
        )]);
        app.set_watch(true);

        app.check_for_file_changes();

        assert_eq!(app.active_doc_content(), "# Original\n");

        std::fs::remove_file(&file_path).ok();
    }

    fn key_press(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        })
    }

    fn key(code: KeyCode) -> Event {
        key_press(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_query_completions_match_selector_prefix() {
        let mut app = create_test_app();
        app.set_query(".h".to_string());
        app.set_cursor_position(2);

        let completions = app.query_completions();
        assert!(completions.iter().any(|(name, _)| name == ".h"));
        assert!(completions.iter().all(|(name, _)| name.starts_with(".h")));
    }

    #[test]
    fn test_query_completions_match_function_prefix() {
        let mut app = create_test_app();
        app.set_query("flat".to_string());
        app.set_cursor_position(4);

        let completions = app.query_completions();
        assert!(completions.iter().any(|(name, _)| name == "flatten"));
    }

    #[test]
    fn test_query_completions_empty_word_has_no_suggestions() {
        let mut app = create_test_app();
        app.set_query("select(.a) | ".to_string());
        app.set_cursor_position(13);

        assert!(app.query_completions().is_empty());
    }

    #[test]
    fn test_tab_accepts_top_completion_in_query_mode() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("flat".to_string());
        app.set_cursor_position(4);

        app.handle_event(key(KeyCode::Tab)).unwrap();

        assert_eq!(app.query(), "flatten");
        assert_eq!(app.cursor_position(), "flatten".len());
    }

    #[test]
    fn test_tab_cycles_through_multiple_completions() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("base64".to_string());
        app.set_cursor_position(6);

        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64");

        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64d");

        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64url");

        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64urld");

        // Wraps back around to the first candidate.
        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64");
    }

    #[test]
    fn test_shift_tab_cycles_backward() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("base64".to_string());
        app.set_cursor_position(6);

        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64");

        // Backward from the first candidate wraps to the last.
        app.handle_event(key(KeyCode::BackTab)).unwrap();
        assert_eq!(app.query(), "base64urld");

        app.handle_event(key(KeyCode::BackTab)).unwrap();
        assert_eq!(app.query(), "base64url");
    }

    #[test]
    fn test_typing_after_completion_starts_a_fresh_cycle() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query("base64".to_string());
        app.set_cursor_position(6);

        app.handle_event(key(KeyCode::Tab)).unwrap();
        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64d");

        for c in " fla".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }
        assert_eq!(app.query(), "base64d fla");

        // Cycling again should be based on the new word ("fla"), not the
        // stale "base64*" candidate list from before.
        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.query(), "base64d flatten");
    }

    #[test]
    fn test_tab_switches_tabs_when_no_completion() {
        let mut app = App::with_files(vec![
            ("# One".to_string(), "one.md".to_string()),
            ("# Two".to_string(), "two.md".to_string()),
        ]);
        app.set_mode(Mode::Query);
        app.set_query("".to_string());

        app.handle_event(key(KeyCode::Tab)).unwrap();

        assert_eq!(app.active_doc_index(), 1);
    }

    fn create_app_for_search() -> App {
        let mut app = App::new("# Alpha\n\nneedle here\n\n# Beta\n".to_string());
        app.set_query(".".to_string());
        app.exec_query();
        app
    }

    #[test]
    fn test_slash_enters_search_mode_from_normal() {
        let mut app = create_app_for_search();
        app.handle_event(key(KeyCode::Char('/'))).unwrap();
        assert_eq!(app.mode(), Mode::Search);
    }

    #[test]
    fn test_incremental_search_jumps_to_match() {
        let mut app = create_app_for_search();
        app.handle_event(key(KeyCode::Char('/'))).unwrap();

        for c in "needle".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }

        assert_eq!(app.selected_idx(), 1);
    }

    #[test]
    fn test_search_esc_restores_original_selection() {
        let mut app = create_app_for_search();
        assert_eq!(app.selected_idx(), 0);

        app.handle_event(key(KeyCode::Char('/'))).unwrap();
        for c in "Beta".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }
        assert_eq!(app.selected_idx(), 2);

        app.handle_event(key(KeyCode::Esc)).unwrap();

        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.selected_idx(), 0);
    }

    #[test]
    fn test_search_enter_commits_and_repeat_with_n() {
        let mut app = App::new("# Alpha\n\nneedle one\n\nneedle two\n".to_string());
        app.set_query(".".to_string());
        app.exec_query();

        app.handle_event(key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }
        app.handle_event(key(KeyCode::Enter)).unwrap();

        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.selected_idx(), 1);

        app.handle_event(key(KeyCode::Char('n'))).unwrap();
        assert_eq!(app.selected_idx(), 2);

        app.handle_event(key_press(KeyCode::Char('N'), KeyModifiers::SHIFT))
            .unwrap();
        assert_eq!(app.selected_idx(), 1);
    }

    #[test]
    fn test_incremental_search_jumps_to_match_in_other_tab() {
        let mut app = App::with_files(vec![
            ("# Alpha\n\nnothing here\n".to_string(), "a.md".to_string()),
            ("# Beta\n\nneedle here\n".to_string(), "b.md".to_string()),
        ]);
        app.set_query(".".to_string());
        app.exec_query();
        assert_eq!(app.active_doc_index(), 0);

        app.handle_event(key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }

        assert_eq!(app.active_doc_index(), 1);
        assert_eq!(app.selected_idx(), 1);
    }

    #[test]
    fn test_search_esc_restores_original_tab() {
        let mut app = App::with_files(vec![
            ("# Alpha\n\nnothing here\n".to_string(), "a.md".to_string()),
            ("# Beta\n\nneedle here\n".to_string(), "b.md".to_string()),
        ]);
        app.set_query(".".to_string());
        app.exec_query();

        app.handle_event(key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }
        assert_eq!(app.active_doc_index(), 1);

        app.handle_event(key(KeyCode::Esc)).unwrap();

        assert_eq!(app.active_doc_index(), 0);
        assert_eq!(app.selected_idx(), 0);
    }

    #[test]
    fn test_repeat_search_falls_back_to_other_tab_with_no_local_match() {
        let mut app = App::with_files(vec![
            ("# Alpha\n\nneedle one\n".to_string(), "a.md".to_string()),
            ("# Beta\n\nno match here\n".to_string(), "b.md".to_string()),
        ]);
        app.set_query(".".to_string());
        app.exec_query();

        app.handle_event(key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }
        app.handle_event(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.active_doc_index(), 0);
        assert_eq!(app.selected_idx(), 1);

        // Manually switch to the tab with no match, then repeat-search.
        app.handle_event(key(KeyCode::Tab)).unwrap();
        assert_eq!(app.active_doc_index(), 1);

        app.handle_event(key(KeyCode::Char('n'))).unwrap();

        assert_eq!(app.active_doc_index(), 0);
        assert_eq!(app.selected_idx(), 1);
    }

    #[test]
    fn test_tree_view_search_jumps_selection() {
        let mut app = App::new("# Alpha\n\n## Needle Heading\n\n# Beta\n".to_string());
        app.handle_event(key(KeyCode::Char('t'))).unwrap();
        assert_eq!(app.mode(), Mode::TreeView);

        app.handle_event(key(KeyCode::Char('/'))).unwrap();
        assert_eq!(app.mode(), Mode::Search);
        assert_eq!(app.search_return_mode(), Mode::TreeView);

        for c in "Needle".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }
        app.handle_event(key(KeyCode::Enter)).unwrap();

        assert_eq!(app.mode(), Mode::TreeView);
        let selected = app.tree_view().unwrap().get_selected_node().unwrap();
        if let mq_markdown::Node::Heading(h) = selected {
            let text: String = h.values.iter().map(|n| n.value()).collect();
            assert!(text.contains("Needle Heading"));
        } else {
            panic!("expected the search to land on the heading node");
        }
    }

    #[test]
    fn test_favorites_navigation_and_enter_applies_query() {
        let mut app = create_test_app();
        app.set_saved_queries(vec![
            SavedQuery {
                name: "headings".to_string(),
                query: ".h".to_string(),
            },
            SavedQuery {
                name: "all".to_string(),
                query: ".".to_string(),
            },
        ]);

        app.handle_event(key(KeyCode::Char('F'))).unwrap();
        assert_eq!(app.mode(), Mode::Favorites);
        assert_eq!(app.favorites_selected(), 0);

        app.handle_event(key(KeyCode::Char('j'))).unwrap();
        assert_eq!(app.favorites_selected(), 1);

        app.handle_event(key(KeyCode::Char('k'))).unwrap();
        assert_eq!(app.favorites_selected(), 0);

        app.handle_event(key(KeyCode::Enter)).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.query(), ".h");
    }

    #[test]
    fn test_save_query_mode_name_input_and_cancel() {
        let mut app = create_test_app();
        app.set_query(".h".to_string());

        app.handle_event(key(KeyCode::Char('S'))).unwrap();
        assert_eq!(app.mode(), Mode::SaveQuery);

        for c in "my query".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }
        assert_eq!(app.save_query_name(), "my query");

        app.handle_event(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
        // Esc cancels without saving.
        assert!(!app.saved_queries().iter().any(|q| q.name == "my query"));
    }

    #[test]
    fn test_ctrl_k_opens_command_palette() {
        let mut app = create_test_app();
        app.handle_event(key_press(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.mode(), Mode::CommandPalette);
        assert!(!app.filtered_palette_actions().is_empty());
    }

    #[test]
    fn test_command_palette_filters_by_fuzzy_match() {
        let mut app = create_test_app();
        app.handle_event(key_press(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();

        for c in "tree".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }

        let matches = app.filtered_palette_actions();
        assert!(matches.iter().any(|a| a.label == "Tree view"));
        assert!(!matches.iter().any(|a| a.label == "Quit"));
    }

    #[test]
    fn test_command_palette_enter_runs_action() {
        let mut app = create_test_app();
        app.handle_event(key_press(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        for c in "tree".chars() {
            app.handle_event(key(KeyCode::Char(c))).unwrap();
        }

        app.handle_event(key(KeyCode::Enter)).unwrap();

        assert_eq!(app.mode(), Mode::TreeView);
    }

    #[test]
    fn test_command_palette_esc_cancels() {
        let mut app = create_test_app();
        app.handle_event(key_press(KeyCode::Char('k'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_event(key(KeyCode::Esc)).unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn test_ghost_suffix_suggests_top_completion_at_cursor_end() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query(".head".to_string());
        app.set_cursor_position(5);

        assert_eq!(app.ghost_suffix().as_deref(), Some("ing"));
    }

    #[test]
    fn test_ghost_suffix_none_when_cursor_not_at_end() {
        let mut app = create_test_app();
        app.set_mode(Mode::Query);
        app.set_query(".heading extra".to_string());
        app.set_cursor_position(5);

        assert_eq!(app.ghost_suffix(), None);
    }

    #[test]
    fn test_detail_split_adjust_clamped() {
        let mut app = create_test_app();
        app.handle_event(key(KeyCode::Char('d'))).unwrap();
        assert!(app.show_detail());
        assert_eq!(app.detail_split_percent(), 40);

        for _ in 0..20 {
            app.handle_event(key(KeyCode::Char('>'))).unwrap();
        }
        assert_eq!(app.detail_split_percent(), 85);

        for _ in 0..20 {
            app.handle_event(key(KeyCode::Char('<'))).unwrap();
        }
        assert_eq!(app.detail_split_percent(), 15);
    }

    #[test]
    fn test_sidebar_split_adjust_clamped() {
        let mut app = create_test_app();
        app.handle_event(key(KeyCode::Char('s'))).unwrap();
        assert!(app.show_tree_sidebar());
        assert_eq!(app.sidebar_split_percent(), 20);

        for _ in 0..20 {
            app.handle_event(key(KeyCode::Char('>'))).unwrap();
        }
        assert_eq!(app.sidebar_split_percent(), 50);

        for _ in 0..20 {
            app.handle_event(key(KeyCode::Char('<'))).unwrap();
        }
        assert_eq!(app.sidebar_split_percent(), 10);
    }

    #[test]
    fn test_split_resize_prioritizes_sidebar_over_detail() {
        let mut app = create_test_app();
        app.handle_event(key(KeyCode::Char('s'))).unwrap();
        app.handle_event(key(KeyCode::Char('d'))).unwrap();
        assert!(app.show_tree_sidebar());
        assert!(app.show_detail());

        app.handle_event(key(KeyCode::Char('>'))).unwrap();

        assert_eq!(app.sidebar_split_percent(), 25);
        assert_eq!(app.detail_split_percent(), 40);
    }

    #[test]
    fn test_theme_override_does_not_persist() {
        let mut app = create_test_app();
        assert_eq!(app.theme_name(), ThemeName::Dark);
        app.set_theme_name(ThemeName::Light);
        assert_eq!(app.theme_name(), ThemeName::Light);
    }
}
