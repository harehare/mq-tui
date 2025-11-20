<h1 align="center">mq-tui - TUI for mq Markdown Processor</h1>

[![ci](https://github.com/harehare/mq-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/harehare/mq-tui/actions/workflows/ci.yml)
[![mq language](https://img.shields.io/badge/mq-language-orange.svg)](https://github.com/harehare/mq)

<div align="center">

Interactive terminal interface for querying and manipulating Markdown content

![demo](./assets/demo.gif)

</div>

## Overview

`mq-tui` is a Text-based User Interface for the [mq](https://github.com/harehare/mq) Markdown processor. It provides an interactive terminal experience for querying, filtering, and exploring Markdown documents using the mq query language.

### Key Features

- 🔍 **Interactive Query Mode** - Real-time Markdown querying with instant results
- 🌳 **Tree View** - Visual exploration of Markdown document structure
- ⚡ **Vim-style Navigation** - Efficient keyboard shortcuts (j/k, hjkl)
- 📋 **Clipboard Integration** - Copy results directly to clipboard
- 🎨 **Syntax Highlighting** - Color-coded display of different Markdown elements
- 📖 **Detail View** - Inspect individual elements in depth
- 🔄 **Query History** - Navigate through previous queries
- 🎯 **fx-inspired UX** - Familiar interface for JSON query tool users

## Installation

### Using the Installation Script (Recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/harehare/mq-tui/main/bin/install.sh | bash
```

The installer will:
- Download the latest release for your platform
- Verify the binary with SHA256 checksum
- Install to `~/.mq-tui/bin/`
- Update your shell profile (bash, zsh, or fish)

After installation, restart your terminal or run:
```bash
source ~/.bashrc  # or ~/.zshrc, or ~/.config/fish/config.fish
```

### From Source

```bash
git clone https://github.com/harehare/mq-tui.git
cd mq-tui
cargo build --release
# Binary will be at target/release/mq-tui
```

### Supported Platforms

- **Linux**: x86_64, aarch64
- **macOS**: x86_64 (Intel), aarch64 (Apple Silicon)
- **Windows**: x86_64

## Usage

### Basic Usage

```bash
# Open a Markdown file
mq-tui README.md
```

### Query Examples

Once in the TUI, press `:` to enter query mode and try these queries:

```mq
# Select all headings
.h

# Select level 2 headings
.h | select(.depth == 2)

# Select all links
.link

# Select code blocks with specific language
.code | select(.lang == "rust")

# Select list items
.list

# Complex filtering
.h | select(.depth >= 2 and .depth <= 3)
```

## Key Bindings

### Normal Mode

| Key         | Action                               |
| ----------- | ------------------------------------ |
| `q` / `Esc` | Quit the application                 |
| `:`         | Enter query mode                     |
| `?` / `F1`  | Show help screen                     |
| `t`         | Toggle tree view mode                |
| `d`         | Toggle detail view for selected item |
| `y`         | Copy results to clipboard            |
| `Ctrl+L`    | Clear current query                  |

### Navigation

| Key        | Action               |
| ---------- | -------------------- |
| `↑` / `k`  | Move up              |
| `↓` / `j`  | Move down            |
| `PageUp`   | Page up (10 items)   |
| `PageDown` | Page down (10 items) |
| `Home`     | Jump to first item   |
| `End`      | Jump to last item    |

### Query Mode

| Key                    | Action                                  |
| ---------------------- | --------------------------------------- |
| `Enter`                | Execute query and return to normal mode |
| `Esc`                  | Exit query mode without executing       |
| `↑` / `↓`              | Navigate query history                  |
| `←` / `→`              | Move cursor in query string             |
| `Home` / `End`         | Jump to start/end of query              |
| `Backspace` / `Delete` | Edit query text                         |

### Tree View Mode

| Key               | Action               |
| ----------------- | -------------------- |
| `↑` / `k`         | Move up in tree      |
| `↓` / `j`         | Move down in tree    |
| `Enter` / `Space` | Expand/collapse node |
| `Esc` / `t`       | Exit tree view       |
| `?` / `F1`        | Show help            |

## Modes

### Normal Mode

Default mode for navigating and viewing query results. Use arrow keys or Vim-style navigation to browse through results.

### Query Mode

Activated by pressing `:`. Type your mq query and press Enter to execute. The query is evaluated in real-time as you type.

### Tree View Mode

Activated by pressing `t`. Displays the Markdown document structure as an expandable tree, showing the hierarchy of headings, lists, and other elements.

### Help Mode

Activated by pressing `?` or `F1`. Displays all available keyboard shortcuts and commands.

## Features in Detail

### Real-time Query Execution

Queries are executed as you type, providing immediate feedback and results.

### Detail View

Press `d` to toggle between list view and split view. In split view, the left pane shows the result list while the right pane displays detailed information about the selected item.

### Query History

All executed queries are saved in history. Use `↑` and `↓` in query mode to navigate through previous queries.

### Clipboard Support

Press `y` to copy the current query results to your system clipboard in Markdown format.

### Tree Visualization

The tree view mode provides a visual representation of your Markdown document's structure, with color-coded elements:

- 🔵 **Blue**: Headings
- 🟢 **Green**: Lists
- 🔴 **Red**: Math expressions
- 🟣 **Magenta**: Links
- 🟡 **Yellow**: Images
- 🔵 **Cyan**: Code blocks

## Configuration

`mq-tui` works out of the box with sensible defaults. The UI adapts to your terminal's color scheme and size.

## Related Projects

- [mq](https://github.com/harehare/mq) - The underlying Markdown query processor
- [mq-view](https://github.com/harehare/mq-view) - Markdown viewer with syntax highlighting
- [mqlang.org](https://mqlang.org) - Documentation and language reference

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This project is licensed under the MIT License - see the LICENSE file for details.
