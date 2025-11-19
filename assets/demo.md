# mq-tui Demo Document

Welcome to the mq-tui demonstration document. This file contains various Markdown elements to showcase mq-tui's querying capabilities.

## Getting Started

mq-tui is a powerful TUI for querying Markdown documents. Use the `:` key to enter query mode and start exploring.

### Basic Queries

Here are some queries you can try:

- `.h` - Select all headings
- `.link` - Select all links
- `.code` - Select all code blocks
- `.list` - Select all lists

## Code Examples

### JavaScript Example

```javascript
function greet(name) {
  console.log(`Hello, ${name}!`);
  return true;
}

greet("World");
```

### Python Example

```python
def calculate_sum(numbers):
    """Calculate the sum of a list of numbers."""
    return sum(numbers)

result = calculate_sum([1, 2, 3, 4, 5])
print(f"Sum: {result}")
```

### Rust Example

```rust
fn fibonacci(n: u32) -> u32 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn main() {
    println!("Fibonacci(10) = {}", fibonacci(10));
}
```

## Lists and Tasks

### Shopping List

- Apples
- Bananas
- Oranges
- Milk
- Bread

### Task List

- [x] Install mq-tui
- [x] Open demo document
- [ ] Try query mode
- [ ] Explore tree view
- [ ] Copy results to clipboard

### Nested List

1. First level
   - Second level item 1
   - Second level item 2
     - Third level item
2. Another first level
   - More nested items
3. Final item

## Links and References

### External Links

- [mq-tui GitHub Repository](https://github.com/harehare/mq-tui)
- [mq Markdown Processor](https://github.com/harehare/mq)
- [mqlang.org Documentation](https://mqlang.org)
- [Rust Programming Language](https://www.rust-lang.org)

### Internal References

See [Getting Started](#getting-started) for basic information.

## Images

### Logo Examples

![mq-tui Logo](https://via.placeholder.com/150?text=mq-tui)
![Sample Image](https://via.placeholder.com/200x100?text=Sample)

## Tables

### Feature Comparison

| Feature           | mq-tui | Other Tools |
| ----------------- | ------ | ----------- |
| Interactive Query | ✅      | ❌           |
| Tree View         | ✅      | ⚠️           |
| Vim Navigation    | ✅      | ❌           |
| Clipboard Support | ✅      | ✅           |
| Syntax Highlight  | ✅      | ✅           |

### Keyboard Shortcuts

| Key   | Action             |
| ----- | ------------------ |
| `q`   | Quit application   |
| `:`   | Enter query mode   |
| `t`   | Toggle tree view   |
| `d`   | Toggle detail view |
| `y`   | Copy to clipboard  |
| `?`   | Show help          |
| `j/k` | Navigate up/down   |

## Blockquotes

> "The best way to predict the future is to invent it."
> - Alan Kay

> **Note:** mq-tui supports real-time query execution as you type.

> **Tip:** Use the tree view mode to visualize your document structure.

## Mathematical Expressions

Inline math: $E = mc^2$

Block math:

$$
\int_{-\infty}^{\infty} e^{-x^2} dx = \sqrt{\pi}
$$

$$
\sum_{i=1}^{n} i = \frac{n(n+1)}{2}
$$

## Horizontal Rules

Here are some sections separated by horizontal rules:

---

## Advanced Features

### Custom Filtering

You can use complex filters with mq-tui:

```mq
.h | select(.depth >= 2 and .depth <= 3)
.code | select(.lang == "rust")
.link | select(.url | contains("github"))
```

### Tree Visualization

Press `t` to see the document structure as a tree with color-coded elements:

- Headings (Blue)
- Lists (Green)
- Math (Red)
- Links (Magenta)
- Images (Yellow)
- Code blocks (Cyan)

---

## Conclusion

This demo document covers the main Markdown elements that mq-tui can query and manipulate. Try different queries to explore the power of mq-tui!

### Quick Start Commands

```bash
# Open this demo
mq-tui assets/demo.md

# Open any Markdown file
mq-tui README.md

# Installation
curl -fsSL https://raw.githubusercontent.com/harehare/mq-tui/main/bin/install.sh | bash
```

### Next Steps

1. Try query mode with `:` key
2. Explore tree view with `t` key
3. View details with `d` key
4. Copy results with `y` key
5. Check help with `?` key

---

*Happy querying with mq-tui!*
