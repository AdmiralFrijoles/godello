//! The real network client for the CLI.
//!
//! The core library keeps all network work behind two traits, HttpClient for
//! reading remote text and Downloader for fetching engine archives to disk. The
//! core never depends on a concrete http library, so it can be tested with fakes
//! and no real requests. This module supplies the one real client built on
//! reqwest and tokio that the binary hands to the core.
//!
//! One client serves both traits. Text reads pull the whole body into memory.
//! Engine downloads can be hundreds of megabytes, so they stream to disk in
//! chunks rather than buffering the whole file.

use std::path::Path;

use godello_core::{Downloader, HttpClient, InstallError, RepositoryError};
use tokio::io::AsyncWriteExt;

/// A short identifier sent on every request. The GitHub API rejects requests
/// with no user agent, so this is required, not just polite.
const USER_AGENT: &str = concat!("godello/", env!("CARGO_PKG_VERSION"));

/// The real http client used by the binary. It wraps a reqwest client that is
/// cheap to clone and reuses connections, so one instance is shared for all
/// requests.
#[derive(Clone)]
pub struct WebClient {
    client: reqwest::Client,
}

impl WebClient {
    /// Build a client with the Godello user agent. This only fails if the TLS
    /// backend cannot start, which is fatal for a download tool, so the caller
    /// gets a plain error to report and exit.
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|err| err.to_string())?;
        Ok(WebClient { client })
    }
}

impl HttpClient for WebClient {
    async fn get_text(&self, url: &str) -> Result<String, RepositoryError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|err| RepositoryError::Network(err.to_string()))?;
        // Turn a 4xx or 5xx into an error so a missing page does not read as an
        // empty but successful body.
        let response = response
            .error_for_status()
            .map_err(|err| RepositoryError::Network(err.to_string()))?;
        response
            .text()
            .await
            .map_err(|err| RepositoryError::Network(err.to_string()))
    }
}

impl Downloader for WebClient {
    async fn download_to(&self, url: &str, dest: &Path) -> Result<(), InstallError> {
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|err| InstallError::Download(err.to_string()))?
            .error_for_status()
            .map_err(|err| InstallError::Download(err.to_string()))?;

        // Make sure the parent exists. The install flow creates the downloads
        // folder already, but this keeps the client usable on its own.
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::File::create(dest).await?;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| InstallError::Download(err.to_string()))?
        {
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    /// Build a raw HTTP response with a body and a content length.
    fn http_response(status_line: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    /// Start a one shot server on a free port. It accepts a single connection,
    /// captures the request text so a test can check headers, then writes the
    /// canned response. Returns the base url and a receiver for the request.
    fn spawn_server(response: Vec<u8>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let read = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]).to_string();
                let _ = tx.send(request);
                let _ = stream.write_all(&response);
                let _ = stream.flush();
            }
        });
        (format!("http://{addr}/"), rx)
    }

    /// An address with nothing listening, so a connection is refused at once.
    fn dead_url() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn get_text_returns_the_body_on_success() {
        let (url, _rx) = spawn_server(http_response("200 OK", b"versions: hello"));
        let client = WebClient::new().unwrap();
        let text = client.get_text(&url).await.unwrap();
        assert_eq!(text, "versions: hello");
    }

    #[tokio::test]
    async fn get_text_sends_the_user_agent() {
        let (url, rx) = spawn_server(http_response("200 OK", b"ok"));
        let client = WebClient::new().unwrap();
        client.get_text(&url).await.unwrap();
        let request = rx.recv().unwrap();
        let lowered = request.to_ascii_lowercase();
        assert!(lowered.contains("user-agent:"));
        assert!(request.contains("godello/"));
    }

    #[tokio::test]
    async fn get_text_maps_a_not_found_to_a_network_error() {
        let (url, _rx) = spawn_server(http_response("404 Not Found", b"missing"));
        let client = WebClient::new().unwrap();
        let result = client.get_text(&url).await;
        assert!(matches!(result, Err(RepositoryError::Network(_))));
    }

    #[tokio::test]
    async fn get_text_maps_a_refused_connection_to_a_network_error() {
        let client = WebClient::new().unwrap();
        let result = client.get_text(&dead_url()).await;
        assert!(matches!(result, Err(RepositoryError::Network(_))));
    }

    #[tokio::test]
    async fn download_to_writes_the_exact_bytes() {
        let body = b"the engine archive bytes";
        let (url, _rx) = spawn_server(http_response("200 OK", body));
        let dir = std::env::temp_dir().join("godello-net-tests");
        let _ = std::fs::create_dir_all(&dir);
        let dest = dir.join("download-ok.zip");
        let _ = std::fs::remove_file(&dest);

        let client = WebClient::new().unwrap();
        client.download_to(&url, &dest).await.unwrap();
        let written = std::fs::read(&dest).unwrap();
        assert_eq!(written, body);
    }

    #[tokio::test]
    async fn download_to_creates_a_missing_parent_folder() {
        let body = b"bytes";
        let (url, _rx) = spawn_server(http_response("200 OK", body));
        let dir = std::env::temp_dir()
            .join("godello-net-tests")
            .join("nested-parent");
        let _ = std::fs::remove_dir_all(&dir);
        let dest = dir.join("deep").join("file.zip");

        let client = WebClient::new().unwrap();
        client.download_to(&url, &dest).await.unwrap();
        assert!(dest.is_file());
    }

    #[tokio::test]
    async fn download_to_maps_a_not_found_to_a_download_error_and_writes_nothing() {
        let (url, _rx) = spawn_server(http_response("404 Not Found", b"missing"));
        let dir = std::env::temp_dir().join("godello-net-tests");
        let _ = std::fs::create_dir_all(&dir);
        let dest = dir.join("download-404.zip");
        let _ = std::fs::remove_file(&dest);

        let client = WebClient::new().unwrap();
        let result = client.download_to(&url, &dest).await;
        assert!(matches!(result, Err(InstallError::Download(_))));
        // The status is checked before the file is created, so a failed fetch
        // leaves no empty file behind.
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn download_to_maps_a_refused_connection_to_a_download_error() {
        let dir = std::env::temp_dir().join("godello-net-tests");
        let _ = std::fs::create_dir_all(&dir);
        let dest = dir.join("download-refused.zip");
        let _ = std::fs::remove_file(&dest);

        let client = WebClient::new().unwrap();
        let result = client.download_to(&dead_url(), &dest).await;
        assert!(matches!(result, Err(InstallError::Download(_))));
    }
}
