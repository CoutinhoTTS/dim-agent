//! Authenticated Windows pipe transport for clients without native handle access.
use std::io::{self, Write};

pub fn run(name: &str) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let pipe = match connect(name).await {
            Ok(pipe) => pipe,
            Err((code, error)) => {
                writeln!(
                    io::stderr(),
                    "{}",
                    serde_json::json!({"status":"error", "code":code, "message":error.to_string()})
                )?;
                return Err(error);
            }
        };
        writeln!(io::stderr(), "{}", serde_json::json!({"status":"ready"}))?;
        io::stderr().flush()?;
        let (mut reader, mut writer) = tokio::io::split(pipe);
        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        tokio::select! {
            result = tokio::io::copy(&mut stdin, &mut writer) => { result?; },
            result = tokio::io::copy(&mut reader, &mut stdout) => { result?; },
        }
        use tokio::io::AsyncWriteExt;
        stdout.flush().await
    });
    runtime.shutdown_background();
    result
}

async fn connect(
    name: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, (&'static str, io::Error)> {
    use tokio::net::windows::named_pipe::ClientOptions;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
    if !name.starts_with(r"\\.\pipe\") || name.len() == r"\\.\pipe\".len() || name.contains('\0') {
        return Err((
            "EINVAL",
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "a local named pipe is required",
            ),
        ));
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let pipe = loop {
        match ClientOptions::new().open(name) {
            Ok(pipe) => break pipe,
            Err(error) => {
                let code = match error.raw_os_error().map(|code| code as u32) {
                    Some(ERROR_PIPE_BUSY) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        continue;
                    }
                    Some(ERROR_FILE_NOT_FOUND) => "ENOENT",
                    Some(ERROR_PIPE_BUSY) => "EBUSY",
                    _ => "EACCES",
                };
                return Err((code, error));
            }
        }
    };
    authenticate(&pipe).map_err(|error| ("EACCES", error))?;
    Ok(pipe)
}

fn authenticate(pipe: &tokio::net::windows::named_pipe::NamedPipeClient) -> io::Result<()> {
    use std::{
        mem::size_of,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr::null_mut,
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, LocalFree},
        Security::{
            Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT},
            EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut length = 0;
    unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut length) };
    if io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return Err(io::Error::last_os_error());
    }
    let mut user = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            user.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut owner = null_mut();
    let mut descriptor = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let same_user = !owner.is_null()
        && unsafe { EqualSid(owner, (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid) != 0 };
    unsafe { LocalFree(descriptor) };
    if !same_user {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "named pipe is not owned by the current user",
        ));
    }
    Ok(())
}
