#[test]
fn icns_has_a_valid_header_and_the_1024_png() {
    let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/app.icns")).unwrap();
    assert_eq!(&data[..4], b"icns");
    assert_eq!(
        u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize,
        data.len()
    );
    let mut types = Vec::new();
    let mut i = 8;
    while i < data.len() {
        let len = u32::from_be_bytes(data[i + 4..i + 8].try_into().unwrap()) as usize;
        assert_eq!(&data[i + 8..i + 16], b"\x89PNG\r\n\x1a\n");
        types.push(String::from_utf8(data[i..i + 4].to_vec()).unwrap());
        i += len;
    }
    assert_eq!(i, data.len());
    for t in [
        "icp4", "icp5", "ic11", "ic12", "ic07", "ic13", "ic08", "ic14", "ic09", "ic10",
    ] {
        assert!(types.iter().any(|x| x == t), "{t}");
    }
}
