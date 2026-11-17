use hashing::HashingWriter;
use sha2::{Digest, Sha256, Sha512};
use std::io::Write;

#[test]
fn test_streaming_sha256_verify_success_and_mismatch() {
    let mut sink = Vec::new();
    let mut hw = HashingWriter::new(&mut sink, "sha256").unwrap();
    hw.write_all(b"hello ").unwrap();
    hw.write_all(b"world").unwrap();
    hw.flush().unwrap();

    let expected = format!("{:x}", Sha256::digest(b"hello world"));
    assert!(hw.verify_digest("my_tool", "sha256", &expected).is_ok());
    assert_eq!(sink, b"hello world");

    let mut sink2 = Vec::new();
    let mut hw2 = HashingWriter::new(&mut sink2, "sha256").unwrap();
    hw2.write_all(b"hello world").unwrap();
    let err = hw2.verify_digest("my_tool", "sha256", "deadbeef").unwrap_err();
    assert_eq!(
        err,
        format!(
            "SHA256 verification failed for my_tool.\nExpected: deadbeef\nActual:   {}",
            expected
        )
    );
}

#[test]
fn test_streaming_sha512_verify_success_and_mismatch() {
    let mut sink = Vec::new();
    let mut hw = HashingWriter::new(&mut sink, "sha512").unwrap();
    hw.write_all(b"part1-").unwrap();
    hw.write_all(b"part2").unwrap();
    hw.flush().unwrap();

    let expected = format!("{:x}", Sha512::digest(b"part1-part2"));
    assert!(hw.verify_digest("sha512_tool", "sha512", &expected).is_ok());
    assert_eq!(sink, b"part1-part2");

    let mut sink2 = Vec::new();
    let mut hw2 = HashingWriter::new(&mut sink2, "sha512").unwrap();
    hw2.write_all(b"part1-part2").unwrap();
    let err = hw2.verify_digest("sha512_tool", "sha512", "bad512").unwrap_err();
    assert_eq!(
        err,
        format!(
            "SHA512 verification failed for sha512_tool.\nExpected: bad512\nActual:   {}",
            expected
        )
    );
}

#[test]
fn test_unsupported_digest_algorithm_rejected() {
    let mut sink: Vec<u8> = Vec::new();
    let err = HashingWriter::new(&mut sink, "md5").err().unwrap();
    assert_eq!(err, "Unsupported digest algorithm: md5");
}
