mod config;
mod font;
mod pty;
mod terminal;
mod vulkan;
mod wayland;

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::config::Config;
use crate::font::FontEngine;
use crate::pty::Pty;
use crate::terminal::TerminalState;
use crate::vulkan::VulkanRenderer;
use crate::wayland::WaylandApp;
use crate::wayland::input::KeyAction;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let config = Config::default();

    // Load font at 1.0 scale first to determine clean initial logical window dimensions.
    let mut font = FontEngine::load(&config.font_family_preferences, config.font_size_pt, 1.0)?;
    let base_metrics = font.metrics();
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
    font.set_scale(config.font_size_pt, scale_factor);

    let (mut phys_w, mut phys_h) = wayland.state.physical_size();
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

    let mut terminal = TerminalState::new(cols as usize, rows as usize, config.scrollback_lines);
    terminal.term.set_pixel_size(
        phys_w.min(u16::MAX as u32) as u16,
        phys_h.min(u16::MAX as u32) as u16,
    );
    let mut pty_buf = vec![0u8; 64 * 1024];

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

        // Flush any outbound Wayland requests (e.g. frame callbacks, title updates).
        let _ = wayland.connection.flush();

        // Drain any already-queued Wayland events, or prepare to poll both FDs with zero busy-wait.
        if let Some(read_guard) = wayland.event_queue.prepare_read() {
            let timeout = match wayland.state.xkb.next_repeat_timeout_ms() {
                Some(ms) => PollTimeout::from(ms.min(u16::MAX as u64) as u16),
                None => PollTimeout::NONE,
            };

            let wl_ready;
            let pty_ready;
            {
                let wl_fd = read_guard.connection_fd();
                let pty_fd = pty.as_fd();
                let mut fds = [
                    PollFd::new(wl_fd, PollFlags::POLLIN),
                    PollFd::new(pty_fd, PollFlags::POLLIN),
                ];
                match poll(&mut fds, timeout) {
                    Ok(_) => {
                        wl_ready = fds[0]
                            .revents()
                            .is_some_and(|r| r.contains(PollFlags::POLLIN));
                        pty_ready = fds[1]
                            .revents()
                            .is_some_and(|r| r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP));
                    }
                    Err(nix::errno::Errno::EINTR) => {
                        wl_ready = false;
                        pty_ready = false;
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

            if pty_ready {
                drain_pty(&mut pty, &mut terminal, &mut pty_buf)?;
            }
        } else {
            wayland.event_queue.dispatch_pending(&mut wayland.state)?;
            drain_pty(&mut pty, &mut terminal, &mut pty_buf)?;
        }

        // Forward window title updates from OSC 0 / OSC 2 to KWin titlebar.
        if let Some(title) = terminal.term.pending_title.take() {
            wayland.set_title(&title);
        }
        wayland.state.app_cursor_keys = terminal.term.application_cursor_keys;

        // Handle keyboard & scroll actions + active key repeat.
        if let Some(repeat_action) = wayland
            .state
            .xkb
            .poll_repeat(terminal.term.application_cursor_keys)
        {
            wayland.state.pending_key_actions.push(repeat_action);
        }

        for action in wayland.state.pending_key_actions.drain(..) {
            match action {
                KeyAction::WritePty(bytes) => {
                    let _ = pty.write_all(&bytes);
                }
                KeyAction::ScrollLines(delta) => {
                    terminal.term.scroll_view(delta);
                }
            }
        }

        // Handle fractional scale or window dimension changes atomically.
        if wayland.state.scale_changed || wayland.state.size_changed {
            let new_scale = wayland.state.scale_factor();
            if wayland.state.scale_changed {
                font.set_scale(config.font_size_pt, new_scale);
                renderer.clear_atlas();
                wayland.state.scale_changed = false;
            }

            let (new_phys_w, new_phys_h) = wayland.state.physical_size();
            if new_phys_w != phys_w || new_phys_h != phys_h || wayland.state.size_changed {
                phys_w = new_phys_w;
                phys_h = new_phys_h;
                renderer.recreate_swapchain(phys_w, phys_h)?;

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
                wayland.state.size_changed = false;
                wayland.state.needs_redraw = true;
            }
        }

        // Frame-callback driven Vulkan rendering: only draw when dirty and compositor is ready.
        let dirty = wayland.state.needs_redraw || terminal.term.dirty;
        if dirty && !wayland.state.frame_pending {
            let current_scale = wayland.state.scale_factor();
            let suboptimal = renderer.render_and_present(
                &terminal.term,
                &mut font,
                &config,
                current_scale,
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
        let _ = pty.write_all(&replies);
    }

    Ok(())
}
