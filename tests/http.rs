use omarchy_app_updates::{domain::*, io::http::*};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Replies {
    calls: Arc<AtomicUsize>,
    redirect: bool,
}
impl HttpTransport for Replies {
    fn send(&self, url: &str, _: &DownloadPolicy) -> AppResult<HttpResponse> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut headers = BTreeMap::new();
        let status = if self.redirect {
            headers.insert("location".into(), "https://untrusted.example/asset".into());
            302
        } else if n < 2 {
            503
        } else {
            200
        };
        Ok(HttpResponse {
            url: url.into(),
            status,
            headers,
            bytes: b"{}".to_vec(),
        })
    }
}
#[test]
fn https_hosts_and_redirects_are_checked_before_network_requests() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = HttpClient::with_transport(Arc::new(Replies {
        calls: calls.clone(),
        redirect: true,
    }));
    let policy = DownloadPolicy::for_hosts(&["trusted.example"], 100);
    assert!(client.get("http://trusted.example/data", &policy).is_err());
    assert!(client.get("https://evil.example/data", &policy).is_err());
    assert!(
        client
            .get("https://user:password@trusted.example/data", &policy)
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(client.get("https://trusted.example/data", &policy).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[test]
fn transient_http_errors_retry_at_most_three_times() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = HttpClient::with_transport(Arc::new(Replies {
        calls: calls.clone(),
        redirect: false,
    }));
    let policy = DownloadPolicy::for_hosts(&["trusted.example"], 100);
    assert_eq!(
        client
            .get("https://trusted.example/data", &policy)
            .unwrap()
            .bytes,
        b"{}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}
