#[cfg(unix)]
mod unix_line_source_contract {
    use ai_stock_forum::ui::command::{CancellableLineSource, LineSourceEvent, UnixLineSource};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(2);

    fn pipe() -> [libc::c_int; 2] {
        let mut descriptors = [-1; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        descriptors
    }

    #[test]
    fn cancellation_wakes_an_in_progress_unix_read_and_allows_join() {
        let descriptors = pipe();
        let mut source = UnixLineSource::from_borrowed_fd(descriptors[0])
            .expect("Unix source should initialize");
        let cancellation = source.cancellation();
        let (result_tx, result_rx) = mpsc::sync_channel(1);

        let reader = thread::spawn(move || {
            let result = source.next_line();
            result_tx.send(result).expect("result receiver remains");
        });
        cancellation.cancel();

        let result = result_rx
            .recv_timeout(WAIT)
            .expect("cancellation must wake the blocked read");
        assert!(matches!(
            result.expect("cancellation is not an I/O error"),
            LineSourceEvent::Cancelled
        ));
        reader.join().expect("owned reader thread must join");
        unsafe {
            libc::close(descriptors[0]);
            libc::close(descriptors[1]);
        }
    }

    #[test]
    fn invalid_input_descriptor_returns_a_typed_error_without_spinning() {
        let descriptors = pipe();
        let mut source = UnixLineSource::from_borrowed_fd(descriptors[0])
            .expect("Unix source should initialize");
        let cancellation = source.cancellation();
        unsafe { libc::close(descriptors[0]) };
        let (result_tx, result_rx) = mpsc::sync_channel(1);

        let reader = thread::spawn(move || {
            let result = source.next_line();
            result_tx.send(result).expect("result receiver remains");
        });
        let result = match result_rx.recv_timeout(WAIT) {
            Ok(result) => result,
            Err(error) => {
                cancellation.cancel();
                panic!("invalid descriptor did not terminate within bound: {error}");
            }
        };
        assert!(result.is_err(), "POLLNVAL must become a typed input error");
        reader.join().expect("reader must join after POLLNVAL");
        unsafe { libc::close(descriptors[1]) };
    }

    #[test]
    fn closed_pipe_is_eof_and_does_not_spin() {
        let descriptors = pipe();
        let mut source = UnixLineSource::from_borrowed_fd(descriptors[0])
            .expect("Unix source should initialize");
        unsafe { libc::close(descriptors[1]) };
        let (result_tx, result_rx) = mpsc::sync_channel(1);

        let reader = thread::spawn(move || {
            let result = source.next_line();
            result_tx.send(result).expect("result receiver remains");
        });
        let result = result_rx
            .recv_timeout(WAIT)
            .expect("closed pipe must terminate within bound")
            .expect("pipe hangup is a clean input termination");
        assert!(matches!(result, LineSourceEvent::Eof));
        reader.join().expect("reader must join after EOF");
        unsafe { libc::close(descriptors[0]) };
    }
}
