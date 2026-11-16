# Nexus demo, rendered with https://github.com/charmbracelet/vhs
#
#   cargo build --release -p nexus
#   cargo build --release -p nexus-testkit --example demo
#   vhs demo/nexus.tape        # run from the repo root; writes assets/demo.mp4
#
# Everything on screen is the testkit's private world: a private dbus-daemon,
# a scripted fake NetworkManager and a real Daemon, with HOME and
# XDG_RUNTIME_DIR in a temp dir. No real network, name or path appears.
# The `radios` scenario: six Wi-Fi Networks in range, Home-5G joined.

Output assets/demo.mp4

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font Mono"
Set FontSize 36
Set LineHeight 1.2
Set Width 2560
Set Height 1600
Set Padding 36
Set Margin 56
Set MarginFill "#060a0b"
Set BorderRadius 16
Set WindowBar Colorful
Set WindowBarSize 48
Set Framerate 30
Set TypingSpeed 90ms
Set CursorBlink false
Set Theme { "name": "Midnight Bloom", "background": "#0b1214", "foreground": "#e4f2f0", "cursor": "#8de3dc", "selection": "#17302f", "black": "#0b1214", "brightBlack": "#5d7377", "red": "#f7768e", "brightRed": "#f7768e", "green": "#9ece6a", "brightGreen": "#9ece6a", "yellow": "#e0af68", "brightYellow": "#e0af68", "blue": "#7aa2f7", "brightBlue": "#7aa2f7", "magenta": "#bb9af7", "brightMagenta": "#bb9af7", "cyan": "#8de3dc", "brightCyan": "#8de3dc", "white": "#e4f2f0", "brightWhite": "#ffffff" }

# Launch off camera so the first frame is the populated app, not a prompt.
Hide
Type "clear && COLORTERM=truecolor target/release/examples/demo target/release/nexus radios 2>/dev/null"
Enter
Sleep 2.5s
Show

# Beat 1: the Overview: status, every kind of Network, traffic and activity.
Sleep 2.5s

# Beat 2: the Wi-Fi tab glides in with every Network in range.
Type "2"
Sleep 2s

# Beat 3: join a secured Network: the Daemon's Secret prompt asks for the password.
Down 1
Sleep 700ms
Enter
Sleep 1.2s
Type@110ms "correct-horse"
Sleep 400ms
Enter
Sleep 3s

# Beat 4: every action has an undo: turn Wi-Fi off, then take it back.
Type "w"
Sleep 2s
Type "u"
Sleep 1.6s

# Close where it opened, with the join now in Recent activity.
Type "1"
Sleep 3s
