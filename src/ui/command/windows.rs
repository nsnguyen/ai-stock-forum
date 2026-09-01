#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_OPERATION_ABORTED,
};

#[cfg(not(windows))]
const ERROR_BROKEN_PIPE: u32 = 109;
#[cfg(not(windows))]
const ERROR_HANDLE_EOF: u32 = 38;
#[cfg(not(windows))]
const ERROR_OPERATION_ABORTED: u32 = 995;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadErrorDisposition {
    Cancelled,
    Eof,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadPhase {
    IdleWaiting,
    AboutToRead,
    ReadActive,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CancelAttempt {
    Succeeded,
    NotFound,
    Failed(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CancelDecision {
    Complete,
    Retry,
    Failed(u32),
}

pub(super) fn cancellation_decision(
    phase: ReadPhase,
    attempt: CancelAttempt,
) -> CancelDecision {
    match attempt {
        CancelAttempt::Failed(error) => CancelDecision::Failed(error),
        CancelAttempt::Succeeded => CancelDecision::Complete,
        CancelAttempt::NotFound => match phase {
            ReadPhase::AboutToRead | ReadPhase::ReadActive => CancelDecision::Retry,
            ReadPhase::IdleWaiting | ReadPhase::Exited => CancelDecision::Complete,
        },
    }
}

pub(super) fn may_begin_read(cancellation_requested: bool) -> bool {
    !cancellation_requested
}

pub(super) fn classify_read_error(
    error_code: u32,
    cancellation_requested: bool,
) -> ReadErrorDisposition {
    match error_code {
        ERROR_OPERATION_ABORTED if cancellation_requested => ReadErrorDisposition::Cancelled,
        ERROR_BROKEN_PIPE | ERROR_HANDLE_EOF => ReadErrorDisposition::Eof,
        _ => ReadErrorDisposition::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_synchronous_read_is_clean_only_after_cancellation() {
        assert_eq!(
            classify_read_error(ERROR_OPERATION_ABORTED, true),
            ReadErrorDisposition::Cancelled
        );
        assert_eq!(
            classify_read_error(ERROR_OPERATION_ABORTED, false),
            ReadErrorDisposition::Error
        );
    }

    #[test]
    fn broken_pipe_and_handle_eof_are_clean_input_termination() {
        assert_eq!(
            classify_read_error(ERROR_BROKEN_PIPE, false),
            ReadErrorDisposition::Eof
        );
        assert_eq!(
            classify_read_error(ERROR_HANDLE_EOF, false),
            ReadErrorDisposition::Eof
        );
    }

    #[test]
    fn unrelated_windows_read_error_remains_typed_as_an_error() {
        assert_eq!(
            classify_read_error(5, false),
            ReadErrorDisposition::Error
        );
    }

    #[test]
    fn cancel_before_pending_retries_until_read_or_exit_acknowledges() {
        assert_eq!(
            cancellation_decision(ReadPhase::AboutToRead, CancelAttempt::NotFound),
            CancelDecision::Retry
        );
        assert_eq!(
            cancellation_decision(ReadPhase::ReadActive, CancelAttempt::NotFound),
            CancelDecision::Retry
        );
        assert_eq!(
            cancellation_decision(ReadPhase::Exited, CancelAttempt::NotFound),
            CancelDecision::Complete
        );
        assert_eq!(
            cancellation_decision(ReadPhase::IdleWaiting, CancelAttempt::NotFound),
            CancelDecision::Complete
        );
        assert_eq!(
            cancellation_decision(ReadPhase::ReadActive, CancelAttempt::Succeeded),
            CancelDecision::Complete
        );
    }

    #[test]
    fn persistent_cancellation_prevents_another_read_and_preserves_real_errors() {
        assert!(!may_begin_read(true));
        assert!(may_begin_read(false));
        assert_eq!(
            cancellation_decision(ReadPhase::ReadActive, CancelAttempt::Failed(5)),
            CancelDecision::Failed(5)
        );
    }
}
