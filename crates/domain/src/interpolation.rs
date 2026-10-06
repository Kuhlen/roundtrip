/// Replace `{{name}}` with `lookup(name.trim())`; unresolved stays as-is.
/// Same matching as ApiArk's `\{\{([^}]+)\}\}` regex, without the regex crate.
pub fn interpolate(input: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("{{") {
        let inner = &rest[start + 2..];
        let resolved = inner
            .find('}')
            .filter(|&end| end > 0 && inner[end..].starts_with("}}"))
            .map(|end| {
                (
                    lookup(inner[..end].trim()),
                    &rest[start..start + 2 + end + 2],
                    &inner[end + 2..],
                )
            });
        match resolved {
            Some((value, raw, after)) => {
                out.push_str(&rest[..start]);
                out.push_str(value.as_deref().unwrap_or(raw));
                rest = after;
            }
            // regex retries one char later: "{{{x}}" still matches "{x"
            None => {
                out.push_str(&rest[..start + 1]);
                rest = &rest[start + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}
