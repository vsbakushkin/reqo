# reqo

A fast native HTTP client built on [GPUI Kit](https://github.com/longbridge/gpui-kit).

```sh
cargo run -- examples/requests   # open a folder as a collection
```

## Files

A collection is a folder. Every `*.req.toml` inside it is a request:

```toml
name = "Create user"
method = "POST"
url = "{{base}}/users"
body = """
{ "name": "vasya" }
"""

[headers]
Content-Type = "application/json"
```

Environments live in `reqo.env.toml` at the folder root; `{{name}}` is
substituted in the URL, headers and body:

```toml
[dev]
base = "http://localhost:8080"
```

## Keys

| Key          | Action                  |
| ------------ | ----------------------- |
| Ctrl+Enter   | Send                    |
| Ctrl+S       | Save                    |
| Ctrl+N       | New request             |
| Ctrl+R       | Reload files from disk  |

Edits are saved automatically when you send or switch requests.

## Layout

- `crates/reqo-core` — file format, environments, request execution (no UI)
- `crates/reqo` — the GPUI app
