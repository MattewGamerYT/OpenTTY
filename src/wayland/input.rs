use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Instant;

use xkbcommon::xkb::{self, Keycode, Keysym, State};

#[derive(Debug, Clone)]
pub enum KeyAction {
    WritePty(Vec<u8>),
    ScrollLines(isize),
}

#[derive(Debug, Clone)]
pub struct RepeatState {
    pub rate_hz: i32,
    pub delay_ms: i32,
    pub active_key: Option<u32>,
    pub next_repeat_at: Option<Instant>,
}

impl Default for RepeatState {
    fn default() -> Self {
        Self {
            rate_hz: 25,
            delay_ms: 400,
            active_key: None,
            next_repeat_at: None,
        }
    }
}

pub struct XkbInput {
    context: xkb::Context,
    state: Option<State>,
    pub repeat: RepeatState,
}

impl XkbInput {
    pub fn new() -> Self {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let fallback_state = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            "",
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .map(|km| State::new(&km));

        Self {
            context,
            state: fallback_state,
            repeat: RepeatState::default(),
        }
    }

    pub fn load_keymap_from_fd(&mut self, fd: OwnedFd, size: u32) {
        if size == 0 {
            return;
        }
        let ptr = unsafe {
            nix::libc::mmap(
                std::ptr::null_mut(),
                size as usize,
                nix::libc::PROT_READ,
                nix::libc::MAP_PRIVATE,
                fd.as_raw_fd(),
                0,
            )
        };
        if ptr == nix::libc::MAP_FAILED {
            log::warn!("mmap failed for Wayland keymap fd");
            return;
        }

        let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, size as usize) };
        let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        let keymap_str = String::from_utf8_lossy(&slice[..end]).into_owned();
        unsafe {
            nix::libc::munmap(ptr, size as usize);
        }

        if let Some(keymap) = xkb::Keymap::new_from_string(
            &self.context,
            keymap_str,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) {
            self.state = Some(State::new(&keymap));
        } else {
            log::warn!("failed to compile XKB keymap from Wayland compositor");
        }
    }

    pub fn update_modifiers(
        &mut self,
        mods_depressed: u32,
        mods_latched: u32,
        mods_locked: u32,
        group: u32,
    ) {
        if let Some(state) = &mut self.state {
            state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
        }
    }

    pub fn set_repeat_info(&mut self, rate: i32, delay: i32) {
        self.repeat.rate_hz = rate;
        self.repeat.delay_ms = delay;
    }

    pub fn on_key_pressed(&mut self, evdev_key: u32, app_cursor: bool) -> Option<KeyAction> {
        let now = Instant::now();
        if self.repeat.rate_hz > 0 {
            self.repeat.active_key = Some(evdev_key);
            self.repeat.next_repeat_at =
                Some(now + std::time::Duration::from_millis(self.repeat.delay_ms.max(50) as u64));
        }
        self.translate_key(evdev_key, app_cursor)
    }

    pub fn on_key_released(&mut self, evdev_key: u32) {
        if self.repeat.active_key == Some(evdev_key) {
            self.repeat.active_key = None;
            self.repeat.next_repeat_at = None;
        }
    }

    pub fn clear_repeat(&mut self) {
        self.repeat.active_key = None;
        self.repeat.next_repeat_at = None;
    }

    pub fn poll_repeat(&mut self, app_cursor: bool) -> Option<KeyAction> {
        let key = self.repeat.active_key?;
        let next = self.repeat.next_repeat_at?;
        let now = Instant::now();
        if now >= next && self.repeat.rate_hz > 0 {
            let interval_us = (1_000_000 / self.repeat.rate_hz.max(1)) as u64;
            self.repeat.next_repeat_at = Some(now + std::time::Duration::from_micros(interval_us));
            return self.translate_key(key, app_cursor);
        }
        None
    }

    pub fn next_repeat_timeout_ms(&self) -> Option<u64> {
        let next = self.repeat.next_repeat_at?;
        let now = Instant::now();
        if next <= now {
            Some(0)
        } else {
            Some(next.duration_since(now).as_millis().max(1) as u64)
        }
    }

    fn translate_key(&self, evdev_key: u32, app_cursor: bool) -> Option<KeyAction> {
        let state = self.state.as_ref()?;
        let keycode = Keycode::from(evdev_key + 8);
        let keysym = state.key_get_one_sym(keycode);

        let shift = state.mod_name_is_active(xkb::MOD_NAME_SHIFT, xkb::STATE_MODS_EFFECTIVE);
        let ctrl = state.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE);
        let alt = state.mod_name_is_active(xkb::MOD_NAME_ALT, xkb::STATE_MODS_EFFECTIVE);

        if shift && keysym == Keysym::Page_Up {
            return Some(KeyAction::ScrollLines(8));
        }
        if shift && keysym == Keysym::Page_Down {
            return Some(KeyAction::ScrollLines(-8));
        }

        let mod_param = 1
            + if shift { 1 } else { 0 }
            + if alt { 2 } else { 0 }
            + if ctrl { 4 } else { 0 };

        let special: Option<Vec<u8>> = match keysym {
            Keysym::Return | Keysym::KP_Enter => {
                Some(if alt { b"\x1b\r".to_vec() } else { b"\r".to_vec() })
            }
            Keysym::BackSpace => Some(if alt {
                vec![0x1B, 0x7F]
            } else if ctrl {
                vec![0x08]
            } else {
                vec![0x7F]
            }),
            Keysym::Tab => Some(if alt { b"\x1b\t".to_vec() } else { b"\t".to_vec() }),
            Keysym::ISO_Left_Tab => Some(b"\x1b[Z".to_vec()),
            Keysym::Escape => Some(vec![0x1B]),
            Keysym::Up => Some(csi_or_ss3(b'A', mod_param, app_cursor)),
            Keysym::Down => Some(csi_or_ss3(b'B', mod_param, app_cursor)),
            Keysym::Right => Some(csi_or_ss3(b'C', mod_param, app_cursor)),
            Keysym::Left => Some(csi_or_ss3(b'D', mod_param, app_cursor)),
            Keysym::Home => Some(csi_or_ss3(b'H', mod_param, app_cursor)),
            Keysym::End => Some(csi_or_ss3(b'F', mod_param, app_cursor)),
            Keysym::Insert => Some(csi_tilde(2, mod_param)),
            Keysym::Delete => Some(csi_tilde(3, mod_param)),
            Keysym::Page_Up => Some(csi_tilde(5, mod_param)),
            Keysym::Page_Down => Some(csi_tilde(6, mod_param)),
            Keysym::F1 => Some(fn_key_ss3_or_csi(b'P', mod_param)),
            Keysym::F2 => Some(fn_key_ss3_or_csi(b'Q', mod_param)),
            Keysym::F3 => Some(fn_key_ss3_or_csi(b'R', mod_param)),
            Keysym::F4 => Some(fn_key_ss3_or_csi(b'S', mod_param)),
            Keysym::F5 => Some(csi_tilde(15, mod_param)),
            Keysym::F6 => Some(csi_tilde(17, mod_param)),
            Keysym::F7 => Some(csi_tilde(18, mod_param)),
            Keysym::F8 => Some(csi_tilde(19, mod_param)),
            Keysym::F9 => Some(csi_tilde(20, mod_param)),
            Keysym::F10 => Some(csi_tilde(21, mod_param)),
            Keysym::F11 => Some(csi_tilde(23, mod_param)),
            Keysym::F12 => Some(csi_tilde(24, mod_param)),
            _ => None,
        };

        if let Some(bytes) = special {
            return Some(KeyAction::WritePty(bytes));
        }

        // Ctrl + ASCII character handling
        if ctrl {
            let raw = keysym.raw();
            let ctrl_byte = match raw {
                0x61..=0x7A => Some((raw as u8) - b'a' + 1),
                0x41..=0x5A => Some((raw as u8) - b'A' + 1),
                0x20 | 0x40 => Some(0x00), // Ctrl+Space / Ctrl+@
                0x5B => Some(0x1B),        // Ctrl+[
                0x5C => Some(0x1C),        // Ctrl+\
                0x5D => Some(0x1D),        // Ctrl+]
                0x5E | 0x36 => Some(0x1E), // Ctrl+^ / Ctrl+6
                0x5F | 0x2F => Some(0x1F), // Ctrl+_ / Ctrl+/
                _ => None,
            };
            if let Some(b) = ctrl_byte {
                let bytes = if alt { vec![0x1B, b] } else { vec![b] };
                return Some(KeyAction::WritePty(bytes));
            }
        }

        let utf8 = state.key_get_utf8(keycode);
        if !utf8.is_empty() {
            let mut bytes = Vec::with_capacity(utf8.len() + 1);
            if alt {
                bytes.push(0x1B);
            }
            bytes.extend_from_slice(utf8.as_bytes());
            return Some(KeyAction::WritePty(bytes));
        }

        None
    }
}

fn csi_or_ss3(letter: u8, mod_param: u8, app_cursor: bool) -> Vec<u8> {
    if mod_param == 1 {
        let prefix = if app_cursor { b'O' } else { b'[' };
        vec![0x1B, prefix, letter]
    } else {
        format!("\x1b[1;{mod_param}{}", letter as char).into_bytes()
    }
}

fn fn_key_ss3_or_csi(letter: u8, mod_param: u8) -> Vec<u8> {
    if mod_param == 1 {
        vec![0x1B, b'O', letter]
    } else {
        format!("\x1b[1;{mod_param}{}", letter as char).into_bytes()
    }
}

fn csi_tilde(code: u8, mod_param: u8) -> Vec<u8> {
    if mod_param == 1 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{mod_param}~").into_bytes()
    }
}
