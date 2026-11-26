# OpenTTY

GPU-accelerated terminal emulator in Rust (edition 2024) targeting native Wayland (KDE Plasma / KWin) and Vulkan on CachyOS (`x86_64`).

## Architecture

- **Wayland (`src/wayland/`)**: Native `wayland-client` with `xdg_wm_base`, `zxdg_decoration_manager_v1` (KWin Breeze server-side decorations), `wp_fractional_scale_manager_v1`, `wp_viewporter`, `wl_surface::frame` callbacks, `wl_data_device` clipboard & drag-and-drop, and `xkbcommon` keyboard handling. Zero X11, zero OpenGL.
- **Vulkan (`src/vulkan/`)**: Direct `ash` Vulkan renderer preferring `MAILBOX` present mode, single-draw-call instanced rendering (`vkCmdDraw(6, cell_count, 0, 0)`), and a dynamic `R8_UNORM` glyph atlas.
- **Terminal & PTY (`src/terminal/`, `src/pty.rs`)**: Non-blocking PTY launching `/usr/bin/fish` with `TERM=xterm-256color`, `COLORTERM=truecolor`, `SIGWINCH` resize via `TIOCSWINSZ`, reflowable scrollback, alternate screen, SGR/X10 mouse reporting, focus reporting (`?1004`), synchronized output (`?2026`), OSC 52 clipboard, and `vte` ANSI/CSI/OSC parsing.

## Shortcuts & Mouse

- **`Ctrl+Shift+C`**: Copy active mouse selection (drag, double-click word, triple-click logical line) to the Wayland clipboard.
- **`Ctrl+Shift+V`**: Paste clipboard text cleanly (honoring bracketed paste `\x1b[200~...\x1b[201~`).
- **Drag & Drop**: Drop files or folders into the terminal to insert shell-quoted filesystem paths, or drop selected text directly.
- **`Ctrl++` / `Ctrl+-` / `Ctrl+0`**: Zoom font size in, out, or reset at runtime.
- **`Shift+PageUp` / `Shift+PageDown`** or **Mouse Wheel**: Scroll scrollback (or hold `Shift` while clicking/selecting to bypass TUI mouse reporting).

## Configuration (`~/.config/opentty/opentty.conf`)

Optional `key = value` overrides (also respects `$XDG_CONFIG_HOME/opentty/opentty.conf`):

```ini
font_family = JetBrains Mono
font_size_pt = 10.0
auto_zoom = true
auto_zoom_min_pt = 7.5
auto_zoom_max_pt = 10.5
auto_zoom_target_cols = 110
auto_zoom_target_rows = 32
padding_px = 8
scrollback_lines = 10000
scroll_lines_per_step = 3
cursor_style = block        # block | beam | underline
cursor_blink = false
cursor_blink_ms = 530
copy_on_select = false
strip_trailing_spaces_on_copy = true
bracketed_paste = true
shell_quote_dropped_paths = true
mouse_reporting = true
alternate_scroll = true
default_fg = #FCFCFC
default_bg = #232629
cursor_color = #EFF0F1
selection_fg = #FCFCFC
selection_bg = #2D5F85
```

## Prerequisites (CachyOS / Arch Linux)

```bash
sudo pacman -S rustup vulkan-icd-loader wayland libxkbcommon
```

## Build & Run

```bash
cargo run --release
```
