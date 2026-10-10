# Roundtrip

A desktop HTTP client in Rust + Slint. Personal fork of [ApiArk](https://github.com/berbicanes/apiark)
(MIT, commit `46ba45a`), ported from Tauri 2 + React.

Roundtrip opens ApiArk collections as they are on disk (`.apiark/apiark.yaml`, one YAML file per
request, `.apiark/environments{,.local}/`, `.env` files) and writes them back without dropping fields
it does not understand yet. You can switch back to ApiArk at any time.

## Features

- Open a collection, browse the tree, pick an environment (remembered between runs).
- Edit environments with the pencil button next to the environment picker: create, duplicate, rename, delete, move
  between Shared and Personal (`.apiark/environments.local/`, gitignored). A Secret variable keeps
  its value in `.apiark/.env` (gitignored, masked in the table) instead of the YAML file.
- Edit method, URL, params, headers; `{{var}}` and `{{$uuid}}`-style variables.
- Bodies: JSON, XML, raw, urlencoded and form-data (a table; form-data rows can be files), binary
  (a file), and GraphQL (Query, Variables, Operation name). `…` / "Choose file…" store a path
  relative to the collection when the file is inside it; `{{var}}` works in paths.
- Auth tab: Bearer, Basic, API key (header or query), or Inherit the collection's default auth
  ("Collection settings…" in a collection row's right-click menu, which also has "Close collection").
  A header you write yourself (`Authorization`, or the API key's name) wins over auth.
- Send with Ctrl+Enter; status, time, size, pretty JSON, headers.
- Cancel a request in flight with the Cancel button or Esc; the connection closes at once.
- History of sent requests in the sidebar: search by URL, method or name, reopen any entry in a scratch tab, clear all. Auth secrets and secret headers are stored as `[REDACTED]`.
- Paste a cURL command into the URL bar to fill the request.
- Copy the active request as cURL with a button or Ctrl+Shift+C.
- Import a Postman v2.0/v2.1 collection, an OpenAPI 3.x spec or an Insomnia v4/v5 export as a new ApiArk collection from the sidebar "+" menu.
- Export a collection as a Postman v2.1 file from its right-click menu.
- Open several collections at once; environments come from the first one (as in ApiArk).
- Tabs: open, pin, drag to reorder, close (×, middle click, Ctrl+W), Close others / all; open tabs come back on the next start.
- Ctrl+T opens a scratch request you can send without a collection; Ctrl+S saves it into a collection.
- Edits are saved automatically one second after the last change.
- Ctrl+S saves immediately; ● marks a tab with edits not saved yet.
- "+" in the sidebar creates a request or folder at the root of the first collection, or imports a collection or API spec; right-click a row for New request,
  New folder, Rename (F2) and Delete (Del). Delete is permanent.
- ApiArk content Roundtrip can't edit (other auth types, other body types, a Bruno or HAR import)
  is kept on save but blocks Send. Invalid GraphQL Variables block Send and Save.

## Known limits

- YAML comments are lost on save.
- Disabled and blank-key params/headers are not stored (ApiArk maps have no enabled flag); duplicate keys collapse.
- A personal environment with the same name as a shared one (made outside Roundtrip) is shadowed; the editor refuses such names.
- No file watcher: external edits after loading are overwritten on save.
- Every request uses the first collection's environment, as in ApiArk.
- Folder auth in _folder.yaml is ignored when sending (ApiArk does the same).
- Disabled urlencoded rows are not stored (same as params and headers).
- ApiArk sends a binary body's path as text, not the file.
- The history list shows the newest 50 entries; search to reach older ones. Responses are not kept.
- A reopened history entry carries the auth its collection had at send time (secrets redacted), not Inherit.
- A secret key has one value per collection: environments that both mark `token` secret share it.
- Once saved, secret variables are listed after the others.
- The root `.env` file is not editable in the app.

## Build

Fedora: `sudo dnf install fontconfig-devel libxkbcommon-devel`.

    cargo run -p app
    cargo test --workspace

Try it on `examples/sample-collection/` (uses https://httpbin.org).

## License

MIT. See `LICENSE`: ApiArk Contributors and kuhlen. Bundled fonts: SIL OFL (`crates/app/assets/fonts/OFL.txt`).
