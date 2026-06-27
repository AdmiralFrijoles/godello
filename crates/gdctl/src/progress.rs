//! A terminal progress bar for downloads.
//!
//! This is the display side of the core DownloadProgress hook. It draws a bar to
//! stderr while an engine downloads. When stderr is not a terminal, for example
//! in a pipe or a CI job, it stays hidden so it does not spam the log. The bar
//! clears itself when the download ends, leaving the surrounding messages clean.

use std::io::IsTerminal;
use std::time::Duration;

use godello_core::DownloadProgress;
use indicatif::{ProgressBar, ProgressStyle};

/// A download progress bar. Holds an indicatif bar whose methods take a shared
/// reference, which matches the DownloadProgress contract.
pub struct BarProgress {
    bar: ProgressBar,
}

impl BarProgress {
    /// Build a bar labeled with what is downloading. It draws only when stderr
    /// is a terminal.
    pub fn new(label: String) -> Self {
        let bar = if std::io::stderr().is_terminal() {
            ProgressBar::new_spinner()
        } else {
            ProgressBar::hidden()
        };
        bar.set_message(label);
        BarProgress { bar }
    }
}

impl DownloadProgress for BarProgress {
    fn start(&self, total: Option<u64>) {
        match total {
            // A known size gets a real bar with sizes, speed, and an estimate.
            Some(total) => {
                self.bar.set_length(total);
                if let Ok(style) = ProgressStyle::with_template(
                    "{msg}  {bar:30} {bytes}/{total_bytes}  {bytes_per_sec}  {eta}",
                ) {
                    self.bar.set_style(style.progress_chars("=> "));
                }
            }
            // No size means a spinner that ticks while bytes arrive.
            None => {
                self.bar.enable_steady_tick(Duration::from_millis(120));
                if let Ok(style) = ProgressStyle::with_template("{msg}  {spinner}  {bytes}") {
                    self.bar.set_style(style);
                }
            }
        }
    }

    fn update(&self, downloaded: u64) {
        self.bar.set_position(downloaded);
    }

    fn finish(&self) {
        self.bar.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driving_a_known_size_bar_does_not_panic() {
        let bar = BarProgress::new("standard 4.3-stable".to_string());
        bar.start(Some(100));
        bar.update(40);
        bar.update(100);
        bar.finish();
    }

    #[test]
    fn driving_an_unknown_size_bar_does_not_panic() {
        let bar = BarProgress::new("standard 4.3-stable".to_string());
        bar.start(None);
        bar.update(10);
        bar.finish();
    }
}
