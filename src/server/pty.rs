// A pane's process on a pseudo-terminal it owns as its controlling terminal (job control, ^C, and the foreground
// process group agent detection reads).
use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::Stdio;

use tokio::io::unix::AsyncFd;
use tokio::process::{Child, Command};

pub struct Pty {
    pub master: AsyncFd<OwnedFd>,
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 }
}

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

// Starts argv with the pty's other end as its stdio and controlling terminal, in a session of its own.
pub fn spawn<S: AsRef<OsStr>>(argv: &[S], cwd: &str, env: &[(String, String)], cols: u16, rows: u16) -> io::Result<(Pty, Child)> {
    let (mut master, mut slave) = (-1, -1);
    let ws = winsize(cols, rows);
    check(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &ws as *const _ as *mut _) })?;
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let fd = master.as_raw_fd();
    check(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) })?; // the child mustn't hold it open
    check(unsafe { libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK) })?;

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).current_dir(cwd).envs(env.iter().map(|(k, v)| (k, v)));
    cmd.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
    unsafe {
        cmd.pre_exec(|| {
            check(libc::setsid())?; // a session of its own, so the pty can become its controlling terminal
            check(libc::ioctl(0, libc::TIOCSCTTY as _, 0))?;
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok((Pty { master: AsyncFd::new(master)? }, child))
}

impl Pty {
    // What's there to read now; Ok(None) when it would block.
    pub fn try_read(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        let n = unsafe { libc::read(self.master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 {
            return Ok(Some(n as usize));
        }
        let e = io::Error::last_os_error();
        match e.kind() {
            io::ErrorKind::WouldBlock => Ok(None),
            io::ErrorKind::Interrupted => self.try_read(buf),
            _ => Err(e),
        }
    }

    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.master.readable().await?;
            if let Some(n) = self.try_read(buf)? {
                return Ok(n);
            }
            guard.clear_ready();
        }
    }

    pub async fn write_all(&self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let mut guard = self.master.writable().await?;
            let n = unsafe { libc::write(self.master.as_raw_fd(), data.as_ptr().cast(), data.len()) };
            if n >= 0 {
                data = &data[n as usize..];
                continue;
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => guard.clear_ready(),
                io::ErrorKind::Interrupted => {}
                _ => return Err(e),
            }
        }
        Ok(())
    }

    // The process group in the foreground of the pane's terminal: its shell, or the job the shell handed it to.
    pub fn foreground(&self) -> Option<i32> {
        let pgid = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        (pgid > 0).then_some(pgid)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let ws = winsize(cols, rows);
        check(unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) }).map(|_| ())
    }
}
