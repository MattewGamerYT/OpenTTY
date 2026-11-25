use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::libc;
use nix::pty::{Winsize, openpty};

pub struct Pty {
    master: File,
    child: Child,
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

        // Set non-blocking mode on master PTY so the event loop never stalls.
        let flags = fcntl(master_fd.as_raw_fd(), FcntlArg::F_GETFL).map_err(io::Error::other)?;
        let nonblock = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
        fcntl(master_fd.as_raw_fd(), FcntlArg::F_SETFL(nonblock)).map_err(io::Error::other)?;

        let slave_raw = slave_fd.as_raw_fd();
        let slave_stdin = slave_fd.try_clone()?;
        let slave_stdout = slave_fd.try_clone()?;
        let slave_stderr = slave_fd;

        let mut cmd = Command::new(shell);
        cmd.stdin(Stdio::from(slave_stdin))
            .stdout(Stdio::from(slave_stdout))
            .stderr(Stdio::from(slave_stderr))
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "OpenTTY")
            .env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));

        // Safety: runs in the forked child immediately prior to execvp.
        unsafe {
            cmd.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(slave_raw, libc::TIOCSCTTY as libc::c_ulong, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = cmd.spawn()?;
        let master = File::from(master_fd);

        Ok(Self { master, child })
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

        // Kernel delivers SIGWINCH to the foreground process group automatically.
        let res = unsafe {
            libc::ioctl(
                self.master.as_raw_fd(),
                libc::TIOCSWINSZ as libc::c_ulong,
                &ws,
            )
        };
        if res == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn read_nonblocking(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.master.read(buf) {
            Ok(n) => Ok(n),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(err) => Err(err),
        }
    }

    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        let mut offset = 0;
        while offset < data.len() {
            match self.master.write(&data[offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write to PTY master",
                    ));
                }
                Ok(n) => offset += n,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::yield_now();
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
}
