//! GraphQL ⇄ JSON body in ApiArk's layout: query, variables, operationName.

use domain::AppError;
use domain::graphql::GraphqlBody;
use serde_json::{Map, Value};

/// None: not an object of only query (string), variables, operationName (string or null).
pub fn parse(text: &str) -> Option<GraphqlBody> {
    let Value::Object(map) = serde_json::from_str(text).ok()? else {
        return None;
    };
    let known = ["query", "variables", "operationName"];
    // plain JSON bodies with a `query` key (e.g. Elasticsearch) must not be rewritten
    if !map.keys().all(|k| known.contains(&k.as_str()))
        || !matches!(
            map.get("operationName"),
            None | Some(Value::Null | Value::String(_))
        )
    {
        return None;
    }
    let query = map.get("query")?.as_str()?.to_owned();
    let variables = match map.get("variables") {
        Some(v) if !v.is_null() => serde_json::to_string_pretty(v).ok()?,
        _ => "{}".to_owned(),
    };
    let operation_name = map
        .get("operationName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Some(GraphqlBody {
        query,
        variables,
        operation_name,
    })
}

/// Blank or `{}` variables and an empty operation name are left out, as upstream does.
pub fn to_json(body: &GraphqlBody) -> Result<String, AppError> {
    let invalid = |e: serde_json::Error| AppError::InvalidJson(e.to_string());
    let mut map = Map::new();
    map.insert("query".into(), body.query.clone().into());
    if !body.variables.trim().is_empty() {
        let vars: Value = serde_json::from_str(&body.variables).map_err(invalid)?;
        if vars.as_object().is_none_or(|o| !o.is_empty()) {
            map.insert("variables".into(), vars);
        }
    }
    if !body.operation_name.is_empty() {
        map.insert("operationName".into(), body.operation_name.clone().into());
    }
    serde_json::to_string_pretty(&Value::Object(map)).map_err(invalid)
}
