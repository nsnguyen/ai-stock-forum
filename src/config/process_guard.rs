//! Process-lifetime startup ownership guard anchored in the application state directory.

use std::{
    fmt,
    path::{Component, Path},
    sync::{Mutex, OnceLock},
};

use rustix::fs::{flock, fchmod, fstat, openat, CWD, FileType, FlockOperation, Mode, OFlags};

use super::StartupError;

const LOCK_FILENAME: &str = "phase0-bootstrap.lock";
static PROCESS_GUARD_ACQUISITION: OnceLock<Mutex<()>> = OnceLock::new();

pub struct ProcessGuard {
    #[cfg(unix)]
    _file: std::os::fd::OwnedFd,
}

impl fmt::Debug for ProcessGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessGuard(..)")
    }
}

impl ProcessGuard {
    pub const fn is_held(&self) -> bool {
        true
    }

    pub(crate) fn acquire(state_dir: &Path) -> Result<Self, StartupError> {
        #[cfg(unix)]
        {
            use std::os::fd::AsFd;

            let _acquisition = PROCESS_GUARD_ACQUISITION
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            let directory = open_state_directory(state_dir)?;
            let file = openat(
                &directory,
                LOCK_FILENAME,
                OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| StartupError::StatePermissions)?;
            let state = fstat(file.as_fd()).map_err(|_| StartupError::StatePermissions)?;
            if FileType::from_raw_mode(state.st_mode) != FileType::RegularFile {
                return Err(StartupError::StatePermissions);
            }
            fchmod(&file, Mode::from_raw_mode(0o600)).map_err(|_| StartupError::StatePermissions)?;
            let state = fstat(file.as_fd()).map_err(|_| StartupError::StatePermissions)?;
            if state.st_mode & 0o777 != 0o600 {
                return Err(StartupError::StatePermissions);
            }

            match flock(&file, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => Ok(Self { _file: file }),
                Err(error) if error == rustix::io::Errno::WOULDBLOCK => {
                    Err(StartupError::AlreadyRunning)
                }
                Err(_) => Err(StartupError::StatePermissions),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = state_dir;
            Err(StartupError::StatePermissions)
        }
    }
}

#[cfg(unix)]
fn open_state_directory(state_dir: &Path) -> Result<std::os::fd::OwnedFd, StartupError> {
    #[cfg(target_os = "macos")]
    let walk_path = if state_dir.starts_with(Path::new("/var")) {
        Path::new("/private").join(state_dir.strip_prefix("/").unwrap_or(state_dir))
    } else {
        state_dir.to_path_buf()
    };
    #[cfg(not(target_os = "macos"))]
    let walk_path = state_dir.to_path_buf();

    let mut current = openat(
        CWD,
        if walk_path.is_absolute() { Path::new("/") } else { Path::new(".") },
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| StartupError::StateDirectoryUnavailable)?;
    let mut saw_component = false;
    for component in walk_path.components() {
        let name = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(StartupError::StateDirectoryUnavailable);
            }
        };
        saw_component = true;
        current = openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| StartupError::StateDirectoryUnavailable)?;
    }
    if !saw_component {
        return Err(StartupError::StateDirectoryUnavailable);
    }
    Ok(current)
}
