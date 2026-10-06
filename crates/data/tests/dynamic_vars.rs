use data::dynamic_vars::resolve;

#[test]
fn uuid_is_36_chars() {
    assert_eq!(resolve("$uuid").unwrap().len(), 36);
}

#[test]
fn random_int_is_0_to_1000() {
    for _ in 0..200 {
        let n: u32 = resolve("$randomInt").unwrap().parse().unwrap();
        assert!(n <= 1000);
    }
}

#[test]
fn random_string_is_16_lowercase_alnum() {
    let s = resolve("$randomString").unwrap();
    assert_eq!(s.len(), 16);
    assert!(
        s.chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
    );
}

#[test]
fn unknown_is_none() {
    assert_eq!(resolve("$nope"), None);
    assert_eq!(resolve("uuid"), None);
}
