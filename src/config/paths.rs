use std::fs::{self, FileType, OpenOptions};
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

    pub fn ensure(&self) -> Result<(), StartupError> {
        ensure_state_directory(&self.state_dir)?;
        ensure_database_file(&self.database_path())
    }
}

fn ensure_state_directory(path: &Path) -> Result<(), StartupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !is_directory(&metadata.file_type()) {
                return Err(StartupError::StateDirectoryUnavailable);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| StartupError::StateDirectoryUnavailable)?;
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| StartupError::StateDirectoryUnavailable)?;
            if !is_directory(&metadata.file_type()) {
                return Err(StartupError::StateDirectoryUnavailable);
            }
        }
        Err(_) => return Err(StartupError::StateDirectoryUnavailable),
    }

    set_directory_permissions(path)
}

fn ensure_database_file(path: &Path) -> Result<(), StartupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !is_regular_file(&metadata.file_type()) {
                return Err(StartupError::StateDirectoryUnavailable);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            configure_new_file_permissions(&mut options);
            match options.open(path) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(path)
                        .map_err(|_| StartupError::StateDirectoryUnavailable)?;
                    if !is_regular_file(&metadata.file_type()) {
                        return Err(StartupError::StateDirectoryUnavailable);
                    }
                }
                Err(_) => return Err(StartupError::StatePermissions),
            }
        }
        Err(_) => return Err(StartupError::StateDirectoryUnavailable),
    }

    set_database_permissions(path)
}

fn is_directory(file_type: &FileType) -> bool {
    file_type.is_dir() && !file_type.is_symlink()
}

fn is_regular_file(file_type: &FileType) -> bool {
    file_type.is_file() && !file_type.is_symlink()
}

#[cfg(unix)]
fn set_directory_permissions(path: &Path) -> Result<(), StartupError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| StartupError::StatePermissions)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| StartupError::StatePermissions)?;
    if !is_directory(&metadata.file_type()) || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(StartupError::StatePermissions);
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &Path) -> Result<(), StartupError> {
    Ok(())
}

#[cfg(unix)]
fn configure_new_file_permissions(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_new_file_permissions(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_database_permissions(path: &Path) -> Result<(), StartupError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| StartupError::StatePermissions)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| StartupError::StatePermissions)?;
    if !is_regular_file(&metadata.file_type()) || metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(StartupError::StatePermissions);
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_database_permissions(path: &Path) -> Result<(), StartupError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| StartupError::StatePermissions)?;
    if !is_regular_file(&metadata.file_type()) {
        return Err(StartupError::StatePermissions);
    }
    Ok(())
}
