use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
};

pub(super) fn root() -> std::path::PathBuf {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/api-cache-fixtures")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root).unwrap();
    root
}

pub(crate) fn seed(root: &Path, url: &str, at: i64, value: serde_json::Value) {
    let token = url
        .starts_with("https://api.github.com/")
        .then(github_token)
        .flatten();
    let key = cache::key(root, url, token.as_ref().map(|(_, token)| token.as_str()));
    let disk = disk_cache::DiskCache::new(&key.root.join("runtime/api-cache"));
    let _guard = disk.lock(&key.hash).unwrap();
    disk.write(&key.hash, at, None, &value).unwrap();
    cache::forget(root);
}

#[test]
fn cache_write_failure_does_not_fail_successful_response() {
    let root = root();
    fs::create_dir_all(root.join("runtime")).unwrap();
    fs::write(root.join("runtime/api-cache"), b"unwritable cache fixture").unwrap();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/data", server.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        assert!(stream.read(&mut [0; 2048]).unwrap() > 0);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
            )
            .unwrap();
    });
    let result = Network::new(&root).unwrap().json::<serde_json::Value>(&url);
    worker.join().unwrap();
    assert!(
        result.is_ok(),
        "A disposable cache failed a valid response: {result:?}"
    );
    assert_eq!(result.unwrap()["ok"], true);
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}

fn reply(stream: &mut std::net::TcpStream, status: &str, headers: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
}

#[test]
fn denied_http_responses_do_not_confuse_reset_headers_with_exhausted_quota() {
    for (status, remaining, message, expected) in [
        (
            "403 Forbidden",
            51,
            "Resource not accessible by personal access token",
            "denied",
        ),
        (
            "403 Forbidden",
            51,
            "You have exceeded a secondary rate limit.",
            "temporarily",
        ),
        (
            "429 Too Many Requests",
            51,
            "Please slow down",
            "temporarily",
        ),
        (
            "403 Forbidden",
            0,
            "API rate limit exceeded",
            "API limit reached",
        ),
    ] {
        let root = root();
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/repos/fixture/releases",
            server.local_addr().unwrap()
        );
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            assert!(stream.read(&mut [0; 4096]).unwrap() > 0);
            reply(
                &mut stream,
                status,
                &format!("X-RateLimit-Remaining: {remaining}\r\nX-RateLimit-Reset: 2000000000\r\n"),
                &serde_json::to_vec(&serde_json::json!({"message":message})).unwrap(),
            );
        });
        let error = Network::new(&root)
            .unwrap()
            .json::<serde_json::Value>(&url)
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
        if remaining > 0 {
            assert!(!error.contains("resets at"), "{error}");
        }
        worker.join().unwrap();
        cache::forget(&root);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn api_metadata_requests_revalidate_intermediary_caches() {
    let root = root();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", server.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        let mut input = [0; 4096];
        let n = stream.read(&mut input).unwrap();
        let revalidates = String::from_utf8_lossy(&input[..n])
            .to_ascii_lowercase()
            .contains("cache-control: no-cache");
        reply(&mut stream, "200 OK", "", b"{}");
        revalidates
    });
    request_tests::with_route(&root, &address, || {
        Network::new(&root)
            .unwrap()
            .json::<serde_json::Value>("https://api.github.com/repos/fixture/app")
            .unwrap();
    });
    assert!(
        worker.join().unwrap(),
        "An actual API fetch must revalidate intermediary responses"
    );
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_latest_release_invalidates_history_even_for_same_version_asset_changes() {
    let root = root();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!(
        "http://{}/repos/owner/app/releases",
        server.local_addr().unwrap()
    );
    let latest = format!("{base}/latest");
    let versions = format!("{base}?per_page=100");
    let tagged = format!("{base}/tags/v1.0");
    let now = chrono::Utc::now().timestamp();
    seed(
        &root,
        &latest,
        now - 61,
        serde_json::json!({"tag_name":"v1.0","assets":[{"size":10}]}),
    );
    seed(
        &root,
        &versions,
        now,
        serde_json::json!([{"tag_name":"v1.0"}]),
    );
    seed(&root, &tagged, now, serde_json::json!({"tag_name":"v1.0"}));
    let network = Network::new(&root).unwrap();
    let _: serde_json::Value = network.json(&versions).unwrap(); // Warm old memory too.
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        assert!(stream.read(&mut [0; 4096]).unwrap() > 0);
        reply(
            &mut stream,
            "200 OK",
            "",
            br#"{"tag_name":"v1.0","assets":[{"size":20}]}"#,
        );
    });
    let value: serde_json::Value = network
        .json_with_policy(&latest, CachePolicy::Update)
        .unwrap();
    worker.join().unwrap();
    assert_eq!(value["assets"][0]["size"], 20);
    assert!(Network::cached_json::<serde_json::Value>(&root, &versions).is_none());
    assert!(Network::cached_json::<serde_json::Value>(&root, &tagged).is_none());
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unchanged_latest_release_keeps_the_cached_version_list() {
    let root = root();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!(
        "http://{}/repos/owner/app/releases",
        server.local_addr().unwrap()
    );
    let latest = format!("{base}/latest");
    let versions = format!("{base}?per_page=100");
    let value = serde_json::json!({"tag_name":"v1.0","assets":[]});
    let now = chrono::Utc::now().timestamp();
    seed(&root, &latest, now - 61, value);
    seed(
        &root,
        &versions,
        now,
        serde_json::json!([{"tag_name":"v1.0"}]),
    );
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        assert!(stream.read(&mut [0; 4096]).unwrap() > 0);
        reply(
            &mut stream,
            "200 OK",
            "",
            br#"{"tag_name":"v1.0","assets":[]}"#,
        );
    });
    let _: serde_json::Value = Network::new(&root)
        .unwrap()
        .json_with_policy(&latest, CachePolicy::Update)
        .unwrap();
    worker.join().unwrap();
    assert!(Network::cached_json::<serde_json::Value>(&root, &versions).is_some());
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unchanged_response_reuses_body_and_optional_missing_preserves_404_semantics() {
    let root = root();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/data", server.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        for index in 0..3 {
            let (mut stream, _) = server.accept().unwrap();
            let mut bytes = [0; 4096];
            let size = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]).to_ascii_lowercase();
            match index {
                0 => reply(
                    &mut stream,
                    "200 OK",
                    "ETag: \"same\"\r\n",
                    br#"{"ok":true}"#,
                ),
                1 => {
                    assert!(request.contains("if-none-match: \"same\""));
                    reply(&mut stream, "304 Not Modified", "", b"");
                }
                _ => reply(&mut stream, "404 Not Found", "", b"missing"),
            }
        }
    });
    let network = Network::new(&root).unwrap();
    assert_eq!(network.json::<serde_json::Value>(&url).unwrap()["ok"], true);
    let key = cache::key(&root, &url, None);
    let path = root
        .join("runtime/api-cache")
        .join(format!("{}.body", key.hash));
    let before = (
        fs::read(&path).unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap(),
    );
    cache::forget(&root);
    assert_eq!(
        network.json_fresh::<serde_json::Value>(&url).unwrap()["ok"],
        true
    );
    assert_eq!(
        (
            fs::read(&path).unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        ),
        before
    );
    assert!(network
        .json_optional::<serde_json::Value>(&format!("{url}/missing"))
        .unwrap()
        .is_none());
    worker.join().unwrap();
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_store_responses_are_not_reused() {
    let root = root();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/data", server.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        for index in 1..=2 {
            let (mut stream, _) = server.accept().unwrap();
            assert!(stream.read(&mut [0; 4096]).unwrap() > 0);
            reply(
                &mut stream,
                "200 OK",
                "Cache-Control: no-store\r\n",
                format!("{{\"value\":{index}}}").as_bytes(),
            );
        }
    });
    let network = Network::new(&root).unwrap();
    assert_eq!(network.json::<serde_json::Value>(&url).unwrap()["value"], 1);
    assert_eq!(network.json::<serde_json::Value>(&url).unwrap()["value"], 2);
    worker.join().unwrap();
    assert!(Network::cached_json::<serde_json::Value>(&root, &url).is_none());
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_store_missing_responses_are_not_remembered() {
    let root = root();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/missing", server.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = server.accept().unwrap();
            assert!(stream.read(&mut [0; 4096]).unwrap() > 0);
            reply(
                &mut stream,
                "404 Not Found",
                "Cache-Control: no-store\r\n",
                b"missing",
            );
        }
    });
    let network = Network::new(&root).unwrap();
    for _ in 0..2 {
        assert!(network
            .json_optional::<serde_json::Value>(&url)
            .unwrap()
            .is_none());
    }
    worker.join().unwrap();
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_cache_is_promoted_without_network_and_scoped_to_library_and_credential() {
    let root = root();
    let url = "http://127.0.0.1:9/legacy-fixture";
    let key = cache::key(&root, url, None);
    let legacy = root
        .join("runtime/api-cache")
        .join(format!("{}.json", key.hash));
    files::write_json(&legacy,&serde_json::json!({"at":chrono::Utc::now().timestamp(),"etag":"legacy","value":{"preserved":true}})).unwrap();
    assert_eq!(
        Network::new(&root)
            .unwrap()
            .json::<serde_json::Value>(url)
            .unwrap()["preserved"],
        true
    );
    assert!(!legacy.exists());
    assert!(root
        .join("runtime/api-cache")
        .join(format!("{}.meta.json", key.hash))
        .exists());
    assert_ne!(key, cache::key(&root.join("other"), url, None));
    assert_ne!(
        cache::key(&root, url, Some("fixture-a")),
        cache::key(&root, url, Some("fixture-b"))
    );
    assert!(cache::key(&root, url, Some("fixture-a")).legacy.is_none());
    cache::forget(&root);
    fs::remove_dir_all(root).unwrap();
}
