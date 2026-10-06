use std::collections::HashMap;

use domain::interpolation::interpolate;

fn vars() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("baseUrl", "http://localhost:3000"),
        ("token", "abc123"),
        ("{a", "brace"),
    ])
}

fn run(input: &str) -> String {
    let v = vars();
    interpolate(input, |k| v.get(k).map(|s| s.to_string()))
}

// parity: apiark http/interpolation.rs tests
#[test]
fn basic() {
    assert_eq!(
        run("{{baseUrl}}/api/users"),
        "http://localhost:3000/api/users"
    );
    assert_eq!(run("Bearer {{token}}"), "Bearer abc123");
}

#[test]
fn unresolved_left_as_is() {
    assert_eq!(run("{{unknown}}"), "{{unknown}}");
}

#[test]
fn regex_edge_cases() {
    assert_eq!(run("{{ token }}"), "abc123");
    assert_eq!(run("{{}}"), "{{}}");
    assert_eq!(run("{{{a}}"), "brace");
    assert_eq!(run("{{token}"), "{{token}");
    assert_eq!(run("{{to}ken}}"), "{{to}ken}}");
    assert_eq!(run("a{{token}}b{{token}}c"), "aabc123babc123c");
    assert_eq!(run("ü{{token}}ü"), "üabc123ü");
}
