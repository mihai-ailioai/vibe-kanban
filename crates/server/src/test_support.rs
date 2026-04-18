use std::{
    ffi::OsString,
    sync::{LazyLock, Mutex, MutexGuard},
};

use deployment::Deployment;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::DeploymentImpl;

static ASSET_DIR_OVERRIDE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub struct TestAssetDirGuard {
    previous: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
    _tempdir: TempDir,
}

impl Drop for TestAssetDirGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe {
                std::env::set_var("VK_ASSET_DIR_OVERRIDE", value);
            },
            None => unsafe {
                std::env::remove_var("VK_ASSET_DIR_OVERRIDE");
            },
        }
    }
}

pub fn isolated_asset_dir_guard() -> TestAssetDirGuard {
    let lock = ASSET_DIR_OVERRIDE_LOCK.lock().unwrap();
    let tempdir = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("VK_ASSET_DIR_OVERRIDE");

    unsafe {
        std::env::set_var("VK_ASSET_DIR_OVERRIDE", tempdir.path());
    }

    TestAssetDirGuard {
        previous,
        _lock: lock,
        _tempdir: tempdir,
    }
}

pub async fn new_test_deployment() -> (TestAssetDirGuard, DeploymentImpl) {
    let guard = isolated_asset_dir_guard();
    let deployment = <DeploymentImpl as Deployment>::new(CancellationToken::new())
        .await
        .unwrap();
    (guard, deployment)
}
