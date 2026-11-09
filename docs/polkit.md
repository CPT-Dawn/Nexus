# Polkit

Nexus never runs as root and never installs a polkit rule that is enabled.
NetworkManager's own polkit actions decide what your user may do. On a normal
logind session (a graphical login) scanning, connecting and managing your own
connections are allowed without a password.

## When you need a rule

The Daemon asks NetworkManager which actions are allowed (`GetPermissions`)
and greys out the ones that need authentication. If you see "auth required"
for an action you want to allow, typically because the Daemon runs in a
session polkit treats as inactive (lingering services, SSH), you can add a rule.

The package ships an example, disabled, at
`/usr/share/doc/nexus/examples/50-nexus-wheel.rules.example`. To enable it:

```sh
sudo install -Dm644 /usr/share/doc/nexus/examples/50-nexus-wheel.rules.example \
  /etc/polkit-1/rules.d/50-nexus.rules
```

It lets members of `wheel` use these NetworkManager actions without a password:
`network-control`, `wifi.scan`, `enable-disable-wifi`, `settings.modify.own`
and `settings.modify.system`. Edit the list to match your threat model, and
remove the file to go back to the defaults.
