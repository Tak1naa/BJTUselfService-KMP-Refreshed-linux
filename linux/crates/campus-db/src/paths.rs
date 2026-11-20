use crate::storage;
use campus_core::{Error, Result};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, GetFileInformationByHandle,
};

#[derive(Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub runtime: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        let dir = |env: &str, default: PathBuf| -> Result<PathBuf> {
            let p = std::env::var_os(env).map(PathBuf::from).unwrap_or(default);
            if !p.is_absolute() {
                return Err(Error::Usage(format!("{env} must be absolute")));
            }
            Ok(p.join("campus-portal"))
        };
        #[cfg(unix)]
        {
            let home = dirs::home_dir().ok_or_else(|| storage("HOME is not configured"))?;
            Ok(Self {
                data: dir("XDG_DATA_HOME", home.join(".local/share"))?,
                config: dir("XDG_CONFIG_HOME", home.join(".config"))?,
                cache: dir("XDG_CACHE_HOME", home.join(".cache"))?,
                state: dir("XDG_STATE_HOME", home.join(".local/state"))?,
                runtime: dir("XDG_RUNTIME_DIR", home.join(".cache"))?,
            })
        }
        #[cfg(windows)]
        {
            let local =
                dirs::data_local_dir().ok_or_else(|| storage("Local AppData is not configured"))?;
            Ok(Self {
                data: dir("XDG_DATA_HOME", local.clone())?,
                config: dir("XDG_CONFIG_HOME", local.clone())?,
                cache: dir("XDG_CACHE_HOME", local.clone())?,
                state: dir("XDG_STATE_HOME", local.clone())?,
                runtime: dir("XDG_RUNTIME_DIR", local)?,
            })
        }
    }
    pub fn database(&self) -> PathBuf {
        self.data.join("campus.db")
    }
}
#[cfg(unix)]
pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

pub fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(path).map_err(storage)?;
    }
    let m = fs::symlink_metadata(path).map_err(storage)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(storage("unsafe directory type"));
    }
    #[cfg(unix)]
    {
        if m.uid() != rustix::process::getuid().as_raw() {
            return Err(storage("unsafe directory owner"));
        }
        if m.permissions().mode() & 0o077 != 0 {
            return Err(storage(format!("{} must have mode 0700", path.display())));
        }
    }
    #[cfg(windows)]
    if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(storage("unsafe directory reparse point"));
    }
    Ok(())
}

pub(crate) fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    options
}

pub fn private_file(path: &Path) -> Result<File> {
    let file = private_options()
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(storage)?;
    let m = file.metadata().map_err(storage)?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(storage("unsafe file type"));
    }
    #[cfg(unix)]
    if m.uid() != rustix::process::getuid().as_raw()
        || m.permissions().mode() & 0o077 != 0
        || m.nlink() != 1
    {
        return Err(storage("unsafe file owner, permissions or link count"));
    }
    #[cfg(windows)]
    {
        use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
        let mut info = MaybeUninit::uninit();
        // The handle belongs to the live file and Windows initializes info on success.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(storage(std::io::Error::last_os_error()));
        }
        let info = unsafe { info.assume_init() };
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || info.nNumberOfLinks != 1 {
            return Err(storage("unsafe file reparse point or link count"));
        }
    }
    Ok(file)
}
pub struct SyncLock {
    _file: File,
}
impl SyncLock {
    pub fn acquire(paths: &Paths) -> Result<Self> {
        private_dir(&paths.runtime)?;
        let file = private_file(&paths.runtime.join("sync.lock"))?;
        #[cfg(unix)]
        rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Error::Busy)?;
        #[cfg(windows)]
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => Error::Busy,
            std::fs::TryLockError::Error(error) => storage(error),
        })?;
        Ok(Self { _file: file })
    }
}
