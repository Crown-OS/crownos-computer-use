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
crownos-computer-use                       # the daemon, one per session (the crownos-computer-use user service)
claude mcp add crownos -- crownos-computer-use mcp
crownos-events tail                        # window and focus events for hook scripts
```

Tools an agent gets: `list_windows`, `snapshot`, `find`, `act`, `type_secret`, `fill_login`, `screenshot`, `focus`, `close`,
`make_visible`, `make_hidden`, `launch` (with `headless` the program opens on the agent
workspace, out of the user's way), `agent_workspace`, `open_url`. `CROWNOS_AGENT_NAME` and
`CROWNOS_AGENT_COLOR` name the agent and colour its cursor. Text read from windows is returned
under an `untrusted_content` marker. Nothing is ever typed into an element that holds a secret,
and each agent gets at most 20 actions a second.

`type_secret {ref, name}` and `fill_login {username_ref, password_ref, username_name,
password_name}` type stored secrets by name. `type_secret` types only into a secure (password)
element. `fill_login` needs a plain element for the user name and a secure element for the
password, in the same window, and sends one request: the user name, Tab, then the password,
with one confirmation.

For a browser tab the daemon brings the tab to the front, finds the one window of the agent
browser process whose title is exactly `<tab title> - <browser name>`, focuses it through the
compositor, checks in the page that the element has the focus (`document.hasFocus()` and
`activeElement`), and checks the window again. A background tab, a title that only starts the
same, another browser process or two windows with the same title give `focus changed`, and
nothing is typed. The daemon then asks crownos-secretsd (`type_into` or `fill_login`) with the
window, its app id and the page origin. The compositor shows its own confirmation and types
through text-input. Chromium 153 sends the text-input password purpose on Wayland without
extra flags, so the agent browser needs none.

The daemon never holds a value. The result says only what happened (`typed`, `denied`, `focus
changed`, `timed out` …) and the page origin after the request, so the agent can see a redirect.
Each field a secret goes into is remembered before the request is sent, under its tab and its
toplevel, until the page navigates or the field leaves the document (another window's field:
until the window closes). Meanwhile screenshots of the tab and the toplevel are refused,
snapshots and `find` of the tab hide the field's value and text even when the page reveals a
password, and snapshots and `find` of the toplevel through another view are refused. The
secrets' bindings must name `exe:/usr/bin/crownos-computer-use` as an agent, and crownos-secretsd
takes typing requests only from the installed, root-owned computer-use daemon.

## What the compositor must allow

crownpositor shows `crownos_window_control_v1`, `crownos_agent_cursor_v1` and the context protocol
only to clients whose executable is listed in `~/.config/crownos/agents.ron` under
`trusted_executables`. The defaults cover the installed paths, `/usr/bin/crownos-computer-use`
among them. For a development build add the path of your `target/debug/crownos-computer-use`
there; the compositor reloads the file live. Without the grant the daemon still runs on
`ext_foreign_toplevel_list_v1`: windows are listed by title and app id but carry no process or
placement, so only CrownUI apps and browser tabs can be read, and nothing can be moved.

The daemon gives each agent its own cursor through `crownos_agent_cursor_v1` when the compositor
offers it, and otherwise sends the input through `crownos_input_injector_v1` on the user's pointer.

## The agent browser

The daemon drives the browser chosen at install time (Brave, Google Chrome or Chromium) on a profile
of its own, `$XDG_DATA_HOME/crownos/agent-browser/<browser>`, through `--remote-debugging-pipe`. It
reads the choice from `/etc/crownos/setup.ron` each time it launches the browser; without that file
the daemon uses Chromium. While the chosen Chromium-based browser is not installed yet, the daemon
falls back to Chromium. Firefox cannot be driven over the pipe: when it is the choice, the daemon
refuses to launch it and returns an error that names the playwright MCP server
(`crownos-setup playwright-mcp`). Chromium 136 and later ignore remote debugging on the default
profile, so the user's browser is never touched. Tabs appear in `list_windows` as
`browser:<target>`.

## Develop

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p crownos-ui-sources --example atspi_dump -- <pid>          # a GTK or Qt app's tree
cargo run -p crownos-ui-sources --example cdp_dump -- /tmp/profile https://example.com
```

Sibling CrownOS repositories are path dependencies one directory up, as everywhere in CrownOS.
