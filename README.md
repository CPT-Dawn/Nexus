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
