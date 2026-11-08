use std::{
    fs,
    io,
    os::unix::{fs::{symlink, FileTypeExt, PermissionsExt}, net::UnixListener as StdUnixListener},
    path::{Path, PathBuf},
    sync::{Arc, atomic::{AtomicUsize, Ordering}},
    time::Duration,
};

use serde_json::{json, Value};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::UnixStream, time::timeout};

use super::{
    client::Client,
    socket::{self, Operation, Origin, Request, Server, MAX_FRAME_BYTES},
};

// Each test owns its entire root and never touches an existing service.
struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cockpit-voice-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path { &self.0 }
}

impl Drop for TestRoot {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn bind(root: &TestRoot, calls: Arc<AtomicUsize>) -> io::Result<Server> {
    Server::bind(root.path(), move |request: Request| {
        calls.fetch_add(1, Ordering::SeqCst);
        async move {
            let origin = match request.origin { Origin::Native => "native", Origin::Web => "web" };
            let (kind, payload) = match request.operation {
                Operation::Connect(value) => ("connect", value),
                Operation::Events(value) => ("events", value),
                Operation::Control(value) => ("control", value),
            };
            Ok(json!({"origin": origin, "kind": kind, "payload": payload}))
        }
    })
}

async fn rejected_frame(root: &TestRoot, frame: &[u8]) {
    let mut stream = UnixStream::connect(socket::socket_path(root.path())).await.unwrap();
    // An oversized request can be rejected before the client finishes writing.
    let _ = stream.write_all(frame).await;
    let _ = stream.shutdown().await;
    let mut response = Vec::new();
    let result = timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await.expect("server did not close rejected connection");
    if let Err(error) = result {
        assert!(matches!(error.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe), "{error}");
    }
    assert!(response.is_empty(), "rejected request received a response: {response:?}");
}

#[tokio::test]
async fn bind_refuses_symlinks_and_occupied_paths_without_removing_them() {
    let calls = Arc::new(AtomicUsize::new(0));

    let root = TestRoot::new();
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    symlink(&target, root.path().join("voice")).unwrap();
    assert!(bind(&root, calls.clone()).is_err());
    assert!(fs::symlink_metadata(root.path().join("voice")).unwrap().file_type().is_symlink());
    assert!(target.is_dir());

    let root = TestRoot::new();
    fs::create_dir(root.path().join("voice")).unwrap();
    fs::set_permissions(root.path().join("voice"), fs::Permissions::from_mode(0o700)).unwrap();
    let path = socket::socket_path(root.path());
    let target = root.path().join("target");
    fs::write(&target, b"keep").unwrap();
    symlink(&target, &path).unwrap();
    assert!(bind(&root, calls.clone()).is_err());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    fs::remove_file(&path).unwrap();

    fs::write(&path, b"occupied").unwrap();
    assert!(bind(&root, calls.clone()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"occupied");
    fs::remove_file(&path).unwrap();

    // Even an inactive socket is not assumed to belong to this server.
    drop(StdUnixListener::bind(&path).unwrap());
    assert!(bind(&root, calls.clone()).is_err());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn only_supported_operations_reach_the_callback() {
    let root = TestRoot::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let server = bind(&root, calls.clone()).unwrap();
    assert_eq!(fs::symlink_metadata(socket::socket_path(root.path())).unwrap().permissions().mode() & 0o777, 0o600);
    socket::verify_socket(root.path()).unwrap();

    let native = Client::new(root.path(), Origin::Native);
    let web = Client::new(root.path(), Origin::Web);
    assert_eq!(native.connect(json!({"a": 1})).await.unwrap(), json!({"origin":"native", "kind":"connect", "payload":{"a":1}}));
    assert_eq!(web.events(json!([1, 2])).await.unwrap(), json!({"origin":"web", "kind":"events", "payload":[1,2]}));
    assert_eq!(native.control(Value::Null).await.unwrap(), json!({"origin":"native", "kind":"control", "payload":null}));
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    for frame in [
        b"not json\n".as_slice(),
        b"{\"version\":1,\"origin\":\"native\",\"operation\":{\"kind\":\"execute\",\"payload\":{}}}\n",
        b"{\"version\":2,\"origin\":\"native\",\"operation\":{\"kind\":\"connect\",\"payload\":{}}}\n",
        b"{\"version\":1,\"origin\":\"forged\",\"operation\":{\"kind\":\"connect\",\"payload\":{}}}\n",
        b"{\"version\":1,\"origin\":\"native\",\"operation\":{\"kind\":\"connect\",\"payload\":{}},\"extra\":true}\n",
        b"{\"version\":1,\"origin\":\"native\",\"operation\":{\"kind\":\"connect\",\"payload\":{}}}",
    ] {
        rejected_frame(&root, frame).await;
    }
    let oversized = vec![b'x'; MAX_FRAME_BYTES + 1];
    rejected_frame(&root, &oversized).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    server.shutdown().await;
}

#[tokio::test]
async fn shutdown_removes_only_the_bound_socket_inode() {
    let calls = Arc::new(AtomicUsize::new(0));
    let root = TestRoot::new();
    let server = bind(&root, calls.clone()).unwrap();
    let path = socket::socket_path(root.path());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
    server.shutdown().await;
    assert_eq!(fs::symlink_metadata(&path).unwrap_err().kind(), io::ErrorKind::NotFound);

    let server = bind(&root, calls.clone()).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"replacement").unwrap();
    server.shutdown().await;
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
    fs::remove_file(&path).unwrap();

    let server = bind(&root, calls).unwrap();
    fs::remove_file(&path).unwrap();
    let replacement = StdUnixListener::bind(&path).unwrap();
    server.shutdown().await;
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_socket());
    drop(replacement);
}
