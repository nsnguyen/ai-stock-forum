use std::path::{Path, PathBuf};

use directories::BaseDirs;

use super::StartupError;

const APPLICATION_DIRECTORY: &str = "ai-stock-forum";
const DATABASE_FILENAME: &str = "ai-stock-forum.sqlite3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    state_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self, StartupError> {
        let base_dirs = BaseDirs::new().ok_or(StartupError::StateDirectoryUnavailable)?;
        Ok(Self {
            state_dir: base_dirs.data_dir().join(APPLICATION_DIRECTORY),
        })
    }

    pub fn for_test(root: impl AsRef<Path>) -> Self {
        Self {
            state_dir: root.as_ref().to_path_buf(),
        }
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn database_path(&self) -> PathBuf {
        self.state_dir.join(DATABASE_FILENAME)
    }

    pub(crate) fn sqlite_open_path(&self) -> PathBuf {
        let path = self.database_path();
        #[cfg(target_os = "macos")]
        if path.starts_with(Path::new("/var")) {
            return Path::new("/private").join(path.strip_prefix("/").unwrap_or(&path));
        }
        path
    }

    pub fn ensure(&self) -> Result<(), StartupError> {
        #[cfg(unix)]
        {
            ensure_unix(&self.state_dir)
        }
        #[cfg(not(unix))]
        {
            ensure_portable(&self.state_dir)
        }
    }

}

#[cfg(unix)]
fn ensure_unix(path: &Path) -> Result<(), StartupError> {
    use std::os::fd::AsFd;
    use std::path::Component;

    use rustix::fs::{fchmod, fstat, mkdirat, openat, CWD, FileType, Mode, OFlags};

    #[cfg(target_os = "macos")]
    let walk_path = macos_walk_path(path);
    #[cfg(not(target_os = "macos"))]
    let walk_path = path.to_path_buf();

    let mut current = openat(
        CWD,
        if walk_path.is_absolute() {
            Path::new("/")
        } else {
            Path::new(".")
        },
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

        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        current = match openat(&current, name, flags, Mode::empty()) {
            Ok(directory) => directory,
            Err(error) if error == rustix::io::Errno::NOENT => {
                mkdirat(&current, name, Mode::from_raw_mode(0o700))
                    .map_err(|_| StartupError::StateDirectoryUnavailable)?;
                openat(&current, name, flags, Mode::empty())
                    .map_err(|_| StartupError::StateDirectoryUnavailable)?
            }
            Err(_) => return Err(StartupError::StateDirectoryUnavailable),
        };
    }

    if !saw_component {
        return Err(StartupError::StateDirectoryUnavailable);
    }

    fchmod(&current, Mode::from_raw_mode(0o700))
        .map_err(|_| StartupError::StatePermissions)?;
    let state = fstat(current.as_fd()).map_err(|_| StartupError::StatePermissions)?;
    if FileType::from_raw_mode(state.st_mode) != FileType::Directory
        || state.st_mode & 0o777 != 0o700
    {
        return Err(StartupError::StatePermissions);
    }

    let database_flags = OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let database = match openat(&current, DATABASE_FILENAME, database_flags, Mode::empty()) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => openat(
            &current,
            DATABASE_FILENAME,
            database_flags | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| StartupError::StatePermissions)?,
        Err(_) => return Err(StartupError::StateDirectoryUnavailable),
    };

    let database_stat = fstat(database.as_fd()).map_err(|_| StartupError::StatePermissions)?;
    if FileType::from_raw_mode(database_stat.st_mode) != FileType::RegularFile {
        return Err(StartupError::StateDirectoryUnavailable);
    }
    fchmod(&database, Mode::from_raw_mode(0o600))
        .map_err(|_| StartupError::StatePermissions)?;
    let database_stat = fstat(database.as_fd()).map_err(|_| StartupError::StatePermissions)?;
    if FileType::from_raw_mode(database_stat.st_mode) != FileType::RegularFile
        || database_stat.st_mode & 0o777 != 0o600
    {
        return Err(StartupError::StatePermissions);
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_walk_path(path: &Path) -> PathBuf {
    if path.starts_with(Path::new("/var")) {
        Path::new("/private").join(path.strip_prefix("/").unwrap_or(path))
    } else {
        path.to_path_buf()
    }
}

#[cfg(not(unix))]
fn ensure_portable(path: &Path) -> Result<(), StartupError> {
    use std::fs;

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(StartupError::StateDirectoryUnavailable);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| StartupError::StateDirectoryUnavailable)?;
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| StartupError::StateDirectoryUnavailable)?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(StartupError::StateDirectoryUnavailable);
            }
        }
        Err(_) => return Err(StartupError::StateDirectoryUnavailable),
    }

    let database = path.join(DATABASE_FILENAME);
    match fs::symlink_metadata(&database) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(StartupError::StateDirectoryUnavailable);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&database)
                .map_err(|_| StartupError::StatePermissions)?;
        }
        Err(_) => return Err(StartupError::StateDirectoryUnavailable),
    }

    Ok(())
}
