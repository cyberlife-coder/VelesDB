use super::*;

#[test]
fn generates_a_ca_and_leaf_cert_on_first_use() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let material = ensure_tls_material(dir.path()).expect("generate TLS material");

    assert!(!material.cert_pem.is_empty());
    assert!(!material.key_pem.is_empty());
    assert!(material.ca_cert_path.exists());
    assert!(dir.path().join(CA_KEY_FILE).exists());
}

#[test]
fn reuses_the_same_ca_across_calls() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let first = ensure_tls_material(dir.path()).expect("first generation");
    let ca_pem_first =
        std::fs::read_to_string(&first.ca_cert_path).expect("read CA cert after first run");

    let second = ensure_tls_material(dir.path()).expect("second generation");
    let ca_pem_second =
        std::fs::read_to_string(&second.ca_cert_path).expect("read CA cert after second run");

    assert_eq!(
        ca_pem_first, ca_pem_second,
        "the CA must never be regenerated once it exists on disk"
    );
}

#[test]
fn leaf_cert_is_re_issued_on_every_call() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let first = ensure_tls_material(dir.path()).expect("first generation");
    let second = ensure_tls_material(dir.path()).expect("second generation");

    assert_ne!(
        first.cert_pem, second.cert_pem,
        "the leaf certificate is expected to be freshly re-issued (renewed) on every start"
    );
}

#[test]
fn builds_a_tls_acceptor_from_generated_material() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let material = ensure_tls_material(dir.path()).expect("generate TLS material");
    tls_acceptor_from_material(&material).expect("build TLS acceptor from valid material");
}

#[cfg(unix)]
#[test]
fn private_key_files_are_not_group_or_world_readable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("create scratch dir");
    ensure_tls_material(dir.path()).expect("generate TLS material");

    for key_file in [CA_KEY_FILE, LEAF_KEY_FILE] {
        let path = dir.path().join(key_file);
        let mode = std::fs::metadata(&path)
            .unwrap_or_else(|_| panic!("stat {key_file}"))
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "{key_file} must not be group/world readable or writable (mode {mode:o})"
        );
    }
}
