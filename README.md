# Roundtrip

A desktop HTTP client in Rust + Slint. Personal fork of [ApiArk](https://github.com/berbicanes/apiark)
(MIT, commit `46ba45a`), ported from Tauri 2 + React.

Roundtrip opens ApiArk collections as they are on disk (`.apiark/apiark.yaml`, one YAML file per
request, `.apiark/environments{,.local}/`, `.env` files) and writes them back without dropping fields
it does not understand yet. You can switch back to ApiArk at any time.

## Slice 1

- Open a collection, browse the tree, pick an environment (remembered between runs).
- Edit method, URL, params, headers and JSON/XML/raw bodies; `{{var}}` and `{{$uuid}}`-style variables.
- Send with Ctrl+Enter; status, time, size, pretty JSON, headers.
- Save with Ctrl+S; unsaved edits are marked ● and guarded by a Save / Discard / Cancel dialog.
- Not yet: cookies, history, tabs, scripts, other protocols.

## Slice 2

- Auth tab: Bearer, Basic, API key (header or query), or Inherit the collection's default auth
  (⚙ Collection settings). Other ApiArk auth types are kept on save but block Send.
- A header you write yourself (`Authorization`, or the API key's name) wins over auth.
- "+" in the sidebar creates a request or folder at the root; right-click a row for New request,
  New folder, Rename (F2) and Delete (Del). Delete is permanent.

## Known limits

- YAML comments are lost on save.
- Disabled and blank-key params/headers are not stored (ApiArk maps have no enabled flag); duplicate keys collapse.
- A personal environment with the same name as a shared one is shadowed.
- No file watcher: external edits after loading are overwritten on save.
- The previous response stays visible when switching request.
- Folder auth in _folder.yaml is ignored when sending (ApiArk does the same).

## Build

Fedora: `sudo dnf install fontconfig-devel libxkbcommon-devel`.

    cargo run -p app
    cargo test --workspace

Try it on `examples/sample-collection/` (uses https://httpbin.org).

## License

MIT. See `LICENSE`: ApiArk Contributors and kuhlen. Bundled fonts: SIL OFL (`crates/app/assets/fonts/OFL.txt`).
