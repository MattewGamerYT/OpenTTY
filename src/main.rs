mod config;
mod font;
mod pty;
mod terminal;
mod vulkan;
mod wayland;

use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::config::Config;
use crate::font::FontEngine;
use crate::pty::Pty;
use crate::terminal::{MouseMode, TerminalState};
use crate::vulkan::VulkanRenderer;
use crate::wayland::clipboard;
use crate::wayland::input::KeyAction;
use crate::wayland::{DropPayload, PointerAction, WaylandApp};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

fn compute_grid_dims(
    physical_w: u32,
    physical_h: u32,
    cell_w: f32,
    cell_h: f32,
    padding_px: f32,
    scale_factor: f32,
) -> (u16, u16) {
    let pad = (padding_px * scale_factor).round() * 2.0;
    let avail_w = (physical_w as f32 - pad).max(cell_w);
    let avail_h = (physical_h as f32 - pad).max(cell_h);
    let cols = (avail_w / cell_w).floor().clamp(2.0, 1024.0) as u16;
    let rows = (avail_h / cell_h).floor().clamp(1.0, 512.0) as u16;
    (cols, rows)
}

fn logical_to_cell(
    logical_x: f64,
    logical_y: f64,
    scale_factor: f32,
    padding_px: f32,
    cell_w: f32,
    cell_h: f32,
    cols: usize,
    rows: usize,
) -> (usize, usize) {
    let pad = (padding_px * scale_factor).round();
    let px = (logical_x as f32) * scale_factor;
    let py = (logical_y as f32) * scale_factor;
    let col = ((px - pad) / cell_w)
        .floor()
        .clamp(0.0, cols.saturating_sub(1) as f32) as usize;
    let row = ((py - pad) / cell_h)
        .floor()
        .clamp(0.0, rows.saturating_sub(1) as f32) as usize;
    (col, row)
}

fn encode_mouse_event(button_code: u8, col: usize, row: usize, pressed: bool, sgr: bool) -> Vec<u8> {
    if sgr {
        let suffix = if pressed { 'M' } else { 'm' };
        format!("\x1b[<{};{};{}{suffix}", button_code, col + 1, row + 1).into_bytes()
    } else {
        let cb = if pressed { button_code } else { 3 };
        let x = (col.min(222) as u8).saturating_add(33);
        let y = (row.min(222) as u8).saturating_add(33);
        vec![0x1B, b'[', b'M', cb.saturating_add(32), x, y]
    }
}

fn compute_auto_font_size_pt(
    physical_w: u32,
    physical_h: u32,
    scale_factor: f32,
    base_width_per_pt: f32,
    base_height_per_pt: f32,
    config: &Config,
) -> f32 {
    let min_pt = config.auto_zoom_min_pt.min(config.auto_zoom_max_pt);
    let max_pt = config.font_size_pt.min(config.auto_zoom_max_pt).max(min_pt);
    if !config.auto_zoom {
        return config.font_size_pt;
    }

    let pad = (config.padding_px * scale_factor).round() * 2.0;
    let avail_w = (physical_w as f32 - pad).max(1.0);
    let avail_h = (physical_h as f32 - pad).max(1.0);

    let width_per_pt = (base_width_per_pt * scale_factor).max(0.1);
    let height_per_pt = (base_height_per_pt * scale_factor).max(0.1);

    let pt_for_cols = avail_w / (width_per_pt * config.auto_zoom_target_cols.max(1) as f32);
    let pt_for_rows = avail_h / (height_per_pt * config.auto_zoom_target_rows.max(1) as f32);

    let raw_pt = max_pt.min(pt_for_cols).min(pt_for_rows).clamp(min_pt, max_pt);
    ((raw_pt * 2.0).floor() / 2.0).clamp(min_pt, max_pt)
}

fn min_timeout(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let config = Config::load();
    let mut manual_zoom = false;

    // Load font at 1.0 scale first to determine clean initial logical window dimensions.
    let mut font = FontEngine::load(&config.font_family_preferences, config.font_size_pt, 1.0)?;
    let base_metrics = font.metrics();
    let base_width_per_pt = base_metrics.cell_width / config.font_size_pt.max(1.0);
    let base_height_per_pt = base_metrics.cell_height / config.font_size_pt.max(1.0);
    let initial_logical_w =
        (base_metrics.cell_width * config.initial_cols as f32 + config.padding_px * 2.0).ceil()
            as u32;
    let initial_logical_h =
        (base_metrics.cell_height * config.initial_rows as f32 + config.padding_px * 2.0).ceil()
            as u32;

    let mut wayland = WaylandApp::connect(
        &config.app_id,
        &config.title,
        initial_logical_w,
        initial_logical_h,
    )?;

    let scale_factor = wayland.state.scale_factor();
    let (mut phys_w, mut phys_h) = wayland.state.physical_size();
    let mut font_size_pt = compute_auto_font_size_pt(
        phys_w,
        phys_h,
        scale_factor,
        base_width_per_pt,
        base_height_per_pt,
        &config,
    );
    font.set_scale(font_size_pt, scale_factor);

    let metrics = font.metrics();
    let (mut cols, mut rows) = compute_grid_dims(
        phys_w,
        phys_h,
        metrics.cell_width,
        metrics.cell_height,
        config.padding_px,
        scale_factor,
    );

    let (wl_display, wl_surface) = wayland.raw_handles();
    let mut renderer = VulkanRenderer::new(wl_display, wl_surface, phys_w, phys_h)?;

    let mut pty = Pty::spawn(
        &config.shell,
        cols,
        rows,
        phys_w.min(u16::MAX as u32) as u16,
        phys_h.min(u16::MAX as u32) as u16,
    )?;

    let mut terminal = TerminalState::new(
        cols as usize,
        rows as usize,
        config.scrollback_lines,
        config.cursor_style,
    );
    terminal.term.set_pixel_size(
        phys_w.min(u16::MAX as u32) as u16,
        phys_h.min(u16::MAX as u32) as u16,
    );
    let mut pty_buf = vec![0u8; 64 * 1024];

    let mut left_mouse_down = false;
    let mut pressed_mouse_btn: Option<u8> = None;
    let mut last_reported_cell: Option<(usize, usize)> = None;
    let mut focused = true;
    let mut cursor_blink_on = true;
    let mut last_blink_toggle = Instant::now();
    let mut pending_font_resize = false;

    // Clear initial flags so first frame renders immediately.
    wayland.state.size_changed = false;
    wayland.state.scale_changed = false;
    wayland.state.needs_redraw = true;

    loop {
        if wayland.state.should_close {
            break;
        }
        if let Ok(Some(_status)) = pty.try_wait() {
            break;
        }

        // Flush any outbound Wayland requests (e.g. frame callbacks, clipboard, title updates).
        let _ = wayland.connection.flush();

        // Drain any already-queued Wayland events, or prepare to poll both FDs with zero busy-wait.
        if let Some(read_guard) = wayland.event_queue.prepare_read() {
            let mut next_timeout_ms = wayland.state.xkb.next_repeat_timeout_ms();

            if config.cursor_blink
                && focused
                && terminal.term.active_grid().cursor.visible
                && terminal.term.active_grid().scroll_offset == 0
            {
                let blink_interval = Duration::from_millis(config.cursor_blink_ms);
                let remaining = blink_interval
                    .saturating_sub(last_blink_toggle.elapsed())
                    .as_millis() as u64;
                next_timeout_ms = min_timeout(next_timeout_ms, Some(remaining.max(1)));
            }

            if terminal.term.sync_update
                && let Some(started) = terminal.term.sync_started_at
            {
                let remaining = Duration::from_millis(50)
                    .saturating_sub(started.elapsed())
                    .as_millis() as u64;
                next_timeout_ms = min_timeout(next_timeout_ms, Some(remaining.max(1)));
            }

            let timeout = match next_timeout_ms {
                Some(ms) => PollTimeout::from(ms.min(u16::MAX as u64) as u16),
                None => PollTimeout::NONE,
            };

            let mut pty_watch = PollFlags::POLLIN;
            if pty.has_pending_writes() {
                pty_watch |= PollFlags::POLLOUT;
            }

            let wl_ready;
            let pty_readable;
            let pty_writable;
            {
                let wl_fd = read_guard.connection_fd();
                let pty_fd = pty.as_fd();
                let mut fds = [
                    PollFd::new(wl_fd, PollFlags::POLLIN),
                    PollFd::new(pty_fd, pty_watch),
                ];
                match poll(&mut fds, timeout) {
                    Ok(_) => {
                        wl_ready = fds[0]
                            .revents()
                            .is_some_and(|r| r.contains(PollFlags::POLLIN));
                        pty_readable = fds[1]
                            .revents()
                            .is_some_and(|r| r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP));
                        pty_writable = fds[1]
                            .revents()
                            .is_some_and(|r| r.contains(PollFlags::POLLOUT));
                    }
                    Err(nix::errno::Errno::EINTR) => {
                        wl_ready = false;
                        pty_readable = false;
                        pty_writable = false;
                    }
                    Err(e) => return Err(Box::new(std::io::Error::other(e))),
                }
            }

            if wl_ready {
                let _ = read_guard.read();
            } else {
                drop(read_guard);
            }

            wayland.event_queue.dispatch_pending(&mut wayland.state)?;

            if pty_writable {
                pty.flush_writes()?;
            }
            if pty_readable {
                drain_pty(&mut pty, &mut terminal, &mut pty_buf)?;
            }
        } else {
            wayland.event_queue.dispatch_pending(&mut wayland.state)?;
            if pty.has_pending_writes() {
                pty.flush_writes()?;
            }
            drain_pty(&mut pty, &mut terminal, &mut pty_buf)?;
        }

        // Forward window title updates from OSC 0 / OSC 2 to KWin titlebar.
        if let Some(title) = terminal.term.pending_title.take() {
            wayland.set_title(&title);
        }
        if let Some(clip_text) = terminal.term.pending_clipboard.take() {
            wayland.set_clipboard(&clip_text);
        }
        wayland.state.app_cursor_keys = terminal.term.application_cursor_keys;

        // Focus reporting (?1004).
        if let Some(is_focused) = wayland.state.pending_focus_event.take() {
            focused = is_focused;
            if !focused {
                left_mouse_down = false;
                pressed_mouse_btn = None;
                cursor_blink_on = true;
            } else {
                last_blink_toggle = Instant::now();
            }
            if terminal.term.focus_reporting {
                let seq: &[u8] = if is_focused { b"\x1b[I" } else { b"\x1b[O" };
                let _ = pty.queue_write(seq);
            }
            wayland.state.needs_redraw = true;
        }

        // Handle dropped files, folders, or text from Wayland drag-and-drop.
        for drop in wayland.take_pending_drops(config.shell_quote_dropped_paths) {
            match drop {
                DropPayload::Paths(paths) => {
                    terminal.term.clear_selection();
                    let _ = pty.queue_write(paths.as_bytes());
                }
                DropPayload::Text(bytes) => {
                    terminal.term.clear_selection();
                    let use_bracketed = config.bracketed_paste && terminal.term.bracketed_paste;
                    let payload = clipboard::normalize_pasted_text(&bytes, use_bracketed);
                    let _ = pty.queue_write(&payload);
                }
            }
            cursor_blink_on = true;
            last_blink_toggle = Instant::now();
        }

        // Handle pointer selection, mouse reporting, and scroll wheel events.
        let pointer_actions: Vec<_> = wayland.state.pending_pointer_actions.drain(..).collect();
        if !pointer_actions.is_empty() {
            let current_scale = wayland.state.scale_factor();
            let m = font.metrics();
            let term_cols = terminal.term.cols();
            let term_rows = terminal.term.rows();
            let shift_held = wayland.state.xkb.is_shift_active();
            let forward_mouse =
                terminal.term.mouse_reporting_active() && config.mouse_reporting && !shift_held;

            for ev in pointer_actions {
                match ev {
                    PointerAction::Button {
                        button,
                        pressed,
                        x,
                        y,
                    } => {
                        let (col, row) = logical_to_cell(
                            x,
                            y,
                            current_scale,
                            config.padding_px,
                            m.cell_width,
                            m.cell_height,
                            term_cols,
                            term_rows,
                        );
                        if forward_mouse {
                            let mouse_btn = match button {
                                BTN_LEFT => Some(0u8),
                                BTN_MIDDLE => Some(1u8),
                                BTN_RIGHT => Some(2u8),
                                _ => None,
                            };
                            if let Some(code) = mouse_btn {
                                pressed_mouse_btn = if pressed { Some(code) } else { None };
                                last_reported_cell = Some((col, row));
                                let seq = encode_mouse_event(
                                    code,
                                    col,
                                    row,
                                    pressed,
                                    terminal.term.sgr_mouse,
                                );
                                let _ = pty.queue_write(&seq);
                            }
                        } else if button == BTN_LEFT {
                            if pressed {
                                left_mouse_down = true;
                                terminal.term.start_selection(col, row);
                            } else if left_mouse_down {
                                left_mouse_down = false;
                                terminal.term.update_selection(col, row);
                                terminal.term.finish_selection();
                                if config.copy_on_select
                                    && let Some(text) = terminal
                                        .term
                                        .selected_text(config.strip_trailing_spaces_on_copy)
                                {
                                    wayland.set_clipboard(&text);
                                }
                            }
                        }
                    }
                    PointerAction::Motion { x, y } => {
                        let (col, row) = logical_to_cell(
                            x,
                            y,
                            current_scale,
                            config.padding_px,
                            m.cell_width,
                            m.cell_height,
                            term_cols,
                            term_rows,
                        );
                        if forward_mouse {
                            let should_report = match terminal.term.mouse_mode {
                                MouseMode::AnyEvent => true,
                                MouseMode::ButtonEvent => pressed_mouse_btn.is_some(),
                                _ => false,
                            };
                            if should_report && last_reported_cell != Some((col, row)) {
                                last_reported_cell = Some((col, row));
                                let code = pressed_mouse_btn.unwrap_or(3) + 32;
                                let seq = encode_mouse_event(
                                    code,
                                    col,
                                    row,
                                    true,
                                    terminal.term.sgr_mouse,
                                );
                                let _ = pty.queue_write(&seq);
                            }
                        } else if left_mouse_down {
                            terminal.term.update_selection(col, row);
                        }
                    }
                    PointerAction::Scroll { steps, x, y } => {
                        if forward_mouse {
                            let (col, row) = logical_to_cell(
                                x,
                                y,
                                current_scale,
                                config.padding_px,
                                m.cell_width,
                                m.cell_height,
                                term_cols,
                                term_rows,
                            );
                            let code = if steps > 0 { 64 } else { 65 };
                            for _ in 0..steps.abs() {
                                let seq = encode_mouse_event(
                                    code,
                                    col,
                                    row,
                                    true,
                                    terminal.term.sgr_mouse,
                                );
                                let _ = pty.queue_write(&seq);
                            }
                        } else if terminal.term.is_alt_screen() && config.alternate_scroll {
                            let seq: &[u8] = if steps > 0 {
                                if terminal.term.application_cursor_keys {
                                    b"\x1bOA"
                                } else {
                                    b"\x1b[A"
                                }
                            } else if terminal.term.application_cursor_keys {
                                b"\x1bOB"
                            } else {
                                b"\x1b[B"
                            };
                            let count = steps.unsigned_abs() * config.scroll_lines_per_step.max(1) as usize;
                            for _ in 0..count {
                                let _ = pty.queue_write(seq);
                            }
                        } else {
                            terminal
                                .term
                                .scroll_view(steps * config.scroll_lines_per_step);
                        }
                    }
                }
            }
        }

        // Handle keyboard & scroll actions + active key repeat.
        if let Some(repeat_action) = wayland
            .state
            .xkb
            .poll_repeat(terminal.term.application_cursor_keys)
        {
            wayland.state.pending_key_actions.push(repeat_action);
        }

        let key_actions: Vec<_> = wayland.state.pending_key_actions.drain(..).collect();
        for action in key_actions {
            match action {
                KeyAction::WritePty(bytes) => {
                    terminal.term.clear_selection();
                    let _ = pty.queue_write(&bytes);
                    cursor_blink_on = true;
                    last_blink_toggle = Instant::now();
                }
                KeyAction::ScrollLines(delta) => {
                    terminal.term.scroll_view(delta);
                }
                KeyAction::Copy => {
                    if let Some(text) = terminal
                        .term
                        .selected_text(config.strip_trailing_spaces_on_copy)
                    {
                        wayland.set_clipboard(&text);
                    }
                }
                KeyAction::Paste => {
                    if let Some(text) = wayland.read_clipboard_text() {
                        terminal.term.clear_selection();
                        let use_bracketed = config.bracketed_paste && terminal.term.bracketed_paste;
                        let payload =
                            clipboard::normalize_pasted_text(text.as_bytes(), use_bracketed);
                        let _ = pty.queue_write(&payload);
                        cursor_blink_on = true;
                        last_blink_toggle = Instant::now();
                    }
                }
                KeyAction::Zoom(delta) => {
                    manual_zoom = true;
                    let next_pt = (font_size_pt + delta * 0.5).clamp(6.0, 48.0);
                    if (next_pt - font_size_pt).abs() > 0.05 {
                        font_size_pt = next_pt;
                        pending_font_resize = true;
                    }
                }
                KeyAction::ResetZoom => {
                    manual_zoom = false;
                    let target_pt = compute_auto_font_size_pt(
                        phys_w,
                        phys_h,
                        wayland.state.scale_factor(),
                        base_width_per_pt,
                        base_height_per_pt,
                        &config,
                    );
                    if (target_pt - font_size_pt).abs() > 0.05 {
                        font_size_pt = target_pt;
                        pending_font_resize = true;
                    }
                }
            }
        }

        // Handle fractional scale, auto-zoom, font zoom, or window dimension changes atomically.
        if wayland.state.scale_changed || wayland.state.size_changed || pending_font_resize {
            let new_scale = wayland.state.scale_factor();
            let (new_phys_w, new_phys_h) = wayland.state.physical_size();

            if new_phys_w != phys_w || new_phys_h != phys_h || wayland.state.size_changed {
                phys_w = new_phys_w;
                phys_h = new_phys_h;
                renderer.recreate_swapchain(phys_w, phys_h)?;
                wayland.state.size_changed = false;
            }

            if !manual_zoom && config.auto_zoom {
                let auto_pt = compute_auto_font_size_pt(
                    phys_w,
                    phys_h,
                    new_scale,
                    base_width_per_pt,
                    base_height_per_pt,
                    &config,
                );
                if (auto_pt - font_size_pt).abs() > 0.05 {
                    font_size_pt = auto_pt;
                    pending_font_resize = true;
                }
            }

            if wayland.state.scale_changed || pending_font_resize {
                font.set_scale(font_size_pt, new_scale);
                renderer.clear_atlas();
                wayland.state.scale_changed = false;
                pending_font_resize = false;
            }

            let m = font.metrics();
            let (new_cols, new_rows) = compute_grid_dims(
                phys_w,
                phys_h,
                m.cell_width,
                m.cell_height,
                config.padding_px,
                new_scale,
            );
            if new_cols != cols || new_rows != rows {
                cols = new_cols;
                rows = new_rows;
                terminal.term.resize(cols as usize, rows as usize);
                terminal.term.set_pixel_size(
                    phys_w.min(u16::MAX as u32) as u16,
                    phys_h.min(u16::MAX as u32) as u16,
                );
                if let Err(err) = pty.resize(
                    cols,
                    rows,
                    phys_w.min(u16::MAX as u32) as u16,
                    phys_h.min(u16::MAX as u32) as u16,
                ) {
                    log::warn!("PTY resize failed: {err}");
                } else {
                    log::info!("PTY resized to {cols}x{rows} ({phys_w}x{phys_h} px)");
                }
            } else {
                terminal.term.set_pixel_size(
                    phys_w.min(u16::MAX as u32) as u16,
                    phys_h.min(u16::MAX as u32) as u16,
                );
            }
            wayland.state.needs_redraw = true;
        }

        if terminal.term.sync_timed_out() {
            wayland.state.needs_redraw = true;
        }

        if config.cursor_blink
            && focused
            && terminal.term.active_grid().cursor.visible
            && terminal.term.active_grid().scroll_offset == 0
            && last_blink_toggle.elapsed() >= Duration::from_millis(config.cursor_blink_ms)
        {
            cursor_blink_on = !cursor_blink_on;
            last_blink_toggle = Instant::now();
            wayland.state.needs_redraw = true;
        }

        let cursor_blink_visible = !config.cursor_blink || !focused || cursor_blink_on;

        // Frame-callback driven Vulkan rendering: only draw when dirty, compositor ready, and not inside ?2026 sync block.
        let dirty = wayland.state.needs_redraw || terminal.term.dirty;
        if dirty && !wayland.state.frame_pending && !terminal.term.sync_update {
            let current_scale = wayland.state.scale_factor();
            let suboptimal = renderer.render_and_present(
                &terminal.term,
                &mut font,
                &config,
                current_scale,
                cursor_blink_visible,
                || wayland.prepare_frame_commit(),
            )?;

            if suboptimal {
                let (pw, ph) = wayland.state.physical_size();
                renderer.recreate_swapchain(pw, ph)?;
                wayland.state.needs_redraw = true;
            } else {
                wayland.state.needs_redraw = false;
                terminal.term.dirty = false;
            }
        }
    }

    Ok(())
}

fn drain_pty(
    pty: &mut Pty,
    terminal: &mut TerminalState,
    buf: &mut [u8],
) -> Result<(), Box<dyn std::error::Error>> {
    // Coalesce burst PTY output up to 512 KiB per tick to keep frame latency tight.
    let mut total = 0usize;
    while total < 512 * 1024 {
        let n = pty.read_nonblocking(buf)?;
        if n == 0 {
            break;
        }
        terminal.feed_bytes(&buf[..n]);
        total += n;
    }

    if !terminal.term.pty_replies.is_empty() {
        let replies: Vec<u8> = terminal.term.pty_replies.drain(..).collect();
        let _ = pty.queue_write(&replies);
    }

    Ok(())
}
