use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::libc;
use nix::pty::{Winsize, openpty};
use nix::sys::termios::{InputFlags, SetArg, tcgetattr, tcsetattr};

nix::ioctl_write_ptr_bad!(tiocswinsz, libc::TIOCSWINSZ, Winsize);

pub struct Pty {
    master: File,
    child: Child,
    write_buf: Vec<u8>,
    write_offset: usize,
}

impl Pty {
    pub fn spawn(shell: &str, cols: u16, rows: u16, pixel_w: u16, pixel_h: u16) -> io::Result<Self> {
        let winsize = Winsize {
            ws_row: rows.max(1),
            ws_col: cols.max(1),
            ws_xpixel: pixel_w,
            ws_ypixel: pixel_h,
        };

        let pty = openpty(Some(&winsize), None).map_err(io::Error::other)?;
        let master_fd: OwnedFd = pty.master;
        let slave_fd: OwnedFd = pty.slave;

        if let Ok(mut tio) = tcgetattr(&slave_fd) {
            tio.input_flags |= InputFlags::IUTF8;
            let _ = tcsetattr(&slave_fd, SetArg::TCSANOW, &tio);
        }

        // Set FD_CLOEXEC + O_NONBLOCK on master PTY so child processes never inherit master_fd
        // and the event loop never stalls.
        let master_flags = fcntl(master_fd.as_raw_fd(), FcntlArg::F_GETFL).map_err(io::Error::other)?;
        let nonblock = OFlag::from_bits_truncate(master_flags) | OFlag::O_NONBLOCK;
        fcntl(master_fd.as_raw_fd(), FcntlArg::F_SETFL(nonblock)).map_err(io::Error::other)?;

        let master_fd_flags =
            fcntl(master_fd.as_raw_fd(), FcntlArg::F_GETFD).map_err(io::Error::other)?;
        fcntl(
            master_fd.as_raw_fd(),
            FcntlArg::F_SETFD(
                nix::fcntl::FdFlag::from_bits_truncate(master_fd_flags)
                    | nix::fcntl::FdFlag::FD_CLOEXEC,
            ),
        )
        .map_err(io::Error::other)?;

        let slave_fd_flags =
            fcntl(slave_fd.as_raw_fd(), FcntlArg::F_GETFD).map_err(io::Error::other)?;
        fcntl(
            slave_fd.as_raw_fd(),
            FcntlArg::F_SETFD(
                nix::fcntl::FdFlag::from_bits_truncate(slave_fd_flags)
                    | nix::fcntl::FdFlag::FD_CLOEXEC,
            ),
        )
        .map_err(io::Error::other)?;

        let slave_stdin = slave_fd.try_clone()?;
        let slave_stdout = slave_fd.try_clone()?;
        let slave_stderr = slave_fd;

        let mut cmd = Command::new(shell);
        cmd.stdin(Stdio::from(slave_stdin))
            .stdout(Stdio::from(slave_stdout))
            .stderr(Stdio::from(slave_stderr))
            .env_remove("LINES")
            .env_remove("COLUMNS")
            .env_remove("TERMCAP")
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "OpenTTY")
            .env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));

        // Safety: runs in the forked child immediately prior to execvp.
        unsafe {
            cmd.pre_exec(|| {
                // Unblock all signals (especially SIGWINCH) so child shells and TUI apps
                // never inherit a masked signal set from the GUI/Wayland process.
                let mut empty_set: libc::sigset_t = std::mem::zeroed();
                if libc::sigemptyset(&mut empty_set) == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::sigprocmask(libc::SIG_SETMASK, &empty_set, std::ptr::null_mut()) == -1 {
                    return Err(io::Error::last_os_error());
                }

                // Restore default signal dispositions via sigaction.
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = libc::SIG_DFL;
                sa.sa_mask = empty_set;
                for sig in [
                    libc::SIGHUP,
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGPIPE,
                    libc::SIGALRM,
                    libc::SIGTERM,
                    libc::SIGCHLD,
                    libc::SIGCONT,
                    libc::SIGTSTP,
                    libc::SIGTTIN,
                    libc::SIGTTOU,
                    libc::SIGWINCH,
                ] {
                    libc::sigaction(sig, &sa, std::ptr::null_mut());
                }

                let pid = libc::setsid();
                if pid == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as libc::Ioctl, 0 as libc::c_int)
                    == -1
                {
                    return Err(io::Error::last_os_error());
                }
                if libc::tcsetpgrp(libc::STDIN_FILENO, pid) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = cmd.spawn()?;
        let master = File::from(master_fd);

        Ok(Self {
            master,
            child,
            write_buf: Vec::with_capacity(4096),
            write_offset: 0,
        })
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.master.as_fd()
    }

    pub fn resize(&self, cols: u16, rows: u16, pixel_w: u16, pixel_h: u16) -> io::Result<()> {
        let ws = Winsize {
            ws_row: rows.max(1),
            ws_col: cols.max(1),
            ws_xpixel: pixel_w,
            ws_ypixel: pixel_h,
        };

        // Kernel updates tty->winsize and delivers SIGWINCH to the foreground process group.
        unsafe { tiocswinsz(self.master.as_raw_fd(), &ws) }
            .map(|_| ())
            .map_err(io::Error::other)
    }

    pub fn read_nonblocking(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.master.read(buf) {
            Ok(n) => Ok(n),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => Ok(0),
            Err(err) => Err(err),
        }
    }

    pub fn has_pending_writes(&self) -> bool {
        self.write_offset < self.write_buf.len()
    }

    /// Enqueues bytes for the PTY and flushes as much as possible without blocking.
    /// Remaining bytes are drained via POLLOUT in the main loop so large pastes never deadlock.
    pub fn queue_write(&mut self, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.write_buf.extend_from_slice(data);
        self.flush_writes()
    }

    pub fn flush_writes(&mut self) -> io::Result<()> {
        while self.write_offset < self.write_buf.len() {
            match self.master.write(&self.write_buf[self.write_offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write to PTY master",
                    ));
                }
                Ok(n) => self.write_offset += n,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        if self.write_offset >= self.write_buf.len() {
            self.write_buf.clear();
            self.write_offset = 0;
            if self.write_buf.capacity() > 64 * 1024 {
                self.write_buf.shrink_to(4096);
            }
        } else if self.write_offset > 32 * 1024 {
            self.write_buf.drain(..self.write_offset);
            self.write_offset = 0;
        }
        Ok(())
    }

    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
}
