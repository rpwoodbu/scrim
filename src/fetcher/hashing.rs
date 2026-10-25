use sha2::{Digest, Sha256, Sha512};
use std::io::{self, Write};

pub enum HasherInstance {
    Sha256(Sha256),
    Sha512(Sha512),
}

impl HasherInstance {
    pub fn new(algorithm: &str) -> Result<Self, String> {
        match algorithm {
            "sha256" => Ok(HasherInstance::Sha256(Sha256::new())),
            "sha512" => Ok(HasherInstance::Sha512(Sha512::new())),
            _ => Err(format!("Unsupported digest algorithm: {}", algorithm)),
        }
    }

    pub fn update(&mut self, buf: &[u8]) {
        match self {
            HasherInstance::Sha256(h) => h.update(buf),
            HasherInstance::Sha512(h) => h.update(buf),
        }
    }

    pub fn finalize_hex(self) -> String {
        match self {
            HasherInstance::Sha256(h) => format!("{:x}", h.finalize()),
            HasherInstance::Sha512(h) => format!("{:x}", h.finalize()),
        }
    }
}

pub struct HashingWriter<W> {
    writer: W,
    hasher: HasherInstance,
}

impl<W> HashingWriter<W> {
    pub fn new(writer: W, algorithm: &str) -> Result<Self, String> {
        Ok(Self {
            writer,
            hasher: HasherInstance::new(algorithm)?,
        })
    }

    pub fn finalize_hex(self) -> String {
        self.hasher.finalize_hex()
    }

    pub fn verify_digest(
        self,
        tool_name: &str,
        algorithm: &str,
        expected_digest: &str,
    ) -> Result<(), String> {
        let computed_digest = self.finalize_hex();
        if computed_digest != expected_digest {
            return Err(format!(
                "{} verification failed for {}.\nExpected: {}\nActual:   {}",
                algorithm.to_uppercase(),
                tool_name,
                expected_digest,
                computed_digest
            ));
        }
        Ok(())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.writer.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
