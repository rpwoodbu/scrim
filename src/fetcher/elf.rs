use std::io::{Read, Seek, SeekFrom};

/// Parses an ELF binary from `reader` and returns the `(e_ident, end_offset)` tuple,
/// where `e_ident` is the 16-byte ELF identification header and `end_offset` is the
/// byte offset immediately following the ELF headers, program headers/segments, and
/// section headers/sections.
pub fn parse_elf_end_offset<R: Read + Seek>(reader: &mut R) -> Result<([u8; 16], u64), String> {
    let mut header = [0u8; 64];
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|e| format!("Failed to read ELF header: {}", e))?;
    let bytes_read = reader
        .read(&mut header)
        .map_err(|e| format!("Failed to read ELF header: {}", e))?;

    if bytes_read < 16 || &header[0..4] != b"\x7fELF" {
        return Err("not a valid ELF binary".to_string());
    }

    let mut e_ident = [0u8; 16];
    e_ident.copy_from_slice(&header[0..16]);

    let ei_class = header[4]; // 1 = 32-bit, 2 = 64-bit
    let ei_data = header[5]; // 1 = Little Endian, 2 = Big Endian
    if ei_data != 1 && ei_data != 2 {
        return Err(format!("Invalid ELF endianness: {}", ei_data));
    }

    let read_u16 = |b: &[u8]| -> u16 {
        if ei_data == 2 {
            u16::from_be_bytes([b[0], b[1]])
        } else {
            u16::from_le_bytes([b[0], b[1]])
        }
    };
    let read_u32 = |b: &[u8]| -> u32 {
        if ei_data == 2 {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        }
    };
    let read_u64 = |b: &[u8]| -> u64 {
        if ei_data == 2 {
            u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        } else {
            u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        }
    };

    let (e_phoff, e_phentsize, e_phnum, e_shoff, e_shentsize, e_shnum) = match ei_class {
        1 => {
            if bytes_read < 52 {
                return Err("Invalid 32-bit ELF header: file too short".to_string());
            }
            let phoff = read_u32(&header[28..32]) as u64;
            let shoff = read_u32(&header[32..36]) as u64;
            let phentsize = read_u16(&header[42..44]) as u64;
            let phnum = read_u16(&header[44..46]) as u64;
            let shentsize = read_u16(&header[46..48]) as u64;
            let shnum = read_u16(&header[48..50]) as u64;
            (phoff, phentsize, phnum, shoff, shentsize, shnum)
        }
        2 => {
            if bytes_read < 64 {
                return Err("Invalid 64-bit ELF header: file too short".to_string());
            }
            let phoff = read_u64(&header[32..40]);
            let shoff = read_u64(&header[40..48]);
            let phentsize = read_u16(&header[54..56]) as u64;
            let phnum = read_u16(&header[56..58]) as u64;
            let shentsize = read_u16(&header[58..60]) as u64;
            let shnum = read_u16(&header[60..62]) as u64;
            (phoff, phentsize, phnum, shoff, shentsize, shnum)
        }
        _ => return Err(format!("Unsupported ELF class: {}", ei_class)),
    };

    let mut max_offset = 0u64;

    if e_phoff > 0 && e_phnum > 0 {
        let min_phentsize = if ei_class == 1 { 32 } else { 56 };
        if e_phentsize < min_phentsize {
            return Err(format!(
                "Invalid ELF program header entry size: {}",
                e_phentsize
            ));
        }
        let ph_table_end = e_phnum
            .checked_mul(e_phentsize)
            .and_then(|sz| e_phoff.checked_add(sz))
            .ok_or_else(|| "ELF program header table offset overflow".to_string())?;
        if ph_table_end > max_offset {
            max_offset = ph_table_end;
        }

        for i in 0..e_phnum {
            let ph_pos = i
                .checked_mul(e_phentsize)
                .and_then(|off| e_phoff.checked_add(off))
                .ok_or_else(|| "ELF program header offset overflow".to_string())?;
            reader
                .seek(SeekFrom::Start(ph_pos))
                .map_err(|e| format!("Failed to seek to ELF program header: {}", e))?;
            if ei_class == 1 {
                let mut phdr = [0u8; 32];
                reader
                    .read_exact(&mut phdr)
                    .map_err(|e| format!("Failed to read 32-bit ELF program header: {}", e))?;
                let p_offset = read_u32(&phdr[4..8]) as u64;
                let p_filesz = read_u32(&phdr[16..20]) as u64;
                let end = p_offset
                    .checked_add(p_filesz)
                    .ok_or_else(|| "ELF segment size overflow".to_string())?;
                if end > max_offset {
                    max_offset = end;
                }
            } else {
                let mut phdr = [0u8; 56];
                reader
                    .read_exact(&mut phdr)
                    .map_err(|e| format!("Failed to read 64-bit ELF program header: {}", e))?;
                let p_offset = read_u64(&phdr[8..16]);
                let p_filesz = read_u64(&phdr[32..40]);
                let end = p_offset
                    .checked_add(p_filesz)
                    .ok_or_else(|| "ELF segment size overflow".to_string())?;
                if end > max_offset {
                    max_offset = end;
                }
            }
        }
    }

    if e_shoff > 0 && e_shnum > 0 {
        let min_shentsize = if ei_class == 1 { 40 } else { 64 };
        if e_shentsize < min_shentsize {
            return Err(format!(
                "Invalid ELF section header entry size: {}",
                e_shentsize
            ));
        }
        let sh_table_end = e_shnum
            .checked_mul(e_shentsize)
            .and_then(|sz| e_shoff.checked_add(sz))
            .ok_or_else(|| "ELF section header table offset overflow".to_string())?;
        if sh_table_end > max_offset {
            max_offset = sh_table_end;
        }

        for i in 0..e_shnum {
            let sh_pos = i
                .checked_mul(e_shentsize)
                .and_then(|off| e_shoff.checked_add(off))
                .ok_or_else(|| "ELF section header offset overflow".to_string())?;
            reader
                .seek(SeekFrom::Start(sh_pos))
                .map_err(|e| format!("Failed to seek to ELF section header: {}", e))?;
            if ei_class == 1 {
                let mut shdr = [0u8; 40];
                reader
                    .read_exact(&mut shdr)
                    .map_err(|e| format!("Failed to read 32-bit ELF section header: {}", e))?;
                let sh_type = read_u32(&shdr[4..8]);
                if sh_type != 8 {
                    // SHT_NOBITS == 8
                    let sh_offset = read_u32(&shdr[16..20]) as u64;
                    let sh_size = read_u32(&shdr[20..24]) as u64;
                    let end = sh_offset
                        .checked_add(sh_size)
                        .ok_or_else(|| "ELF section size overflow".to_string())?;
                    if end > max_offset {
                        max_offset = end;
                    }
                }
            } else {
                let mut shdr = [0u8; 64];
                reader
                    .read_exact(&mut shdr)
                    .map_err(|e| format!("Failed to read 64-bit ELF section header: {}", e))?;
                let sh_type = read_u32(&shdr[4..8]);
                if sh_type != 8 {
                    // SHT_NOBITS == 8
                    let sh_offset = read_u64(&shdr[24..32]);
                    let sh_size = read_u64(&shdr[32..40]);
                    let end = sh_offset
                        .checked_add(sh_size)
                        .ok_or_else(|| "ELF section size overflow".to_string())?;
                    if end > max_offset {
                        max_offset = end;
                    }
                }
            }
        }
    }

    if max_offset == 0 {
        return Err("could not determine ELF size from headers".to_string());
    }

    Ok((e_ident, max_offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use test_helpers::make_elf64_stub;

    #[test]
    fn test_parse_elf64_le_end_offset() {
        let stub = make_elf64_stub();
        let mut cursor = Cursor::new(stub);
        let (ident, offset) = parse_elf_end_offset(&mut cursor).unwrap();
        assert_eq!(&ident[0..4], b"\x7fELF");
        assert_eq!(&ident[8..11], b"AI\x02");
        assert_eq!(offset, 128);
    }

    #[test]
    fn test_parse_elf32_big_endian_and_phdr_and_nobits() {
        let mut elf32_be = vec![0u8; 500];
        elf32_be[0..4].copy_from_slice(b"\x7fELF");
        elf32_be[4] = 1; // 32-bit
        elf32_be[5] = 2; // Big Endian
        elf32_be[6] = 1; // ELF version
        elf32_be[8..11].copy_from_slice(b"AI\x02");

        // e_phoff (28..32) = 52
        elf32_be[28..32].copy_from_slice(&52u32.to_be_bytes());
        // e_shoff (32..36) = 84
        elf32_be[32..36].copy_from_slice(&84u32.to_be_bytes());
        // e_phentsize (42..44) = 32
        elf32_be[42..44].copy_from_slice(&32u16.to_be_bytes());
        // e_phnum (44..46) = 1
        elf32_be[44..46].copy_from_slice(&1u16.to_be_bytes());
        // e_shentsize (46..48) = 40
        elf32_be[46..48].copy_from_slice(&40u16.to_be_bytes());
        // e_shnum (48..50) = 2
        elf32_be[48..50].copy_from_slice(&2u16.to_be_bytes());

        // Program header 0 at 52..84: p_offset (4..8) = 200, p_filesz (16..20) = 150 -> end = 350
        elf32_be[52 + 4..52 + 8].copy_from_slice(&200u32.to_be_bytes());
        elf32_be[52 + 16..52 + 20].copy_from_slice(&150u32.to_be_bytes());

        // Section header 0 at 84..124: sh_type (4..8) = 1 (SHT_PROGBITS), sh_offset (16..20) = 350, sh_size (20..24) = 50 -> end = 400
        elf32_be[84 + 4..84 + 8].copy_from_slice(&1u32.to_be_bytes());
        elf32_be[84 + 16..84 + 20].copy_from_slice(&350u32.to_be_bytes());
        elf32_be[84 + 20..84 + 24].copy_from_slice(&50u32.to_be_bytes());

        // Section header 1 at 124..164: sh_type (4..8) = 8 (SHT_NOBITS), sh_offset (16..20) = 400, sh_size (20..24) = 9999 -> ignored
        elf32_be[124 + 4..124 + 8].copy_from_slice(&8u32.to_be_bytes());
        elf32_be[124 + 16..124 + 20].copy_from_slice(&400u32.to_be_bytes());
        elf32_be[124 + 20..124 + 24].copy_from_slice(&9999u32.to_be_bytes());

        let mut cursor = Cursor::new(elf32_be);
        let (_, offset) = parse_elf_end_offset(&mut cursor).unwrap();
        assert_eq!(offset, 400);
    }

    #[test]
    fn test_parse_elf_invalid_headers() {
        // 1. Not an ELF binary
        let mut c0 = Cursor::new(b"not an elf file".to_vec());
        assert!(parse_elf_end_offset(&mut c0)
            .unwrap_err()
            .contains("not a valid ELF binary"));

        // 2. Invalid endianness (ei_data = 99)
        let mut bad_endian = make_elf64_stub();
        bad_endian[5] = 99;
        let mut c1 = Cursor::new(bad_endian);
        assert!(parse_elf_end_offset(&mut c1)
            .unwrap_err()
            .contains("Invalid ELF endianness"));

        // 3. Invalid section header entry size (e_shentsize = 10 < 64)
        let mut bad_shentsize = make_elf64_stub();
        bad_shentsize[58..60].copy_from_slice(&10u16.to_le_bytes());
        let mut c2 = Cursor::new(bad_shentsize);
        assert!(parse_elf_end_offset(&mut c2)
            .unwrap_err()
            .contains("Invalid ELF section header entry size"));
    }
}
