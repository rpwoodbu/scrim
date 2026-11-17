use version::{format_version, VERSION};

#[test]
fn test_format_version() {
    assert_eq!(format_version(None), format!("scrim {}", VERSION));
    assert_eq!(format_version(Some("")), format!("scrim {}", VERSION));
    assert_eq!(format_version(Some("   ")), format!("scrim {}", VERSION));
    assert_eq!(format_version(Some("{STABLE_GIT_COMMIT}")), format!("scrim {}", VERSION));
    assert_eq!(
        format_version(Some("e259af2039b3f2ade3bb24dcd413467e229418f2")),
        format!("scrim {} (e259af2039b3f2ade3bb24dcd413467e229418f2)", VERSION)
    );
    assert_eq!(
        format_version(Some("  e259af2039b3f2ade3bb24dcd413467e229418f2  ")),
        format!("scrim {} (e259af2039b3f2ade3bb24dcd413467e229418f2)", VERSION)
    );
}
