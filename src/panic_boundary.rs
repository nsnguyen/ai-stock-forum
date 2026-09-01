use std::{
    cell::Cell,
    panic::{self, UnwindSafe},
    sync::Once,
};

thread_local! {
    static SENSITIVE_DEPTH: Cell<usize> = const { Cell::new(0) };
}

static INSTALL_REDACTING_HOOK: Once = Once::new();

struct SensitiveCatchGuard;

impl SensitiveCatchGuard {
    fn enter() -> Self {
        install_redacting_hook();
        SENSITIVE_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        Self
    }
}

impl Drop for SensitiveCatchGuard {
    fn drop(&mut self) {
        SENSITIVE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn install_redacting_hook() {
    INSTALL_REDACTING_HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |information| {
            let sensitive = SENSITIVE_DEPTH.with(|depth| depth.get() > 0);
            if !sensitive {
                previous(information);
            }
        }));
    });
}

pub(crate) fn catch_sensitive_unwind<F, R>(operation: F) -> std::thread::Result<R>
where
    F: FnOnce() -> R + UnwindSafe,
{
    let _guard = SensitiveCatchGuard::enter();
    panic::catch_unwind(operation)
}

#[cfg(test)]
mod tests {
    use std::{panic::AssertUnwindSafe, process::Command};

    const CHILD_ENV: &str = "AI_STOCK_FORUM_SENSITIVE_PANIC_CHILD";
    const SECRET: &str = "credential=round-two-secret-panic-payload";
    const SAFE_LINE: &str = "Command host stopped unexpectedly.\n";

    #[test]
    fn caught_sensitive_panic_subprocess_emits_only_one_safe_line() {
        if std::env::var_os(CHILD_ENV).is_some() {
            let caught = super::catch_sensitive_unwind(AssertUnwindSafe(|| panic!("{SECRET}")));
            assert!(caught.is_err());
            eprint!("{SAFE_LINE}");
            std::process::exit(0);
        }

        let output = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "panic_boundary::tests::caught_sensitive_panic_subprocess_emits_only_one_safe_line",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .expect("sensitive panic child should run");

        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
        assert_eq!(stderr, SAFE_LINE);
        assert!(!stderr.contains(SECRET));
    }
}
