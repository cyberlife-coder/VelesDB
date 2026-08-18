use super::*;

#[test]
fn test_load_tls_config_missing_cert_file() {
    match load_tls_config("/nonexistent/cert.pem", "/nonexistent/key.pem") {
        Err(e) => assert!(e.to_string().contains("cert")),
        Ok(_) => panic!("should fail for missing files"),
    }
}

#[test]
fn test_load_tls_config_empty_cert_file() {
    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::write(&cert_path, "").unwrap();
    std::fs::write(&key_path, "").unwrap();

    match load_tls_config(&cert_path.to_string_lossy(), &key_path.to_string_lossy()) {
        Err(e) => assert!(e.to_string().contains("no certificates")),
        Ok(_) => panic!("should fail for empty cert"),
    }
}

#[test]
fn test_load_tls_config_invalid_pem() {
    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::write(&cert_path, "not a real cert").unwrap();
    std::fs::write(&key_path, "not a real key").unwrap();

    assert!(
        load_tls_config(&cert_path.to_string_lossy(), &key_path.to_string_lossy(),).is_err(),
        "should fail for invalid PEM"
    );
}
