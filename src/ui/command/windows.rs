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
}
