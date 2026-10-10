use std::collections::HashMap;

use domain::interpolation::{Segment, interpolate, segments};

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

fn var(name: &str, raw: &str) -> Segment {
    Segment::Var {
        name: name.into(),
        raw: raw.into(),
    }
}

#[test]
fn segments_split_text_and_variables() {
    assert_eq!(
        segments("{{baseUrl}}/users/{{ id }}"),
        [
            var("baseUrl", "{{baseUrl}}"),
            Segment::Text("/users/".into()),
            var("id", "{{ id }}"),
        ]
    );
}

#[test]
fn segments_keep_empty_and_unclosed_as_text() {
    assert_eq!(segments("a{{}}b{{c"), [Segment::Text("a{{}}b{{c".into())]);
}

#[test]
fn segments_include_dynamic_names() {
    assert_eq!(segments("{{$uuid}}"), [var("$uuid", "{{$uuid}}")]);
}

#[test]
fn segments_rejoin_to_the_input() {
    for input in [
        "",
        "plain",
        "{{a}}{{b}}",
        "x{{{y}}z",
        "{{ }}",
        "{{a}",
        "}}{{",
    ] {
        let joined: String = segments(input)
            .into_iter()
            .map(|s| match s {
                Segment::Text(t) => t,
                Segment::Var { raw, .. } => raw,
            })
            .collect();
        assert_eq!(joined, input);
    }
}
