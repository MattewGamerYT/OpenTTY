# OpenTTY

GPU-accelerated terminal emulator in Rust (edition 2024) targeting native Wayland (KDE Plasma / KWin) and Vulkan on CachyOS (`x86_64`).

## Architecture

- **Wayland (`src/wayland/`)**: Native `wayland-client` with `xdg_wm_base`, `zxdg_decoration_manager_v1` (KWin Breeze server-side decorations), `wp_fractional_scale_manager_v1`, `wp_viewporter`, `wl_surface::frame` callbacks, and `xkbcommon` keyboard handling. Zero X11, zero OpenGL.
- **Vulkan (`src/vulkan/`)**: Direct `ash` Vulkan renderer preferring `MAILBOX` present mode, single-draw-call instanced rendering (`vkCmdDraw(6, cell_count, 0, 0)`), and a dynamic `R8_UNORM` glyph atlas.
- **Terminal & PTY (`src/terminal/`, `src/pty.rs`)**: Non-blocking PTY launching `/usr/bin/fish` with `TERM=xterm-256color`, `COLORTERM=truecolor`, `SIGWINCH` resize via `TIOCSWINSZ`, and `vte` ANSI/CSI/OSC/truecolor parsing with scrollback and alternate screen support.

## Prerequisites (CachyOS / Arch Linux)

```bash
sudo pacman -S rustup vulkan-icd-loader wayland libxkbcommon
```

## Build & Run

```bash
cargo run --release
```
