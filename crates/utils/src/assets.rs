use directories::ProjectDirs;
use rust_embed::RustEmbed;

const PROJECT_ROOT: &str = env!("CARGO_MANIFEST_DIR");

pub fn asset_dir() -> std::path::PathBuf {
    let path = if let Some(override_path) = std::env::var_os("VK_ASSET_DIR_OVERRIDE") {
        std::path::PathBuf::from(override_path)
    } else if cfg!(debug_assertions) {
        std::path::PathBuf::from(PROJECT_ROOT).join("../../dev_assets")
    } else {
        prod_asset_dir_path()
    };

    // Ensure the directory exists
    if !path.exists() {
        std::fs::create_dir_all(&path).expect("Failed to create asset directory");
    }

    path
    // ✔ macOS → ~/Library/Application Support/MyApp
    // ✔ Linux → ~/.local/share/myapp   (respects XDG_DATA_HOME)
    // ✔ Windows → %APPDATA%\Example\MyApp
}

pub fn prod_asset_dir_path() -> std::path::PathBuf {
    ProjectDirs::from("ai", "bloop", "vibe-kanban")
        .expect("OS didn't give us a home directory")
        .data_dir()
        .to_path_buf()
}

pub fn config_path() -> std::path::PathBuf {
    asset_dir().join("config.json")
}

pub fn profiles_path() -> std::path::PathBuf {
    asset_dir().join("profiles.json")
}

pub fn credentials_path() -> std::path::PathBuf {
    asset_dir().join("credentials.json")
}

pub fn trusted_keys_path() -> std::path::PathBuf {
    asset_dir().join("trusted_ed25519_public_keys.json")
}

pub fn server_signing_key_path() -> std::path::PathBuf {
    asset_dir().join("server_ed25519_signing_key")
}

pub fn relay_host_credentials_path() -> std::path::PathBuf {
    asset_dir().join("relay_host_credentials.json")
}

#[derive(RustEmbed)]
#[folder = "../../assets/sounds"]
pub struct SoundAssets;

#[derive(RustEmbed)]
#[folder = "../../assets/scripts"]
pub struct ScriptAssets;

#[cfg(test)]
mod tests {
    use std::{
        sync::{LazyLock, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::asset_dir;

    static ASSET_DIR_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn asset_dir_uses_override_env_when_present() {
        let _lock = ASSET_DIR_ENV_LOCK.lock().unwrap();
        let tempdir = std::env::temp_dir().join(format!(
            "utils-asset-dir-override-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tempdir).unwrap();
        let previous = std::env::var_os("VK_ASSET_DIR_OVERRIDE");

        unsafe {
            std::env::set_var("VK_ASSET_DIR_OVERRIDE", &tempdir);
        }

        let path = asset_dir();

        match previous {
            Some(value) => unsafe {
                std::env::set_var("VK_ASSET_DIR_OVERRIDE", value);
            },
            None => unsafe {
                std::env::remove_var("VK_ASSET_DIR_OVERRIDE");
            },
        }

        let _ = std::fs::remove_dir_all(&tempdir);

        assert_eq!(path, tempdir);
    }
}
