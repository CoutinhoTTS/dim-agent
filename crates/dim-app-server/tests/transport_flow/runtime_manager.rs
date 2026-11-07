//! RuntimeManager (RuntimeManager §3): the runtimes arrive after startup and
//! the plugin rows waiting for them start by themselves. `system` sources
//! keep the interpreters off the network; the pinned packages are installed
//! for real (`npm ci`, `pip install --require-hashes`).
use super::*;
use dim_app_server::{
    runtime_manager::{Kind, Runtimes},
    session_typescript::SessionTypeScriptConfig,
};
use std::path::{Path, PathBuf};

/// The `runtime.list` entry of `runtime` once its status is `status`.
async fn status(client: &mut Client, runtime: usize, status: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let list = client.rpc("runtime.list", json!({})).await;
            assert_ok(&list);
            if list["result"][runtime]["status"] == status {
                return list["result"][runtime].clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("runtime {runtime} did not become {status}"))
}

async fn appears(path: &Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    while !path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} did not appear",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The unpacked archive directory holding `interpreter` (RuntimeManager
/// §2.3): `bin/<name>` inside it, or the executable beside it on Windows.
fn archive_directory(interpreter: &Path) -> PathBuf {
    let directory = interpreter.parent().expect("the interpreter's directory");
    if cfg!(windows) {
        directory.to_owned()
    } else {
        directory
            .parent()
            .expect("the archive directory")
            .to_owned()
    }
}

/// The first `program` on this process's `PATH`: the interpreter a `system`
/// source resolves to (§2.2).
fn on_path(program: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("a PATH"))
        .map(|directory| directory.join(program))
        .find(|path| path.is_file())
        .expect("the program is on PATH")
}

/// The process environment's `PATH` is shared by every flow in this binary,
/// so a flow that changes it holds this while it does.
static PATH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Puts `PATH` back when its flow ends, pass or panic.
struct RestorePath(Option<std::ffi::OsString>);

impl Drop for RestorePath {
    fn drop(&mut self) {
        // SAFETY: the flow holds PATH_LOCK until after this guard drops, so
        // no other flow reads or writes PATH meanwhile.
        unsafe {
            match self.0.take() {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
    }
}

/// `runtime.list` reports an interpreter only while the record marks the
/// runtime installed under the plan in effect (§2.4, §5), the same test that
/// skips a download. Nothing is acquired here: the record and the directories
/// it names are written by the test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_list_reports_a_path_only_while_its_record_matches_the_plan() {
    let home = PathBuf::from("/tmp").join(format!("drm-path-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(home.join("runtimes")).unwrap();
    let defaults = home.join("default-plugins.json");
    std::fs::write(
        &defaults,
        r#"{"runtimes": {"node": {"source": "download", "auto": false},
                         "python": {"source": "download", "auto": false}}}"#,
    )
    .unwrap();
    let runtimes = Arc::new(
        Runtimes::load(
            &home,
            &dim_app_server::plugin_manager::Defaults::read(&defaults)
                .unwrap()
                .runtimes,
        )
        .unwrap(),
    );
    let node = runtimes.plan(Kind::Node).clone();
    std::fs::create_dir_all(&node.packages).unwrap();
    std::fs::create_dir_all(archive_directory(&node.interpreter)).unwrap();
    let packages = node.packages.file_name().unwrap().to_str().unwrap();
    std::fs::write(
        home.join("runtimes/runtimes.json"),
        json!({"node": {"source": "download", "version": &node.version, "packages": packages}})
            .to_string(),
    )
    .unwrap();
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            target_home: Some(home.clone()),
            session_typescript: Some(SessionTypeScriptConfig::acquired(&home, runtimes, vec![])),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;

    let listed = client.rpc("runtime.list", json!({})).await;
    assert_ok(&listed);
    assert_eq!(
        listed["result"][0]["path"],
        node.interpreter.to_str().unwrap(),
        "the installed runtime reports the interpreter it runs with: {listed}"
    );
    assert!(
        listed["result"][1]["path"].is_null(),
        "a runtime nothing resolved reports no path: {listed}"
    );

    std::fs::write(
        home.join("runtimes/runtimes.json"),
        json!({"node": {"source": "system", "version": node.version, "packages": packages}})
            .to_string(),
    )
    .unwrap();
    let listed = client.rpc("runtime.list", json!({})).await;
    assert_ok(&listed);
    assert_eq!(listed["result"][0]["version"], node.version, "{listed}");
    assert!(
        listed["result"][0]["path"].is_null(),
        "a record that is not this plan's reports no path: {listed}"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// A relative `PATH` entry (§5): `which` resolves it against the daemon's
/// working directory, so `runtime.list` reports the absolute interpreter path
/// that a process started from another working directory can launch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_list_reports_an_absolute_path_for_a_relative_path_entry() {
    let _path = PATH_LOCK.lock().await;
    let _restore = RestorePath(std::env::var_os("PATH"));
    let home = std::env::temp_dir().join(format!("drm-relative-{}", uuid::Uuid::new_v4()));
    let fixture = home.join("system-node");
    let bin = fixture.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let node = bin.join("node");
    std::fs::write(
        &node,
        "#!/bin/sh\n[ \"$1\" = --version ] && echo v22.19.0\nexit 0\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir_all(fixture.join("lib/node_modules/npm/bin")).unwrap();
    std::fs::write(fixture.join("lib/node_modules/npm/bin/npm-cli.js"), "").unwrap();

    // The interpreter's directory as a relative entry, built from the current
    // directory so no flow has to change it: one `..` per component of the
    // current directory reaches the root, then the directory's path follows.
    let cwd = std::env::current_dir().unwrap();
    let absolute = bin.canonicalize().unwrap();
    let ups = cwd
        .components()
        .filter(|component| matches!(component, std::path::Component::Normal(_)))
        .count();
    let relative = PathBuf::from("../".repeat(ups)).join(absolute.strip_prefix("/").unwrap());
    assert!(
        !relative.is_absolute(),
        "the entry is relative: {relative:?}"
    );
    let previous = std::env::var_os("PATH").unwrap();
    let joined = std::env::join_paths(
        std::iter::once(relative.clone()).chain(std::env::split_paths(&previous)),
    )
    .unwrap();
    // SAFETY: the flow holds PATH_LOCK, and RestorePath puts PATH back.
    unsafe { std::env::set_var("PATH", &joined) };

    let defaults = home.join("default-plugins.json");
    std::fs::write(
        &defaults,
        r#"{"runtimes": {"node": {"source": "system", "auto": false},
                         "python": {"source": "download", "auto": false}}}"#,
    )
    .unwrap();
    let runtimes = Arc::new(
        Runtimes::load(
            &home,
            &dim_app_server::plugin_manager::Defaults::read(&defaults)
                .unwrap()
                .runtimes,
        )
        .unwrap(),
    );
    std::fs::create_dir_all(&runtimes.plan(Kind::Node).packages).unwrap();
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            target_home: Some(home.clone()),
            session_typescript: Some(SessionTypeScriptConfig::acquired(&home, runtimes, vec![])),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let install = client.rpc("runtime.install", json!({"name": "node"})).await;
    assert_ok(&install);

    let entry = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let list = client.rpc("runtime.list", json!({})).await;
            assert_ok(&list);
            if list["result"][0]["path"].is_string() {
                return list["result"][0].clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("runtime.list reported no path");

    let path = entry["path"].as_str().unwrap();
    assert!(
        Path::new(path).is_absolute(),
        "the reported interpreter path is absolute: {entry}"
    );
    assert_eq!(
        Path::new(path).canonicalize().unwrap(),
        node.canonicalize().unwrap(),
        "the reported path names the fixture: {entry}"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires node 22.19+ and python3 3.12+ with pip on PATH, and the npm and PyPI registries"]
async fn node_runtime_manager_starts_rows_once_their_runtime_is_acquired() {
    let home = PathBuf::from("/tmp").join(format!("drm-{}", uuid::Uuid::new_v4()));
    let plugins = home.join("sources");
    std::fs::create_dir_all(plugins.join("probe-py")).unwrap();
    let defaults = home.join("default-plugins.json");
    std::fs::write(
        &defaults,
        r#"{"runtimes": {"node": {"source": "system", "auto": true},
                         "python": {"source": "system", "auto": false}}}"#,
    )
    .unwrap();
    let node_marker = home.join("node-started");
    std::fs::write(
        plugins.join("probe.mjs"),
        format!(
            "import {{ writeFileSync }} from 'node:fs'\n\
             export default {{ name: 'probe', apply() {{ writeFileSync({:?}, String(process.pid)) }} }}\n",
            node_marker
        ),
    )
    .unwrap();
    let python_marker = home.join("python-started");
    std::fs::write(
        plugins.join("probe-py/entry.py"),
        format!(
            "import pathlib\n\ninject = []\nprovides = {{}}\n\n\n\
             async def apply(ctx, config):\n    pathlib.Path({:?}).write_text('yes')\n",
            python_marker.to_str().unwrap()
        ),
    )
    .unwrap();
    std::fs::create_dir_all(home.join("plugins/.python")).unwrap();
    std::os::unix::fs::symlink(
        plugins.join("probe-py"),
        home.join("plugins/.python/probe_py"),
    )
    .unwrap();
    let rows = vec![
        json!({"id": "probe", "name": plugins.join("probe.mjs"), "provides": {}}),
        json!({"id": "probe-py", "name": "py:probe_py.entry", "provides": {}}),
    ];
    let runtimes = Arc::new(
        Runtimes::load(
            &home,
            &dim_app_server::plugin_manager::Defaults::read(&defaults)
                .unwrap()
                .runtimes,
        )
        .unwrap(),
    );
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            target_home: Some(home.clone()),
            session_typescript: Some(SessionTypeScriptConfig::acquired(&home, runtimes, rows)),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    appears(&node_marker).await;
    let list = client.rpc("runtime.list", json!({})).await;
    assert_ok(&list);
    assert_eq!(list["result"][0]["name"], "node");
    assert_eq!(list["result"][1]["status"], "off", "{list}");
    assert!(
        list["result"][1]["path"].is_null(),
        "a runtime that is off resolved nothing: {list}"
    );
    assert!(!python_marker.exists());
    let ready = status(&mut client, 0, "ready").await;
    assert_eq!(
        ready["path"],
        on_path("node").to_str().unwrap(),
        "the entry names the checked interpreter: {ready}"
    );

    let pid = std::fs::read_to_string(&node_marker).unwrap();
    std::fs::remove_file(&node_marker).unwrap();
    assert!(
        std::process::Command::new("kill")
            .args(["-KILL", &pid])
            .status()
            .unwrap()
            .success()
    );
    let down = status(&mut client, 0, "down").await;
    assert!(
        down["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()),
        "{down}"
    );
    assert!(!node_marker.exists());
    assert_ok(&client.rpc("runtime.install", json!({"name": "node"})).await);
    appears(&node_marker).await;
    assert_ne!(std::fs::read_to_string(&node_marker).unwrap(), pid);
    status(&mut client, 0, "ready").await;

    let install = client
        .rpc("runtime.install", json!({"name": "python"}))
        .await;
    assert_ok(&install);
    appears(&python_marker).await;
    status(&mut client, 1, "ready").await;
    let records: Value =
        serde_json::from_slice(&std::fs::read(home.join("runtimes/runtimes.json")).unwrap())
            .unwrap();
    assert_eq!(records["node"]["source"], "system");
    assert_eq!(records["python"]["source"], "system");
    let checked = std::process::Command::new("node")
        .arg("--version")
        .output()
        .unwrap();
    let checked = String::from_utf8(checked.stdout).unwrap();
    assert_eq!(
        records["node"]["version"],
        checked.trim().trim_start_matches('v')
    );
    let list = client.rpc("runtime.list", json!({})).await;
    assert_eq!(list["result"][0]["version"], records["node"]["version"]);
    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    std::fs::remove_file(&python_marker).unwrap();
    let runtimes = Arc::new(
        Runtimes::load(
            &home,
            &dim_app_server::plugin_manager::Defaults::read(&defaults)
                .unwrap()
                .runtimes,
        )
        .unwrap(),
    );
    let rows = vec![json!({"id": "probe-py", "name": "py:probe_py.entry", "provides": {}})];
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            target_home: Some(home.clone()),
            session_typescript: Some(SessionTypeScriptConfig::acquired(&home, runtimes, rows)),
            ..Default::default()
        },
    )
    .await;
    appears(&python_marker).await;
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// The configuration is read when an action applies it, not at startup: a file
/// written after the App Server started reaches `runtime.install`, and one that
/// does not read fails that call and leaves the running runtime as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires node 22.19+ and the npm registry"]
async fn node_runtime_manager_reads_the_configuration_when_it_applies_it() {
    let home = PathBuf::from("/tmp").join(format!("drm-config-{}", uuid::Uuid::new_v4()));
    let plugins = home.join("sources");
    std::fs::create_dir_all(&plugins).unwrap();
    let defaults = home.join("default-plugins.json");
    std::fs::write(
        &defaults,
        r#"{"runtimes": {"node": {"source": "system", "auto": true},
                         "python": {"source": "system", "auto": false}}}"#,
    )
    .unwrap();
    let marker = home.join("node-started");
    std::fs::write(
        plugins.join("probe.mjs"),
        format!(
            "import {{ writeFileSync }} from 'node:fs'\n\
             export default {{ name: 'probe', apply() {{ writeFileSync({:?}, String(process.pid)) }} }}\n",
            marker
        ),
    )
    .unwrap();
    let rows = vec![json!({"id": "probe", "name": plugins.join("probe.mjs"), "provides": {}})];
    let runtimes = Arc::new(
        Runtimes::load(
            &home,
            &dim_app_server::plugin_manager::Defaults::read(&defaults)
                .unwrap()
                .runtimes,
        )
        .unwrap(),
    );
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            target_home: Some(home.clone()),
            session_typescript: Some(SessionTypeScriptConfig::acquired(&home, runtimes, rows)),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    appears(&marker).await;
    let ready = status(&mut client, 0, "ready").await;

    std::fs::write(
        home.join("runtimes/config.json"),
        r#"{"node": {"source": "elsewhere"}}"#,
    )
    .unwrap();
    let refused = client.rpc("runtime.install", json!({"name": "node"})).await;
    assert_eq!(
        refused["error"]["data"]["appError"], "runtime_configuration",
        "{refused}"
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("elsewhere")),
        "{refused}"
    );
    assert_eq!(
        status(&mut client, 0, "ready").await,
        ready,
        "the runtime is as it was"
    );

    std::fs::write(
        home.join("runtimes/config.json"),
        r#"{"node": {"source": "system"}}"#,
    )
    .unwrap();
    let applied = client.rpc("runtime.install", json!({"name": "node"})).await;
    assert_ok(&applied);
    assert_eq!(
        status(&mut client, 0, "ready").await,
        ready,
        "the same process keeps running"
    );

    let configured = client
        .rpc(
            "runtime.configure",
            json!({"name": "node", "source": "system", "auto": true}),
        )
        .await;
    assert_ok(&configured);
    assert_eq!(configured["result"]["name"], "node", "{configured}");
    assert_eq!(configured["result"]["auto"], true, "{configured}");
    let file: Value =
        serde_json::from_slice(&std::fs::read(home.join("runtimes/config.json")).unwrap()).unwrap();
    assert_eq!(file["node"]["source"], "system", "{file}");
    assert_eq!(file["node"]["auto"], true, "{file}");
    assert_eq!(
        status(&mut client, 0, "ready").await,
        ready,
        "configuring what is already in effect changes nothing"
    );

    let listed = client.rpc("runtime.list", json!({})).await;
    assert_eq!(listed["result"][0]["auto"], true, "{listed}");
    assert_eq!(listed["result"][1]["auto"], false, "{listed}");
    let untouched = listed["result"][1].clone();

    let configured = client
        .rpc("runtime.configure", json!({"name": "node", "auto": false}))
        .await;
    assert_ok(&configured);
    assert_eq!(configured["result"]["auto"], false, "{configured}");
    let listed = client.rpc("runtime.list", json!({})).await;
    assert_eq!(
        listed["result"][1], untouched,
        "applying one runtime leaves the other as it was: {listed}"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// Switching a running runtime's source replaces its launch command and
/// starts the new interpreter, and leaves the other runtime alone: the case a
/// configuration change exists for. It downloads the archive, so it runs only
/// with the other ignored flows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "downloads the Node archive and installs its packages"]
async fn node_runtime_manager_switches_a_running_runtime_to_another_source() {
    let home = PathBuf::from("/tmp").join(format!("drm-switch-{}", uuid::Uuid::new_v4()));
    let plugins = home.join("sources");
    std::fs::create_dir_all(&plugins).unwrap();
    let defaults = home.join("default-plugins.json");
    std::fs::write(
        &defaults,
        r#"{"runtimes": {"node": {"source": "system", "auto": true},
                         "python": {"source": "system", "auto": false}}}"#,
    )
    .unwrap();
    let marker = home.join("node-started");
    std::fs::write(
        plugins.join("probe.mjs"),
        format!(
            "import {{ writeFileSync }} from 'node:fs'\n\
             export default {{ name: 'probe', apply() {{ writeFileSync({:?}, String(process.pid)) }} }}\n",
            marker
        ),
    )
    .unwrap();
    let rows = vec![json!({"id": "probe", "name": plugins.join("probe.mjs"), "provides": {}})];
    let runtimes = Arc::new(
        Runtimes::load(
            &home,
            &dim_app_server::plugin_manager::Defaults::read(&defaults)
                .unwrap()
                .runtimes,
        )
        .unwrap(),
    );
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            target_home: Some(home.clone()),
            session_typescript: Some(SessionTypeScriptConfig::acquired(&home, runtimes, rows)),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    appears(&marker).await;
    status(&mut client, 0, "ready").await;
    let before = std::fs::read_to_string(&marker).unwrap();
    let python = client.rpc("runtime.list", json!({})).await["result"][1].clone();

    let switched = client
        .rpc(
            "runtime.configure",
            json!({"name": "node", "source": "download"}),
        )
        .await;
    assert_ok(&switched);
    assert!(
        switched["result"]["path"].is_null(),
        "the record of the source it left reports no path: {switched}"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    let after = loop {
        let seen = std::fs::read_to_string(&marker).unwrap_or_default();
        if seen != before {
            break seen;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the new interpreter never started a new process: {seen}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_ne!(after, before, "the new interpreter started a new process");
    let ready = status(&mut client, 0, "ready").await;
    assert_eq!(ready["source"], "download", "{ready}");
    assert_eq!(ready["version"], "24.21.0", "{ready}");
    let path = Path::new(ready["path"].as_str().expect("the resolved interpreter"));
    assert!(path.is_absolute(), "{ready}");
    assert_eq!(path.file_name().unwrap(), "node", "{ready}");
    assert!(
        path.starts_with(home.join("runtimes"))
            && path.to_string_lossy().contains("node-v24.21.0-"),
        "the acquired archive's interpreter: {ready}"
    );
    let listed = client.rpc("runtime.list", json!({})).await;
    assert_eq!(
        listed["result"][1], python,
        "the other runtime is untouched: {listed}"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

/// A `download` Python marks its interpreter as externally managed inside its
/// own archive directory (RuntimeManager §7), so pip never installs into a
/// runtime Dim replaces. It downloads the archive and installs its packages,
/// so it runs only with the other ignored flows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "downloads the Python archive and installs its packages"]
async fn python_runtime_manager_marks_the_downloaded_interpreter_as_externally_managed() {
    let home = std::env::temp_dir().join(format!("drm-marker-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let defaults = home.join("default-plugins.json");
    std::fs::write(
        &defaults,
        r#"{"runtimes": {"node": {"source": "system", "auto": false},
                         "python": {"source": "download", "auto": false}}}"#,
    )
    .unwrap();
    let runtimes = Arc::new(
        Runtimes::load(
            &home,
            &dim_app_server::plugin_manager::Defaults::read(&defaults)
                .unwrap()
                .runtimes,
        )
        .unwrap(),
    );
    let python = runtimes.plan(Kind::Python).clone();
    let server = launch(
        Arc::new(MockTools),
        ServerConfig {
            target_home: Some(home.clone()),
            session_typescript: Some(SessionTypeScriptConfig::acquired(&home, runtimes, vec![])),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    assert_ok(
        &client
            .rpc("runtime.install", json!({"name": "python"}))
            .await,
    );
    let ready = status(&mut client, 1, "ready").await;
    assert_eq!(ready["source"], "download", "{ready}");
    let archive = archive_directory(&python.interpreter);
    let stdlib = if cfg!(windows) {
        archive.join("Lib")
    } else {
        archive.join("lib").join(format!(
            "python3.{}",
            python.version.split('.').nth(1).expect("a pinned version")
        ))
    };
    let marker = stdlib.join("EXTERNALLY-MANAGED");
    let text = std::fs::read_to_string(&marker)
        .unwrap_or_else(|error| panic!("{}: {error}", marker.display()));
    assert!(
        text.starts_with("[externally-managed]") && text.contains("Error="),
        "{text}"
    );
    assert!(stdlib.is_dir(), "the marker is in the interpreter's stdlib");
    let output = tokio::process::Command::new(&python.interpreter)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('stdlib'))",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reported = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    assert_eq!(
        std::fs::canonicalize(reported).unwrap(),
        std::fs::canonicalize(&stdlib).unwrap(),
        "the marker's directory is the interpreter's sysconfig stdlib"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
