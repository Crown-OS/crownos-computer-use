# crownos-computer-use

The one process the CrownOS compositor trusts to see the desktop for AI agents and act on it.
Agents never touch Wayland themselves: they launch `crownos-computer-use mcp`, a stdio MCP server
that forwards to this daemon, and the daemon does the looking and the clicking under the agent's
name, with a cursor of the agent's own where the compositor offers one.

Elements come from the best source each window has, in one shape, like Vercel's agent-desktop and
agent-browser: a CrownUI app's own semantic tree, AT-SPI for GTK, Qt, Electron and Chromium, or
the DOM of the agent browser. Every element carries a ref such as `@s7:e12` that dies with its
snapshot, so an agent that acts on a stale view is told to look again instead of clicking where
something else now is.

| Crate | Binary | What it is |
| --- | --- | --- |
| `crates/elements` → `crownos-ui-elements` | | the element record, refs, snapshots, `find` |
| `crates/protocol` → `crownos-computer-use-protocol` | | the crownos-ipc contract: windows, snapshots, actions, screenshots, events |
| `crates/sources` → `crownos-ui-sources` | | CrownUI over its MCP socket, AT-SPI, the agent browser over the DevTools pipe |
| `crates/windows` → `crownos-window-layer` | | the Wayland side: window list and control, injector, agent cursors, screenshots |
| `crates/daemon` | `crownos-computer-use` | the daemon, and `crownos-computer-use mcp` for agents |
| `crates/mcp` → `crownos-computer-use-mcp` | | the stdio MCP server as a library |
| `crates/events` | `crownos-events` | `crownos-events tail`: the daemon's events as JSON lines, for hooks |

## Use

```sh
crownos-computer-use                       # the daemon, one per session (a user service once packaged)
claude mcp add crownos -- crownos-computer-use mcp
crownos-events tail                        # window and focus events for hook scripts
```

Tools an agent gets: `list_windows`, `snapshot`, `find`, `act`, `screenshot`, `focus`, `close`,
`make_visible`, `make_hidden`, `launch` (with `headless` the program opens on the agent
workspace, out of the user's way), `agent_workspace`, `open_url`. `CROWNOS_AGENT_NAME` and
`CROWNOS_AGENT_COLOR` name the agent and colour its cursor. Text read from windows is returned
under an `untrusted_content` marker. Nothing is ever typed into an element that holds a secret,
and each agent gets at most 20 actions a second.

## What the compositor must allow

crownpositor shows `crownos_window_control_v1`, `crownos_agent_cursor_v1` and the context protocol
only to clients whose executable is listed in `~/.config/crownos/agents.ron` under
`trusted_executables`. The defaults cover the installed paths, `/usr/bin/crownos-computer-use`
among them. For a development build add the path of your `target/debug/crownos-computer-use`
there; the compositor reloads the file live. Without the grant the daemon still runs on
`ext_foreign_toplevel_list_v1`: windows are listed by title and app id but carry no process or
placement, so only CrownUI apps and browser tabs can be read, and nothing can be moved.

`crownos_agent_cursor_v1` is not implemented in the compositor yet. Until it is, every agent's
input goes through `crownos_input_injector_v1` and shares the user's pointer.

## The agent browser

The daemon drives a Chromium of its own on `$XDG_DATA_HOME/crownos/agent-browser` through
`--remote-debugging-pipe`. Chromium 136 and later ignore remote debugging on the default profile,
so the user's browser is never touched. Tabs appear in `list_windows` as `browser:<target>`.

## Develop

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p crownos-ui-sources --example atspi_dump -- <pid>          # a GTK or Qt app's tree
cargo run -p crownos-ui-sources --example cdp_dump -- /tmp/profile https://example.com
```

Sibling CrownOS repositories are path dependencies one directory up, as everywhere in CrownOS.
