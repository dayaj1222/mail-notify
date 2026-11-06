# mail-notify

A tiny IMAP IDLE watcher in Rust that pops desktop notifications for new Gmail.

It sits on your INBOX via IMAP IDLE, and whenever a new message arrives it fires a
`notify-send` popup with the sender and a cleaned-up subject — no polling, no mail client
needed.

## Features

- **Real-time** — IMAP IDLE push, no polling loops
- **Multi-account** — watch up to 10 accounts, one thread each, labelled notifications
- **Smart summaries** — strips `Re:/Fwd:` prefixes, truncates long subjects, batches
  simultaneous mail into one notification ("3 new: Alice, Bob, +1 more")
- **Resilient** — auto-reconnects every 30s on errors, UID watermark prevents duplicate
  notifications
- **Tiny** — three dependencies (`imap`, `native-tls`, `dotenvy`), stripped + LTO release
  build

## Setup

1. Create a [Gmail app password](https://myaccount.google.com/apppasswords)
   (requires 2-Step Verification).

2. Copy `.env.example` to `.env` and fill it in:

   ```ini
   MAIL_USERNAME=your.email@gmail.com
   MAIL_PASSWORD=your-16-char-app-password
   MAIL_LABEL=personal          # optional, shown in the notification
   ```

   More accounts: `MAIL_USERNAME_2`, `MAIL_PASSWORD_2`, `MAIL_LABEL_2`, … (stop at the
   first gap).

3. Build and run:

   ```sh
   cargo build --release
   ./target/release/mail-notify
   ```

Requires `notify-send` (libnotify) on a desktop session.

## Run as a systemd user service

```sh
cargo install --path . --root ~/.local --locked
```

Create `~/.config/systemd/user/mail-notify.service`:

```ini
[Unit]
Description=mail-notify Gmail watcher
After=graphical-session.target

[Service]
ExecStart=%h/.local/bin/mail-notify
Restart=on-failure

[Install]
WantedBy=default.target
```

```sh
systemctl --user daemon-reload
systemctl --user enable --now mail-notify
```

> Re-run `cargo install` after editing the source — the unit runs the installed binary,
> not `target/`.

## Notes

- `.env` contains credentials — never commit it (it's gitignored).
- See `SYSTEM_CHANGES.md` for the full setup/revert log.
