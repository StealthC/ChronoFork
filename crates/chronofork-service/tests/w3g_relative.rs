#![allow(clippy::expect_used)]

//! W3g/D1 regression: a durable start with a **relative** identity path.
//!
//! `Path::parent` of a bare relative name is `Some("")`, not `None`. The directory sync
//! opened `""` and the whole start failed with `Storage`, so `--token-file tokens.txt` (and
//! a relative store root) could not start a durable daemon. The fix normalizes an empty
//! parent to the current directory.
//!
//! This file deliberately holds **one** test: it changes the process working directory, and
//! a second test in the same binary could observe it. Other test binaries are separate
//! processes, so they are unaffected.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chronofork_service::{AppService, ServiceConfig, ServiceLimits};
use chronofork_test_backend::SyntheticBackendFactory;

/// Removes the working directory it owns when dropped.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("chronofork-w3g-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn durable(root: &Path) -> Result<AppService, chronofork_service::ServiceError> {
    AppService::new_durable(
        ServiceConfig {
            factory: Arc::new(SyntheticBackendFactory::default()),
            storage_root: None,
            artifacts_root: None,
            limits: ServiceLimits::default(),
        },
        root,
    )
}

#[test]
fn a_relative_identity_path_starts_and_increments() {
    let directory = TempDir::new("relative");
    let original = std::env::current_dir().expect("current directory");
    std::env::set_current_dir(&directory.path).expect("enter the temporary directory");

    // 1. A bare relative name: parent is "", which must mean the current directory.
    let first = durable(Path::new("store")).expect("a bare relative identity starts");
    assert_eq!(first.process_epoch(), 1);
    assert_eq!(
        std::fs::read_to_string("store.chronofork-epoch").expect("relative identity file"),
        "1\n"
    );

    // 2. The explicit `./` form.
    let second = durable(Path::new("./store")).expect("the `./name` identity starts");
    assert_eq!(
        second.process_epoch(),
        2,
        "the same file is incremented through the `./` form"
    );

    // 3. An absolute path keeps working on the same file.
    let absolute = directory.path.join("store");
    let third = durable(&absolute).expect("the absolute identity starts");
    assert_eq!(third.process_epoch(), 3);

    // No temporary or lock file is left behind after a successful publish.
    assert!(!directory.path.join("store.chronofork-epoch.tmp").exists());
    assert!(!directory.path.join("store.chronofork-epoch.lock").exists());

    std::env::set_current_dir(original).expect("restore the working directory");
}
