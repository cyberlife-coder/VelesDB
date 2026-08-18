use super::update_os_str;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

#[test]
fn path_hashing_distinguishes_non_unicode_windows_names() {
    let first = OsString::from_wide(&[0xd800]);
    let second = OsString::from_wide(&[0xd801]);
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());

    let mut first_hash = Sha256::new();
    update_os_str(&mut first_hash, &first);
    let mut second_hash = Sha256::new();
    update_os_str(&mut second_hash, &second);

    assert_ne!(first_hash.finalize(), second_hash.finalize());
}
