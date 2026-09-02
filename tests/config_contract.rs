use ai_stock_forum::config::{AppPaths, StartupError};

#[test]
fn injected_paths_use_the_phase_zero_database_name() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    assert_eq!(
        paths.database_path(),
        temp.path().join("ai-stock-forum.sqlite3")
    );
}

#[test]
fn ensure_creates_an_injected_state_directory() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");

    AppPaths::for_test(&state).ensure().unwrap();

    assert!(state.is_dir());
}

#[cfg(target_os = "macos")]
#[test]
fn ensure_accepts_the_macos_tmp_system_alias() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let state = temp.path().join("state");
    let paths = AppPaths::for_test(&state);

    paths.ensure().unwrap();

    assert!(state.is_dir());
    assert!(paths.database_path().is_file());
}

#[cfg(unix)]
#[test]
fn ensure_makes_the_state_directory_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();

    AppPaths::for_test(&state).ensure().unwrap();

    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn ensure_makes_the_database_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let paths = AppPaths::for_test(&state);

    paths.ensure().unwrap();
    std::fs::set_permissions(
        paths.database_path(),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    paths.ensure().unwrap();

    assert_eq!(
        std::fs::metadata(paths.database_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn ensure_rejects_a_state_directory_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let state = temp.path().join("state");
    std::fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &state).unwrap();

    assert!(matches!(
        AppPaths::for_test(&state).ensure(),
        Err(StartupError::StateDirectoryUnavailable)
    ));
}

#[cfg(unix)]
#[test]
fn ensure_rejects_a_database_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let target = temp.path().join("outside.sqlite3");
    let paths = AppPaths::for_test(&state);
    std::fs::create_dir(&state).unwrap();
    std::fs::File::create(&target).unwrap();
    std::os::unix::fs::symlink(&target, paths.database_path()).unwrap();

    assert!(matches!(
        paths.ensure(),
        Err(StartupError::StateDirectoryUnavailable)
    ));
}

#[cfg(unix)]
#[test]
fn ensure_rejects_an_intermediate_symlink_without_touching_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let intermediate = temp.path().join("intermediate");
    let state = intermediate.join("state");
    std::fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &intermediate).unwrap();

    assert!(matches!(
        AppPaths::for_test(&state).ensure(),
        Err(StartupError::StateDirectoryUnavailable)
    ));
    assert!(!target.join("state").exists());
}

#[cfg(unix)]
#[test]
fn ensure_rejects_a_database_directory() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let paths = AppPaths::for_test(&state);
    std::fs::create_dir(&state).unwrap();
    std::fs::create_dir(paths.database_path()).unwrap();

    assert!(matches!(
        paths.ensure(),
        Err(StartupError::StateDirectoryUnavailable)
    ));
}

#[cfg(unix)]
#[test]
fn ensure_rejects_a_fifo_database_path() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let paths = AppPaths::for_test(&state);
    std::fs::create_dir(&state).unwrap();
    let database = CString::new(paths.database_path().as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(database.as_ptr(), 0o600) }, 0);

    assert!(matches!(
        paths.ensure(),
        Err(StartupError::StateDirectoryUnavailable)
    ));
}

#[cfg(unix)]
#[test]
fn ensure_rejects_a_socket_database_path() {
    use std::os::unix::net::UnixListener;

    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let paths = AppPaths::for_test(&state);
    std::fs::create_dir(&state).unwrap();
    let _listener = match UnixListener::bind(paths.database_path()) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("could not create socket fixture: {error}"),
    };

    assert!(matches!(
        paths.ensure(),
        Err(StartupError::StateDirectoryUnavailable)
    ));
}

#[test]
fn state_errors_do_not_expose_file_contents() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let secret = "credential-material-that-must-not-appear";
    std::fs::write(&state, secret).unwrap();

    let error = AppPaths::for_test(&state).ensure().unwrap_err();

    assert!(!error.to_string().contains(secret));
}
