use std::sync::OnceLock;

use crossbeam_channel::{Receiver, bounded};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum InterruptError {
    #[error("interrupt handler unavailable")]
    Install,
}

struct InterruptBus {
    receiver: Receiver<()>,
}

static INTERRUPT_BUS: OnceLock<Result<InterruptBus, ()>> = OnceLock::new();

pub(crate) fn receiver() -> Result<Receiver<()>, InterruptError> {
    let bus = INTERRUPT_BUS.get_or_init(|| {
        let (sender, receiver) = bounded(1);
        let handler_sender = sender.clone();
        ctrlc::set_handler(move || {
            let _ = handler_sender.try_send(());
        })
        .map_err(|_| ())?;
        Ok(InterruptBus { receiver })
    });
    let receiver = bus
        .as_ref()
        .map_err(|_| InterruptError::Install)?
        .receiver
        .clone();
    drain_pending(&receiver);
    Ok(receiver)
}

fn drain_pending(receiver: &Receiver<()>) {
    while receiver.try_recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{TryRecvError, bounded};

    use super::drain_pending;

    #[test]
    fn stale_interrupts_are_drained_before_a_host_starts() {
        let (sender, receiver) = bounded(2);
        sender.try_send(()).unwrap();
        sender.try_send(()).unwrap();
        drain_pending(&receiver);
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }
}
