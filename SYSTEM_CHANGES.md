# System changes made for mail-notify setup
# Date: 2026-07-20
# To revert: run the commands under each section in reverse

## Files created outside this repo

# Systemd user service
/etc/systemd/user/mail-notify.service -> /home/daya/.config/systemd/user/mail-notify.service

## Commands to revert

# 1. Stop and disable the service
#    systemctl --user stop mail-notify
#    systemctl --user disable mail-notify
#
# 2. Remove the service file
#    rm ~/.config/systemd/user/mail-notify.service
#    systemctl --user daemon-reload
#
# 3. Remove the .env file (contains credentials)
#    rm ~/Development/mail-notify/.env
#
# 4. Delete the project
#    rm -rf ~/Development/mail-notify

## Dependencies installed (system packages)
# None — pure Rust, statically linked

## Dependencies installed (cargo crates)
# imap 3.0.0-alpha.15
# native-tls 0.2
# notify-rust 4
# dotenvy 0.15
# log 0.4
# env_logger 0.11
