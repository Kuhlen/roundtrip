use data::graphql::{parse, to_json};
use domain::AppError;
use domain::graphql::GraphqlBody;

fn gql(query: &str, variables: &str, operation_name: &str) -> GraphqlBody {
    GraphqlBody {
        query: query.into(),
        variables: variables.into(),
        operation_name: operation_name.into(),
    }
}

#[test]
fn writes_upstream_layout() {
    assert_eq!(
        to_json(&gql("{ me }", "{\"id\": 1}", "Me")).unwrap(),
        "{\n  \"query\": \"{ me }\",\n  \"variables\": {\n    \"id\": 1\n  },\n  \"operationName\": \"Me\"\n}"
    );
}

#[test]
fn empty_variables_and_operation_are_left_out() {
    for vars in ["{}", "", "  "] {
        assert_eq!(
            to_json(&gql("{ me }", vars, "")).unwrap(),
            "{\n  \"query\": \"{ me }\"\n}",
            "{vars:?}"
        );
    }
}

#[test]
fn invalid_variables_are_an_error() {
    assert!(matches!(
        to_json(&gql("{ me }", "{\"id\": }", "")),
        Err(AppError::InvalidJson(_))
    ));
}

#[test]
fn parse_reads_compact_upstream_json() {
    assert_eq!(
        parse(r#"{"query":"{ me }","variables":{"id":1},"operationName":"Me"}"#),
        Some(gql("{ me }", "{\n  \"id\": 1\n}", "Me"))
    );
    assert_eq!(
        parse(r#"{"query":"{ me }"}"#),
        Some(gql("{ me }", "{}", ""))
    );
}

#[test]
fn parse_rejects_non_graphql() {
    assert_eq!(parse(r#"{"name":"Ayu"}"#), None);
    assert_eq!(parse("not json"), None);
    assert_eq!(parse("[1]"), None);
    assert_eq!(parse(r#"{"query":{"match_all":{}}}"#), None);
    assert_eq!(parse(r#"{"query":"q","size":1}"#), None);
}

#[test]
fn parse_then_write_is_stable_and_keeps_key_order() {
    let first = to_json(&parse(r#"{"query":"q","variables":{"b":1,"a":2}}"#).unwrap()).unwrap();
    assert_eq!(to_json(&parse(&first).unwrap()).unwrap(), first);
    assert!(first.find("\"b\"") < first.find("\"a\""));
}
