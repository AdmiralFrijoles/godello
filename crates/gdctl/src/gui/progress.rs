//! A download progress sink that feeds the GUI.
//!
//! The core install flow reports progress through the DownloadProgress trait. In
//! the CLI that drives a terminal bar. In the GUI we cannot touch widgets from
//! inside the download future, so this sink forwards every callback into a
//! channel. The receiving end lives on the GUI side and turns each event into a
//! message the update loop can apply to a progress bar.

use godello_core::DownloadProgress;
use tokio::sync::mpsc::UnboundedSender;

/// One step in a download, normalized so the GUI does not depend on the trait.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// The download started. The total is the size in bytes when the server
    /// reported it, or None when the size is unknown.
    Started { total: Option<u64> },
    /// More bytes have arrived. The count is the running total downloaded so far.
    Advanced { downloaded: u64 },
    /// The download finished, either with success or with an error.
    Finished,
}

/// A DownloadProgress sink that owns a channel sender.
///
/// It is cheap to clone and holds no borrows, so it can be moved into the install
/// future and handed to the core as a trait object. The matching receiver feeds
/// the GUI.
#[derive(Clone)]
pub struct ChannelProgress {
    tx: UnboundedSender<ProgressEvent>,
}

impl ChannelProgress {
    /// Build a sink that sends events on the given channel.
    pub fn new(tx: UnboundedSender<ProgressEvent>) -> Self {
        ChannelProgress { tx }
    }
}

impl DownloadProgress for ChannelProgress {
    fn start(&self, total: Option<u64>) {
        // A send fails only when the receiver was dropped, which happens if the
        // window closed mid download. There is nothing useful to do then, so we
        // drop the event and let the download wind down on its own.
        let _ = self.tx.send(ProgressEvent::Started { total });
    }

    fn update(&self, downloaded: u64) {
        let _ = self.tx.send(ProgressEvent::Advanced { downloaded });
    }

    fn finish(&self) {
        let _ = self.tx.send(ProgressEvent::Finished);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn start_forwards_the_total() {
        let (tx, mut rx) = unbounded_channel();
        let sink = ChannelProgress::new(tx);
        sink.start(Some(2048));
        match rx.try_recv() {
            Ok(ProgressEvent::Started { total }) => assert_eq!(total, Some(2048)),
            other => panic!("expected a started event, got {other:?}"),
        }
    }

    #[test]
    fn start_forwards_an_unknown_total() {
        let (tx, mut rx) = unbounded_channel();
        let sink = ChannelProgress::new(tx);
        sink.start(None);
        match rx.try_recv() {
            Ok(ProgressEvent::Started { total }) => assert_eq!(total, None),
            other => panic!("expected a started event, got {other:?}"),
        }
    }

    #[test]
    fn update_forwards_the_running_total() {
        let (tx, mut rx) = unbounded_channel();
        let sink = ChannelProgress::new(tx);
        sink.update(512);
        match rx.try_recv() {
            Ok(ProgressEvent::Advanced { downloaded }) => assert_eq!(downloaded, 512),
            other => panic!("expected an advanced event, got {other:?}"),
        }
    }

    #[test]
    fn finish_forwards_a_finished_event() {
        let (tx, mut rx) = unbounded_channel();
        let sink = ChannelProgress::new(tx);
        sink.finish();
        assert!(matches!(rx.try_recv(), Ok(ProgressEvent::Finished)));
    }

    #[test]
    fn events_arrive_in_order() {
        let (tx, mut rx) = unbounded_channel();
        let sink = ChannelProgress::new(tx);
        sink.start(Some(10));
        sink.update(4);
        sink.update(10);
        sink.finish();
        assert!(matches!(rx.try_recv(), Ok(ProgressEvent::Started { .. })));
        assert!(matches!(
            rx.try_recv(),
            Ok(ProgressEvent::Advanced { downloaded: 4 })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(ProgressEvent::Advanced { downloaded: 10 })
        ));
        assert!(matches!(rx.try_recv(), Ok(ProgressEvent::Finished)));
    }

    #[test]
    fn sending_after_the_receiver_is_dropped_does_not_panic() {
        let (tx, rx) = unbounded_channel();
        let sink = ChannelProgress::new(tx);
        drop(rx);
        // None of these should panic even though there is no receiver left.
        sink.start(Some(100));
        sink.update(50);
        sink.finish();
    }
}
