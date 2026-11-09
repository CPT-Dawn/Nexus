# Nexus

A network manager for Wayland desktops. An unprivileged daemon owns your
network state and talks to NetworkManager; a terminal UI and a scriptable CLI
talk to the daemon.

> Nexus v2 is under active development. Betas are published as GitHub
> prereleases; the AUR packages move to 2.0.0 when the quality gate passes.

<!-- Screenshots: placeholder, added before 2.0.0. -->

## Install

Arch Linux, from the AUR:

```sh
paru -S nexus-nm       # build from source
paru -S nexus-nm-bin   # prebuilt (x86_64, aarch64)
```

Both install the `nexus` binary, systemd user units, shell completions (bash,
zsh, fish) and the `nexus(1)` man page. NetworkManager must be your network
manager; Nexus does not replace it.

## First run

The package enables `nexus.socket` for every user through a systemd user
preset, so the daemon starts on demand after you log in again. To start it
right away:

```sh
systemctl --user enable --now nexus.socket
nexus status
```

`nexus status --json` prints the same thing for scripts. If the daemon cannot
be reached, `nexus` exits with code 3 and says how to start it.

## Scripting

Every read command takes `--json`, every action has a command (`nexus --help`,
`man nexus`, or `man nexus-wifi-join` for one command), and every failure has
its own exit code, listed in [docs/ipc.md](docs/ipc.md#exit-codes). Shell
completions for bash, zsh and fish and the man pages are installed with the
package. Anything the CLI does not cover, a program can do over the daemon's
socket: [docs/ipc.md](docs/ipc.md) documents the JSON-lines protocol with
`socat` and `jq` examples, and the `nexus-ipc` crate
([README](crates/nexus-ipc/README.md)) is the Rust library for writing your own Front end.

## Permissions

Nexus never runs as root. NetworkManager's polkit rules decide what you may
do, and a normal graphical login is allowed the common Wi-Fi actions. An
optional rule for the cases that ask for a password is documented in
[docs/polkit.md](docs/polkit.md); nothing is installed enabled.

## Privacy

Nexus makes no network requests of its own: no telemetry, no update checks, no
speed tests. The daemon's socket accepts connections from your user only.

## Development

See [GLOSSARY.md](GLOSSARY.md) for the vocabulary, [docs/adr](docs/adr) for the
decisions behind the design and [docs/testing.md](docs/testing.md) for the test
harness. Releases are cut by `.github/workflows/release.yml` from signed `v2.*`
tags.

## License

MIT, see [LICENSE](LICENSE).
