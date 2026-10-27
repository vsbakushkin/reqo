# reqo

A fast native HTTP client built on [GPUI Kit](https://github.com/longbridge/gpui-kit).

```sh
cargo run                        # open the last workspace
cargo run -- examples/requests   # add a folder to it as a collection and open it
```

## Workspaces and collections

Like in Postman, a **workspace** holds **collections**, and a collection holds
requests, optionally grouped in folders. Switch, create and rename workspaces
from the menu at the top of the sidebar.

Everything is plain files. Workspaces live in `~/.local/share/reqo`
(`$XDG_DATA_HOME/reqo`):

```text
workspaces/<name>/
    workspace.toml        name and the list of collections
    reqo.env.toml         environments shared by all collections
    collections/<name>/   collections created in the app
```

A collection can also be any folder on disk, such as one inside a project's
repository: add it with the folder button in the sidebar, or pass it on the
command line. Removing such a collection from the workspace leaves the folder
alone.

## Files

A collection is a folder. Every `*.req.toml` inside it is a request, and
subfolders show up as folders in the sidebar:

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

Environments live in `reqo.env.toml`; `{{name}}` is substituted in the URL,
headers and body:

```toml
[dev]
base = "http://localhost:8080"
```

The workspace's file applies to every collection. A collection can have its own
`reqo.env.toml` at its root: its environments are added to the workspace's, and
its variables override those of the environment with the same name. Both files
open from the menus via *Edit environments*.

## Keys

| Key          | Action                  |
| ------------ | ----------------------- |
| Ctrl+Enter   | Send                    |
| Ctrl+S       | Save                    |
| Ctrl+N       | New request             |
| Ctrl+R       | Reload files from disk  |
| Ctrl+,       | Settings                |
| Ctrl+Shift+T | Toggle light/dark mode  |
| F2           | Rename request          |
| Delete       | Delete request          |

Edits are saved automatically when you send or switch requests.

## Layout

- `crates/reqo-core` — file format, workspaces, environments, request execution (no UI)
- `crates/reqo` — the GPUI app
