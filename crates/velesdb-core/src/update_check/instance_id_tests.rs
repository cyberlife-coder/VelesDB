use super::*;
use std::collections::HashMap;
use tempfile::TempDir;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let table: HashMap<String, OsString> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
        .collect();
    move |key| table.get(key).cloned()
}

fn is_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A directory nothing can be created in, on every platform and for every
/// user including root: its parent is a regular file.
fn unusable_dir(tmp: &TempDir) -> PathBuf {
    let file = tmp.path().join("not-a-dir");
    std::fs::write(&file, b"x").expect("write blocker file");
    file.join("velesdb")
}

#[test]
fn test_config_dir_per_platform() {
    let home = [("HOME", "/home/u")];
    assert_eq!(
        user_config_dir(Platform::Linux, env(&home)),
        Some(PathBuf::from("/home/u/.config/velesdb"))
    );
    assert_eq!(
        user_config_dir(
            Platform::Linux,
            env(&[("XDG_CONFIG_HOME", "/x/cfg"), ("HOME", "/home/u")])
        ),
        Some(PathBuf::from("/x/cfg/velesdb"))
    );
    assert_eq!(
        user_config_dir(Platform::MacOs, env(&[("HOME", "/Users/u")])),
        Some(PathBuf::from(
            "/Users/u/Library/Application Support/velesdb"
        ))
    );
    assert_eq!(
        user_config_dir(
            Platform::Windows,
            env(&[("APPDATA", "C:\\Users\\u\\AppData\\Roaming")])
        ),
        Some(PathBuf::from("C:\\Users\\u\\AppData\\Roaming").join("velesdb"))
    );
}

#[test]
fn test_config_dir_absent_when_environment_gives_none() {
    assert_eq!(user_config_dir(Platform::Linux, env(&[])), None);
    assert_eq!(user_config_dir(Platform::MacOs, env(&[])), None);
    assert_eq!(
        user_config_dir(Platform::Windows, env(&[("HOME", "/h")])),
        None
    );
    // A relative XDG_CONFIG_HOME is ignored per the XDG spec, as is an empty one.
    assert_eq!(
        user_config_dir(
            Platform::Linux,
            env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/h")])
        ),
        Some(PathBuf::from("/h/.config/velesdb"))
    );
    assert_eq!(
        user_config_dir(
            Platform::Linux,
            env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")])
        ),
        Some(PathBuf::from("/h/.config/velesdb"))
    );
}

#[test]
fn test_candidates_put_user_config_before_data_dir() {
    let dirs = candidate_dirs(
        Platform::Linux,
        env(&[("HOME", "/h")]),
        Some(Path::new("/data")),
    );
    assert_eq!(
        dirs,
        vec![PathBuf::from("/h/.config/velesdb"), PathBuf::from("/data")]
    );
    let no_data = candidate_dirs(Platform::Linux, env(&[("HOME", "/h")]), None);
    assert_eq!(no_data, vec![PathBuf::from("/h/.config/velesdb")]);
}

#[test]
fn test_install_id_persists_across_calls() {
    let tmp = TempDir::new().expect("tempdir");
    let dirs = vec![tmp.path().join("cfg")];
    let first = resolve_install_id(&dirs);
    let second = resolve_install_id(&dirs);
    assert!(is_id(&first), "id {first:?} is 32 lowercase hex");
    assert_eq!(first, second, "second call reads the file the first wrote");
    let on_disk = std::fs::read_to_string(dirs[0].join(FILE_NAME)).expect("id file written");
    assert_eq!(on_disk.trim(), first);
}

#[test]
fn test_install_id_differs_between_installs() {
    let a = TempDir::new().expect("tempdir a");
    let b = TempDir::new().expect("tempdir b");
    assert_ne!(
        resolve_install_id(&[a.path().to_path_buf()]),
        resolve_install_id(&[b.path().to_path_buf()])
    );
}

#[test]
fn test_corrupt_id_file_is_regenerated_then_stable() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join(FILE_NAME);
    for garbage in ["", "not an id", "ABCDEF0123456789ABCDEF0123456789", "abc"] {
        std::fs::write(&path, garbage).expect("write garbage");
        let id = resolve_install_id(&[tmp.path().to_path_buf()]);
        assert!(is_id(&id), "regenerated id {id:?} after {garbage:?}");
        assert_eq!(resolve_install_id(&[tmp.path().to_path_buf()]), id);
    }
}

#[test]
fn test_unusable_dir_falls_through_to_next_candidate() {
    let tmp = TempDir::new().expect("tempdir");
    let good = tmp.path().join("data");
    let id = resolve_install_id(&[unusable_dir(&tmp), good.clone()]);
    assert!(is_id(&id));
    assert_eq!(
        std::fs::read_to_string(good.join(FILE_NAME))
            .expect("fallback file")
            .trim(),
        id
    );
}

#[test]
fn test_nothing_writable_degrades_to_ephemeral_id() {
    let tmp = TempDir::new().expect("tempdir");
    let dirs = vec![unusable_dir(&tmp)];
    let first = resolve_install_id(&dirs);
    let second = resolve_install_id(&dirs);
    assert!(is_id(&first) && is_id(&second));
    assert_ne!(first, second, "nothing persisted, so each call is a new id");
    assert!(
        is_id(&resolve_install_id(&[])),
        "no candidate at all still yields an id"
    );
}

#[cfg(unix)]
#[test]
fn test_read_only_dir_degrades_to_ephemeral_id() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("ro");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    // Root ignores the mode bits; the case is then not a read-only directory.
    let writable = std::fs::write(dir.join("probe"), b"").is_ok();
    let id = resolve_install_id(std::slice::from_ref(&dir));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("restore");
    assert!(is_id(&id));
    if !writable {
        assert!(
            !dir.join(FILE_NAME).exists(),
            "nothing written to a read-only dir"
        );
    }
}

#[test]
fn test_hash_is_64_lowercase_hex_and_versioned() {
    let hash = hash_install_id("0123456789abcdef0123456789abcdef");
    assert_eq!(hash.len(), 64);
    assert!(hash
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(hash, hash_install_id("0123456789abcdef0123456789abcdef"));
    assert_ne!(hash, hash_install_id("fedcba9876543210fedcba9876543210"));
    // Fixed vector: sha256("velesdb-install-v2:" + id). Changing the prefix
    // re-identifies every install, so it must be a deliberate edit of this line.
    assert_eq!(
        hash,
        "b0a080e4c1d9a83f8cb2f15086b993de06aee26f0ce3ea3c018870161e025c4d"
    );
}
