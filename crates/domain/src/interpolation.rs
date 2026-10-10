#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    /// `name` trimmed for lookup; `raw` as written, shown when unresolved
    Var {
        name: String,
        raw: String,
    },
}

/// Split on `{{name}}` with ApiArk's `\{\{([^}]+)\}\}` matching, without the regex crate.
pub fn segments(input: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let inner = &rest[start + 2..];
        match inner
            .find('}')
            .filter(|&end| end > 0 && inner[end..].starts_with("}}"))
        {
            Some(end) => {
                text.push_str(&rest[..start]);
                if !text.is_empty() {
                    out.push(Segment::Text(std::mem::take(&mut text)));
                }
                out.push(Segment::Var {
                    name: inner[..end].trim().to_owned(),
                    raw: rest[start..start + 2 + end + 2].to_owned(),
                });
                rest = &inner[end + 2..];
            }
            // regex retries one char later: "{{{x}}" still matches "{x"
            None => {
                text.push_str(&rest[..start + 1]);
                rest = &rest[start + 1..];
            }
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        out.push(Segment::Text(text));
    }
    out
}

/// Replace `{{name}}` with `lookup(name.trim())`; unresolved stays as-is.
pub fn interpolate(input: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    segments(input)
        .into_iter()
        .map(|s| match s {
            Segment::Text(t) => t,
            Segment::Var { name, raw } => lookup(&name).unwrap_or(raw),
        })
        .collect()
}
