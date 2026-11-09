# A Profile's password goes to the keyring as an agent-owned secret, per Profile, opt-in

Some people do not want their Wi-Fi passwords in NetworkManager's system store, which any process with the right polkit grant can read (this is also what Reveal reads). Nexus offers "Keep in keyring" per Profile. It does not replace the system store: that stays the default, because it works without a session, before login and for every other client of NetworkManager.

A Profile kept in the keyring has `psk-flags` set to agent-owned. NetworkManager then never stores the password and asks its secret agent (the Daemon) whenever it needs one, including for autoconnect. The Daemon answers from the Secret Service through `oo7`, without a prompt, and writes a password the person typed after a failed one (`SaveSecrets`) back to it. Forgetting the Network (`DeleteSecrets`) removes the keyring entry. Items are found by attributes (`application`, `kind`, `profile-uuid`), never by label.

## Consequences

- Switching is a move, not a flag: turning it on reads the password, writes it to the keyring and only then rewrites the Profile; turning it off does the reverse. If the keyring does not answer the Profile is left exactly as it was and the Front end gets `keyring_unavailable` (CLI exit 33). Nothing is lost half way.
- If the keyring disappears later, a join falls back to the Secret prompt instead of failing, so the Profile is never bricked; it just asks again.
- The keyring is reached on the user's session bus, one short session per call, nothing cached. A locked default collection is unlocked through the Secret Service's own prompt, with a timeout.
- Profiles kept in the keyring only join while the Daemon runs and is registered as the agent (it is for the whole session); other NetworkManager clients (`nmcli`, nm-applet) cannot join them unless they have their own agent for it.
- Wi-Fi PSK/SAE passwords only. 802.1X, WEP and VPN secrets stay in the system store.
- A new dependency, `oo7` (MIT), with `native_crypto`; it uses the same zbus major as the rest of Nexus.
