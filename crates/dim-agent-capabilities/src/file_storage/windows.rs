//! Handle-relative Windows filesystem operations. No pathname fallback.
use super::*;
use std::{
    fs::TryLockError,
    mem::{offset_of, size_of},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Prefix,
    ptr::{null, null_mut},
};
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
            FILE_OPEN_REPARSE_POINT, FILE_RENAME_IGNORE_READONLY_ATTRIBUTE,
            FILE_RENAME_INFORMATION, FILE_RENAME_POSIX_SEMANTICS, FILE_RENAME_REPLACE_IF_EXISTS,
            FILE_SYNCHRONOUS_IO_NONALERT, FILE_WRITE_THROUGH, FileRenameInformationEx,
            NtCreateFile, NtSetInformationFile, NtSetSecurityObject,
        },
    },
    Win32::{
        Foundation::{
            ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, LocalFree, RtlNtStatusToDosError,
            UNICODE_STRING, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::{
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            CreatePrivateObjectSecurityEx, DACL_SECURITY_INFORMATION, DestroyPrivateObjectSecurity,
            GENERIC_MAPPING, GetSecurityDescriptorControl, PROTECTED_DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED, SEF_DACL_AUTO_INHERIT,
            SetSecurityDescriptorControl, TOKEN_QUERY, UNPROTECTED_DACL_SECURITY_INFORMATION,
        },
        Storage::FileSystem::*,
        System::{
            IO::IO_STATUS_BLOCK,
            Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, WaitForSingleObject},
        },
    },
};

const LOCK_DIRECTORY: &str = ".dim-file-storage-locks";
const TEMPORARY_DIRECTORY: &str = ".dim-file-storage-temps";
const DIRECTORY_ACCESS: u32 =
    FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | READ_CONTROL;

pub(super) fn validate_component(name: &OsStr) -> Result<()> {
    let wide: Vec<u16> = name.encode_wide().collect();
    if wide.is_empty()
        || wide
            .iter()
            .any(|c| *c < 32 || b"\":*?<>|/\\".iter().any(|b| *c == u16::from(*b)))
        || matches!(wide.last(), Some(32 | 46))
    {
        return Err(FileStorageError::InvalidPath(
            "invalid Windows file component",
        ));
    }
    let stem = wide.split(|c| *c == u16::from(b'.')).next().unwrap();
    let stem: Vec<u16> = stem
        .iter()
        .map(|c| match *c {
            97..=122 => *c - 32,
            _ => *c,
        })
        .collect();
    let ascii_device = ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"]
        .iter()
        .any(|device| stem.iter().copied().eq(device.encode_utf16()));
    let numbered_device = stem.len() == 4
        && (stem[..3] == [67, 79, 77] || stem[..3] == [76, 80, 84])
        && matches!(stem[3], 49..=57 | 0x00b9 | 0x00b2 | 0x00b3);
    if ascii_device || numbered_device {
        return Err(FileStorageError::InvalidPath(
            "reserved Windows device name",
        ));
    }
    Ok(())
}
fn nt_result(status: i32) -> io::Result<()> {
    if status >= 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ))
    }
}
fn open(
    parent: Option<&File>,
    name: &OsStr,
    access: u32,
    disposition: u32,
    options: u32,
    security: PSECURITY_DESCRIPTOR,
) -> io::Result<File> {
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let length = u16::try_from(wide.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file name is too long"))?;
    let mut object_name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.map_or(null_mut(), |p| p.as_raw_handle()),
        ObjectName: &mut object_name,
        // Respect the actual directory's case-sensitivity setting.
        Attributes: 0,
        SecurityDescriptor: security.cast(),
        SecurityQualityOfService: null(),
    };
    let mut handle = null_mut();
    let mut status = IO_STATUS_BLOCK::default();
    nt_result(unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            disposition,
            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            null(),
            0,
        )
    })?;
    let file = unsafe { File::from_raw_handle(handle) };
    let validated = (|| {
        let metadata = file.metadata()?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (options & FILE_DIRECTORY_FILE != 0 && !metadata.is_dir())
            || (options & FILE_NON_DIRECTORY_FILE != 0 && !metadata.is_file())
            || unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reparse point or unexpected file type",
            ));
        }
        Ok(())
    })();
    if let Err(error) = validated {
        return Err(if disposition == FILE_CREATE {
            cleanup_created(&file, error)
        } else {
            error
        });
    }
    Ok(file)
}
fn directory(parent: &File, name: &OsStr) -> io::Result<File> {
    open(
        Some(parent),
        name,
        DIRECTORY_ACCESS,
        FILE_OPEN_IF,
        FILE_DIRECTORY_FILE,
        null_mut(),
    )
}
fn directories(parent: &File) -> io::Result<(File, File)> {
    let locks = directory(parent, OsStr::new(LOCK_DIRECTORY))?;
    let temporary = directory(&locks, OsStr::new(TEMPORARY_DIRECTORY))?;
    Ok((locks, temporary))
}
pub(super) fn temporary_directory(parent: &File) -> io::Result<File> {
    Ok(directories(parent)?.1)
}
pub(super) fn open_root(path: &Path) -> Result<File> {
    if !path.is_absolute() {
        return Err(FileStorageError::InvalidPath(
            "authorized roots must be absolute",
        ));
    }
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(FileStorageError::InvalidPath(
            "invalid Windows authorized root",
        ));
    };
    let mut nt_path = OsString::from("\\??\\");
    match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            nt_path.push(format!("{}:\\", char::from(drive)));
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            nt_path.push("UNC\\");
            nt_path.push(server);
            nt_path.push("\\");
            nt_path.push(share);
            nt_path.push("\\");
        }
        _ => {
            return Err(FileStorageError::InvalidPath(
                "unsupported Windows namespace",
            ));
        }
    }
    let mut root = open(
        None,
        &nt_path,
        DIRECTORY_ACCESS,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
        null_mut(),
    )?;
    for component in components {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                validate_component(name)?;
                root = open(
                    Some(&root),
                    name,
                    DIRECTORY_ACCESS,
                    FILE_OPEN,
                    FILE_DIRECTORY_FILE,
                    null_mut(),
                )?;
            }
            _ => return Err(FileStorageError::InvalidPath("invalid authorized root")),
        }
    }
    Ok(root)
}
pub(super) fn same_identity(a: &File, b: &File) -> io::Result<bool> {
    fn identity(file: &File) -> io::Result<FILE_ID_INFO> {
        let mut id = FILE_ID_INFO::default();
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut id as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(id)
    }
    let (a, b) = (identity(a)?, identity(b)?);
    Ok(a.VolumeSerialNumber == b.VolumeSerialNumber && a.FileId.Identifier == b.FileId.Identifier)
}
pub(super) fn parent(mut root: File, path: &Path) -> io::Result<(File, OsString)> {
    let mut components = path.iter().peekable();
    while let Some(name) = components.next() {
        if components.peek().is_none() {
            return Ok((root, name.to_os_string()));
        }
        let next = open(
            Some(&root),
            name,
            DIRECTORY_ACCESS,
            FILE_OPEN,
            FILE_DIRECTORY_FILE,
            null_mut(),
        )?;
        match open(
            Some(&root),
            OsStr::new(LOCK_DIRECTORY),
            DIRECTORY_ACCESS,
            FILE_OPEN,
            FILE_DIRECTORY_FILE,
            null_mut(),
        ) {
            Ok(reserved) if same_identity(&next, &reserved)? => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "reserved storage directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        root = next;
    }
    unreachable!("validated file path")
}
pub(super) fn target_name(parent: &File, name: &OsString) -> io::Result<OsString> {
    let file = match open(
        Some(parent),
        name,
        FILE_READ_ATTRIBUTES,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        null_mut(),
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(name.clone()),
        Err(error) => return Err(error),
    };
    let mut wide = vec![0; 256];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                wide.as_mut_ptr(),
                wide.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_NT,
            )
        };
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length as usize >= wide.len() {
            wide.resize(length as usize + 1, 0);
            continue;
        }
        wide.truncate(length as usize);
        let basename = wide.rsplit(|c| *c == u16::from(b'\\')).next().unwrap();
        return Ok(OsString::from_wide(basename));
    }
}
pub(super) fn lock(
    parent: &File,
    name: &OsStr,
    storage: &Arc<FileStorage>,
) -> Result<PhysicalLock> {
    let (directory, _temporary) = directories(parent)?;
    match open(
        Some(parent),
        name,
        FILE_READ_ATTRIBUTES,
        FILE_OPEN,
        0,
        null_mut(),
    ) {
        Ok(target) if same_identity(&target, &directory)? => {
            return Err(FileStorageError::InvalidPath("reserved storage directory"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let key = |name: &OsStr| {
        open(
            Some(&directory),
            name,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE,
            FILE_OPEN_IF,
            FILE_NON_DIRECTORY_FILE,
            null_mut(),
        )
    };
    let alias_lock = key(name)?;
    alias_lock.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => FileStorageError::Locked,
        TryLockError::Error(error) => FileStorageError::Io(error),
    })?;
    if same_identity(&alias_lock, &key(OsStr::new("auth.json.lock"))?)? {
        return Err(FileStorageError::InvalidPath("reserved storage lock"));
    }
    if same_identity(&alias_lock, &key(OsStr::new("auth.json"))?)? {
        return account_lock(parent, alias_lock, storage);
    }
    Ok(PhysicalLock {
        file: alias_lock,
        _alias_lock: None,
        account_parent: None,
        owner: None,
        storage: Arc::downgrade(storage),
    })
}
pub(super) fn create_account_file(parent: &File, name: &OsStr) -> io::Result<File> {
    open(
        Some(parent),
        name,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE | FILE_WRITE_THROUGH,
        null_mut(),
    )
}
pub(super) fn read_account_file(parent: &File, name: &OsString) -> io::Result<File> {
    open(
        Some(parent),
        name,
        FILE_GENERIC_READ | DELETE,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        null_mut(),
    )
}
pub(super) fn read_user_file(parent: &File, name: &OsString) -> io::Result<File> {
    let file = open(
        Some(parent),
        name,
        FILE_GENERIC_READ,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        null_mut(),
    )?;
    match open(
        Some(parent),
        OsStr::new("auth.json.lock"),
        FILE_READ_ATTRIBUTES,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        null_mut(),
    ) {
        Ok(marker) if same_identity(&file, &marker)? => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reserved storage lock",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(file)
}
pub(super) fn permissions(parent: &File, name: &OsString) -> io::Result<Option<Permissions>> {
    match read_user_file(parent, name) {
        Ok(file) => Ok(Some(file.metadata()?.permissions())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
// Descriptors have two native allocators; release each allocation exactly once.
struct SecurityDescriptor {
    pointer: PSECURITY_DESCRIPTOR,
    derived: bool,
}
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            if self.derived {
                DestroyPrivateObjectSecurity(&raw mut self.pointer);
            } else {
                LocalFree(self.pointer);
            }
        }
    }
}
impl SecurityDescriptor {
    fn read(file: &File) -> io::Result<Self> {
        let mut pointer = null_mut();
        let error = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut pointer,
            )
        };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        Ok(Self {
            pointer,
            derived: false,
        })
    }
    fn inherit(parent: &File) -> io::Result<Self> {
        let parent = Self::read(parent)?;
        let mut token = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ,
            GenericWrite: FILE_GENERIC_WRITE,
            GenericExecute: FILE_GENERIC_EXECUTE,
            GenericAll: FILE_ALL_ACCESS,
        };
        let mut pointer = null_mut();
        if unsafe {
            CreatePrivateObjectSecurityEx(
                parent.pointer,
                null_mut(),
                &mut pointer,
                null(),
                0,
                SEF_DACL_AUTO_INHERIT,
                token.as_raw_handle(),
                &mapping,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            pointer,
            derived: true,
        })
    }
    fn protected(&self) -> io::Result<bool> {
        let mut control = 0;
        let mut revision = 0;
        if unsafe { GetSecurityDescriptorControl(self.pointer, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(control & SE_DACL_PROTECTED != 0)
    }
    fn set_protected(&self, protected: bool) -> io::Result<()> {
        if unsafe {
            SetSecurityDescriptorControl(
                self.pointer,
                SE_DACL_PROTECTED,
                if protected { SE_DACL_PROTECTED } else { 0 },
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
pub(super) fn create_temp(
    parent: &File,
    name: &OsString,
    business_parent: &File,
    target: &OsString,
    _: Option<&Permissions>,
) -> io::Result<File> {
    let security = match open(
        Some(business_parent),
        target,
        READ_CONTROL | FILE_READ_ATTRIBUTES,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        null_mut(),
    ) {
        Ok(file) => SecurityDescriptor::read(&file)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            SecurityDescriptor::inherit(business_parent)?
        }
        Err(error) => return Err(error),
    };
    let protected = security.protected()?;
    // Apply the target ACL at creation, before any external reader can obtain a
    // handle. The maintenance directory must never broaden the effective ACL.
    security.set_protected(true)?;
    let file = open(
        Some(parent),
        name,
        // FILE_READ_ATTRIBUTES feeds the handle validation in `open`; without it
        // GetFileInformationByHandle fails with ACCESS_DENIED on the fresh handle.
        FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | DELETE,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE | FILE_WRITE_THROUGH,
        security.pointer,
    )?;
    let result = (|| {
        security.set_protected(protected)?;
        nt_result(unsafe {
            NtSetSecurityObject(
                file.as_raw_handle(),
                DACL_SECURITY_INFORMATION
                    | if protected {
                        PROTECTED_DACL_SECURITY_INFORMATION
                    } else {
                        UNPROTECTED_DACL_SECURITY_INFORMATION
                    },
                security.pointer,
            )
        })
    })();
    if let Err(error) = result {
        return Err(cleanup_created(&file, error));
    }
    Ok(file)
}
pub(super) fn sync_replacement(file: &File, _: &File, _: &File) -> io::Result<()> {
    // Windows cannot provide the Unix directory-fsync durability contract.
    file.sync_all()
}
pub(super) fn replace(
    file: &File,
    _: &File,
    _: &OsString,
    target_parent: &File,
    target: &OsString,
) -> io::Result<()> {
    // The Win32 wrapper rejects RootDirectory; the native API accepts the held
    // directory and a basename without resolving an absolute path again.
    let name: Vec<u16> = target.encode_wide().collect();
    let bytes = size_of::<FILE_RENAME_INFORMATION>() + name.len() * 2;
    let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
    let rename = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut status = IO_STATUS_BLOCK::default();
    unsafe {
        (*rename).Anonymous.Flags = FILE_RENAME_REPLACE_IF_EXISTS
            | FILE_RENAME_POSIX_SEMANTICS
            | FILE_RENAME_IGNORE_READONLY_ATTRIBUTE;
        (*rename).RootDirectory = target_parent.as_raw_handle();
        (*rename).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            buffer
                .as_mut_ptr()
                .cast::<u8>()
                .add(offset_of!(FILE_RENAME_INFORMATION, FileName))
                .cast(),
            name.len(),
        );
        nt_result(NtSetInformationFile(
            file.as_raw_handle(),
            &mut status,
            rename.cast(),
            bytes as u32,
            FileRenameInformationEx,
        ))
    }
}
fn delete(file: &File) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfoEx,
            (&disposition as *const FILE_DISPOSITION_INFO_EX).cast(),
            size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn cleanup_created(file: &File, error: io::Error) -> io::Error {
    match delete(file) {
        Ok(()) => error,
        Err(cleanup) => io::Error::new(
            error.kind(),
            FileStorageError::Cleanup {
                operation: Box::new(FileStorageError::Io(error)),
                cleanup,
            },
        ),
    }
}
pub(super) fn remove_account(_: &File, _: &OsString, file: &File) -> io::Result<()> {
    delete(file)
}
pub(super) fn remove(parent: &File, name: &OsString) -> io::Result<()> {
    delete(&open(
        Some(parent),
        name,
        DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
        null_mut(),
    )?)
}
pub(super) fn process_alive(pid: u32) -> io::Result<bool> {
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return match error.raw_os_error().map(|error| error as u32) {
            Some(ERROR_INVALID_PARAMETER) => Ok(false),
            Some(ERROR_ACCESS_DENIED) => Ok(true),
            _ => Err(error),
        };
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        WAIT_OBJECT_0 => Ok(false),
        WAIT_TIMEOUT => Ok(true),
        _ => Err(io::Error::last_os_error()),
    }
}
