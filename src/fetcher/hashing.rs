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
