use crate::Result;
use std::io::{self, IsTerminal, Write};

#[cfg(unix)]
pub struct Input {
    fd: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
    buffer: Vec<u8>,
    flags: i32,
}

#[cfg(unix)]
impl Input {
    pub fn new() -> Result<Self> {
        use std::os::fd::FromRawFd;
        if !io::stdin().is_terminal() {
            return Err("This operation requires an interactive terminal".into());
        }
        let fd = unsafe { libc::dup(libc::STDIN_FILENO) };
        if fd == -1 {
            return Err(io::Error::last_os_error().into());
        }
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        use std::os::fd::AsRawFd;
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags == -1
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(io::Error::last_os_error().into());
        }
        let fd = match tokio::io::unix::AsyncFd::new(fd) {
            Ok(fd) => fd,
            Err(error) => {
                unsafe {
                    libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, flags);
                }
                return Err(error.into());
            }
        };
        Ok(Self {
            fd,
            buffer: Vec::new(),
            flags,
        })
    }

    pub fn prompt(&mut self, text: &str) -> Result<()> {
        use std::os::fd::AsRawFd;
        self.buffer.clear();
        if unsafe { libc::tcflush(self.fd.get_ref().as_raw_fd(), libc::TCIFLUSH) } == -1 {
            return Err(io::Error::last_os_error().into());
        }
        print!("{text}");
        io::stdout().flush()?;
        Ok(())
    }

    pub async fn line(&mut self) -> Result<Option<String>> {
        use std::os::fd::AsRawFd;
        loop {
            if let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line = String::from_utf8(self.buffer[..index].to_vec())
                    .map_err(|_| "Terminal input must be UTF-8")?;
                self.buffer.drain(..=index);
                return Ok(Some(line.trim_end_matches('\r').into()));
            }
            if self.buffer.len() > 1024 * 1024 {
                return Err("Terminal input exceeds 1 MiB".into());
            }
            let mut ready = self.fd.readable().await?;
            let read = ready.try_io(|fd| {
                let mut bytes = [0u8; 4096];
                let size = unsafe {
                    libc::read(
                        fd.get_ref().as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                    )
                };
                if size < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok((bytes, size as usize))
                }
            });
            match read {
                Ok(Ok((_, 0))) => {
                    return if self.buffer.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(
                            String::from_utf8(std::mem::take(&mut self.buffer))
                                .map_err(|_| "Terminal input must be UTF-8")?,
                        ))
                    };
                }
                Ok(Ok((bytes, size))) => self.buffer.extend_from_slice(&bytes[..size]),
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => {}
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => {}
            }
        }
    }

    pub fn hide_echo(&self) -> Result<EchoGuard> {
        use std::os::fd::AsRawFd;
        let fd = self.fd.get_ref().as_raw_fd();
        let mut previous = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut previous) } == -1 {
            return Err(io::Error::last_os_error().into());
        }
        let mut hidden = previous;
        hidden.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } == -1 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(EchoGuard { fd, previous })
    }
}

#[cfg(unix)]
impl Drop for Input {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::fcntl(self.fd.get_ref().as_raw_fd(), libc::F_SETFL, self.flags);
        }
    }
}

#[cfg(unix)]
pub struct EchoGuard {
    fd: std::os::fd::RawFd,
    previous: libc::termios,
}
#[cfg(unix)]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.previous);
        }
    }
}

#[cfg(windows)]
pub struct Input {
    handle: windows_sys::Win32::Foundation::HANDLE,
    mode: u32,
    buffer: Vec<u16>,
    echo: std::rc::Rc<std::cell::Cell<bool>>,
}
#[cfg(windows)]
impl Input {
    pub fn new() -> Result<Self> {
        use windows_sys::Win32::System::Console::{
            ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE,
            SetConsoleMode,
        };
        if !io::stdin().is_terminal() {
            return Err("This operation requires an interactive terminal".into());
        }
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0;
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0
            || unsafe { SetConsoleMode(handle, mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT)) }
                == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self {
            handle,
            mode,
            buffer: Vec::new(),
            echo: std::rc::Rc::new(std::cell::Cell::new(true)),
        })
    }
    pub fn prompt(&mut self, text: &str) -> Result<()> {
        self.buffer.clear();
        if unsafe { windows_sys::Win32::System::Console::FlushConsoleInputBuffer(self.handle) } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        print!("{text}");
        io::stdout().flush()?;
        Ok(())
    }
    pub async fn line(&mut self) -> Result<Option<String>> {
        use windows_sys::Win32::System::Console::{
            GetNumberOfConsoleInputEvents, INPUT_RECORD, KEY_EVENT, ReadConsoleInputW,
        };
        loop {
            let mut count = 0;
            if unsafe { GetNumberOfConsoleInputEvents(self.handle, &mut count) } == 0 {
                return Err(io::Error::last_os_error().into());
            }
            if count == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }
            let mut event = INPUT_RECORD::default();
            if unsafe { ReadConsoleInputW(self.handle, &mut event, 1, &mut count) } == 0 {
                return Err(io::Error::last_os_error().into());
            }
            if count == 0 || event.EventType != KEY_EVENT as u16 {
                continue;
            }
            let key = unsafe { event.Event.KeyEvent };
            if key.bKeyDown == 0 {
                continue;
            }
            let value = unsafe { key.uChar.UnicodeChar };
            for _ in 0..key.wRepeatCount {
                match value {
                    13 => {
                        if self.echo.get() {
                            println!();
                        }
                        return Ok(Some(
                            String::from_utf16(&std::mem::take(&mut self.buffer))
                                .map_err(|_| "Terminal input must be valid Unicode")?,
                        ));
                    }
                    4 | 26 if self.buffer.is_empty() => return Ok(None),
                    8 => {
                        if let Some(last) = self.buffer.pop() {
                            if (0xdc00..=0xdfff).contains(&last) {
                                self.buffer.pop();
                            }
                            if self.echo.get() {
                                print!("\u{8} \u{8}");
                                io::stdout().flush()?;
                            }
                        }
                    }
                    value if value >= 32 => {
                        if self.buffer.len() >= 1024 * 1024 {
                            return Err("Terminal input exceeds 1 MiB".into());
                        }
                        self.buffer.push(value);
                        if self.echo.get() && !(0xd800..=0xdbff).contains(&value) {
                            let start = if (0xdc00..=0xdfff).contains(&value) {
                                self.buffer.len().saturating_sub(2)
                            } else {
                                self.buffer.len() - 1
                            };
                            print!(
                                "{}",
                                crate::display::safe(&String::from_utf16_lossy(
                                    &self.buffer[start..]
                                ))
                            );
                            io::stdout().flush()?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    pub fn hide_echo(&self) -> Result<EchoGuard> {
        self.echo.set(false);
        Ok(EchoGuard(self.echo.clone()))
    }
}
#[cfg(windows)]
pub struct EchoGuard(std::rc::Rc<std::cell::Cell<bool>>);
#[cfg(windows)]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        self.0.set(true);
    }
}
#[cfg(windows)]
impl Drop for Input {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleMode(self.handle, self.mode);
        }
    }
}
