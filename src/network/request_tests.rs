//! Count real HTTP exchanges through production action/cache paths against an
//! isolated loopback GitHub fixture. No live API budget or credentials are used.
use super::*;
use std::{
    collections::HashMap,
    net::TcpListener,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

static ROUTES: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();

pub(super) fn with_route(root: &Path, base: &str, run: impl FnOnce()) {
    ROUTES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(root.into(), base.into());
    run();
    ROUTES.get().unwrap().lock().unwrap().remove(root);
}

pub(super) fn route<'a>(root: &Path, url: &'a str) -> std::borrow::Cow<'a, str> {
    if let Some(path) = url.strip_prefix("https://api.github.com") {
        if let Some(base) = ROUTES
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .get(root)
        {
            return std::borrow::Cow::Owned(format!("{base}{path}"));
        }
    }
    std::borrow::Cow::Borrowed(url)
}

struct Server {
    root: PathBuf,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(responses: HashMap<String, (u16, serde_json::Value)>) -> Self {
        let root = cache_tests::root();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        ROUTES.get_or_init(Default::default).lock().unwrap().insert(
            root.clone(),
            format!("http://{}", listener.local_addr().unwrap()),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let seen = requests.clone();
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("Fixture server: {e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut input = Vec::new();
                while !input.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut buf = [0; 2048];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    input.extend_from_slice(&buf[..n]);
                    assert!(input.len() < 32768);
                }
                let text = String::from_utf8(input).unwrap();
                assert!(!text.to_ascii_lowercase().contains("authorization:"));
                let path = text
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                seen.lock().unwrap().push(path.into());
                let (status, value) = responses
                    .get(path)
                    .unwrap_or_else(|| panic!("Unexpected request: {path}"));
                let body = serde_json::to_vec(value).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self {
            root,
            requests,
            stop,
            thread: Some(thread),
        }
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn measured<T>(&self, label: &str, action: impl FnOnce() -> T) -> T {
        let before = self.count();
        let result = action();
        println!("{label}: {} HTTP requests", self.count() - before);
        result
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.thread.take().unwrap().join();
        ROUTES.get().unwrap().lock().unwrap().remove(&self.root);
        cache::clear(&self.root).unwrap();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn measure_release_actions_cold_and_warm() {
    use crate::{
        model::{Installed, Paths, Preferences},
        updates,
    };
    let release = |tag: &str| {
        serde_json::json!({
            "tag_name":tag, "draft":false,"prerelease":false,
            "assets":(["windows-x64-portable.zip", "linux-x64.AppImage", "macos-universal.dmg"].into_iter().map(|suffix| serde_json::json!({"name":format!("filmcraft-{}-{suffix}", &tag[1..]),
                "size":100,"browser_download_url":format!("https://github.com/storytold/filmcraft/releases/download/{tag}/filmcraft-{}-{suffix}", &tag[1..]),
                "digest":format!("sha256:{}", "a".repeat(64))})).collect::<Vec<_>>())
        })
    };
    let server = Server::new(HashMap::from([
        (
            "/repos/storytold/filmcraft/releases/latest".into(),
            (200, release("v1.2.0")),
        ),
        (
            "/repos/storytold/filmcraft/releases?per_page=100".into(),
            (
                200,
                serde_json::json!([release("v1.2.0"), release("v1.1.0")]),
            ),
        ),
        (
            "/repos/storytold/filmcraft/releases/tags/v1.1.0".into(),
            (200, release("v1.1.0")),
        ),
    ]));
    let paths = Paths::new(server.root.clone(), None);
    let base = paths.read_preferences().unwrap();
    let mut prefs = Preferences {
        release_format: "portable".into(),
        ..base.clone()
    };
    prefs.app_install_options.insert(
        "filmcraft".into(),
        crate::install_options::InstallOptions {
            release_format: "portable".into(),
            architecture: crate::install_options::host_architecture(),
        },
    );
    crate::settings::save_preferences_patch(&paths, &base, &prefs).unwrap();
    let job = Job::new(paths.at("logs/requests.log"), &Default::default());
    let config = paths.config().unwrap();
    let app = Installed {
        name: "filmcraft".into(),
        version: "1.0.0".into(),
        path: server.root.join("FilmCraft").display().to_string(),
        install_kind: "portable".into(),
        architecture: crate::install_options::host_architecture(),
        ..Default::default()
    };
    for label in ["Check update cold", "Check update warm"] {
        server
            .measured(label, || updates::check_installation(&paths, &app, &prefs))
            .unwrap();
    }
    for label in ["Latest preview after check", "Latest preview again"] {
        server
            .measured(label, || {
                updates::plan_install_preview(
                    &paths,
                    &job,
                    "filmcraft",
                    None,
                    false,
                    config.clone(),
                )
            })
            .unwrap();
    }
    for label in ["Version list cold", "Version list warm"] {
        server
            .measured(label, || updates::available_versions(&paths, "filmcraft"))
            .unwrap();
    }
    let after_history = server.count();
    for label in [
        "Older version cold",
        "Older version warm",
        "Format/architecture re-selection",
    ] {
        server
            .measured(label, || {
                updates::plan_install_preview(
                    &paths,
                    &job,
                    "filmcraft",
                    Some("v1.1.0"),
                    false,
                    config.clone(),
                )
            })
            .unwrap();
    }
    assert_eq!(
        server.count(),
        after_history,
        "Listed releases already contain their package details"
    );
    assert_eq!(server.count(), 2);
}

#[test]
fn measure_discovery_without_releases() {
    let server = Server::new(HashMap::from([
        (
            "/orgs/storytold/repos?per_page=100&page=1".into(),
            (
                200,
                serde_json::json!([{"name":"fixturecraft","archived":false,"fork":false}]),
            ),
        ),
        (
            "/repos/storytold/fixturecraft/releases/latest".into(),
            (404, serde_json::json!({"message":"Not Found"})),
        ),
    ]));
    for label in ["Startup discovery cold", "Startup discovery warm"] {
        server
            .measured(label, || {
                crate::catalog::refresh_for(&server.root, crate::catalog::DiscoveryTrigger::Startup)
            })
            .unwrap();
    }
    let before = server.count();
    for _ in 0..3 {
        let result: Option<serde_json::Value> = Network::new(&server.root)
            .unwrap()
            .json_optional_with_policy(
                "https://api.github.com/repos/storytold/fixturecraft/releases/latest",
                CachePolicy::Discovery,
            )
            .unwrap();
        assert!(result.is_none());
    }
    println!(
        "Repeat missing-release lookups: {} HTTP requests",
        server.count() - before
    );
    assert_eq!(
        server.count() - before,
        0,
        "A known missing release should not be queried repeatedly within its freshness window"
    );
}

#[test]
fn withdrawn_tag_retires_old_history_including_its_persisted_copy() {
    let release =
        serde_json::json!({"tag_name":"v1.0.0","draft":false,"prerelease":false,"assets":[]});
    let server = Server::new(HashMap::from([
        (
            "/repos/fixture/app/releases?per_page=100".into(),
            (200, serde_json::json!([release])),
        ),
        (
            "/repos/fixture/app/releases/tags/v1.0.0".into(),
            (404, serde_json::json!({"message":"Not Found"})),
        ),
    ]));
    let history = "https://api.github.com/repos/fixture/app/releases?per_page=100";
    let tag = "https://api.github.com/repos/fixture/app/releases/tags/v1.0.0";
    let network = Network::new(&server.root).unwrap();
    network.json::<serde_json::Value>(history).unwrap();
    assert!(network
        .json_with_policy::<serde_json::Value>(tag, CachePolicy::Update)
        .is_err());
    let key = cache::key(
        &server.root,
        history,
        github_token().as_ref().map(|(_, token)| token.as_str()),
    );
    let disk = disk_cache::DiskCache::new(&key.root.join("runtime/api-cache"));
    assert!(
        disk.read(&key.hash, key.legacy.as_deref())
            .unwrap()
            .is_none(),
        "A restart must not revive withdrawn tags through old history"
    );
    assert!(network.json::<serde_json::Value>(tag).is_err());
}
