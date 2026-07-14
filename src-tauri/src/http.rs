use std::sync::OnceLock;
use std::time::Duration;

/// Shared HTTP client so transcription requests reuse pooled TLS
/// connections instead of paying DNS + TCP + TLS setup on every request.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(300))
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .expect("failed to build shared HTTP client")
    })
}

/// Fire-and-forget request that opens (or refreshes) a pooled connection to
/// the given host. Called when recording starts, so the connection setup
/// happens while the user is still speaking and the transcription request
/// goes out over an already-warm connection.
pub fn prewarm(url: &'static str) {
    tauri::async_runtime::spawn(async move {
        let _ = client().head(url).send().await;
    });
}
