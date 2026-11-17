use elf::parse_elf_end_offset;
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
