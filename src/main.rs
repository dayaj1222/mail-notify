use imap::Client;
use native_tls::TlsConnector;
use std::collections::BTreeSet;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

/// Minimal leveled logging. Replaces `log` + `env_logger` (~10 crates, ~500 KB)
/// with two macros writing to stderr. systemd's journal adds timestamps, so
/// nothing observable is lost in this deployment.
macro_rules! log_info {
    ($($a:tt)*) => { eprintln!("[info] {}", format_args!($($a)*)) };
}
macro_rules! log_error {
    ($($a:tt)*) => { eprintln!("[error] {}", format_args!($($a)*)) };
}
macro_rules! log_debug {
    ($($a:tt)*) => { eprintln!("[debug] {}", format_args!($($a)*)) };
}

const RECONNECT_DELAY: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 29);
/// Subjects longer than this are truncated with an ellipsis (in chars, not bytes).
const SUBJECT_MAX: usize = 90;
/// Cap how many senders we list in the multi-mail body.
const MAX_SENDERS_SHOWN: usize = 3;

struct Account {
    label: String,
    username: String,
    password: String,
}

fn main() {
    
    let _ = dotenvy::dotenv();

    let accounts = load_accounts();
    if accounts.is_empty() {
        eprintln!("No accounts configured. Set MAIL_USERNAME + MAIL_PASSWORD in .env");
        std::process::exit(1);
    }

    log_info!("Starting {} mail watcher(s)", accounts.len());
    let mut handles = vec![];

    for acc in accounts {
        handles.push(thread::spawn(move || loop {
            log_info!("[{}] connecting...", acc.label);
            match watch_account(&acc) {
                Ok(()) => log_info!("[{}] disconnected, reconnecting...", acc.label),
                Err(e) => log_error!("[{}] error: {}, reconnecting...", acc.label, e),
            }
            std::thread::sleep(RECONNECT_DELAY);
        }));
    }

    for h in handles {
        h.join().ok();
    }
}

fn load_accounts() -> Vec<Account> {
    let mut accounts = Vec::new();

    // Account 1: MAIL_USERNAME / MAIL_PASSWORD
    if let (Ok(u), Ok(p)) = (
        std::env::var("MAIL_USERNAME"),
        std::env::var("MAIL_PASSWORD"),
    ) {
        accounts.push(Account {
            label: std::env::var("MAIL_LABEL").unwrap_or_else(|_| "default".into()),
            username: u,
            password: p,
        });
    }

    // Accounts 2, 3, 4...
    for i in 2..=10 {
        let key_u = format!("MAIL_USERNAME_{}", i);
        let key_p = format!("MAIL_PASSWORD_{}", i);
        match (std::env::var(&key_u), std::env::var(&key_p)) {
            (Ok(u), Ok(p)) => {
                let key_l = format!("MAIL_LABEL_{}", i);
                accounts.push(Account {
                    label: std::env::var(&key_l).unwrap_or_else(|_| format!("account-{}", i)),
                    username: u,
                    password: p,
                });
            }
            _ => break, // stop at first gap
        }
    }

    accounts
}

/// Strip reply/forward prefixes and collapse whitespace, then truncate to
/// SUBJECT_MAX characters (not bytes, so multibyte UTF-8 can't panic or split).
fn clean_subject(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Peel repeated "Re:", "Fwd:", "Fw:" prefixes (case-insensitive), possibly
    // stacked like "Re: Fwd: Re: ...".
    loop {
        let lower = s.to_ascii_lowercase();
        let stripped = ["re:", "fwd:", "fw:"]
            .iter()
            .find_map(|p| lower.strip_prefix(p).map(|_| s[p.len()..].trim().to_string()));
        match stripped {
            Some(next) if !next.is_empty() => s = next,
            _ => break,
        }
    }
    // Collapse runs of whitespace (and folded header newlines) into single spaces.
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > SUBJECT_MAX {
        let truncated: String = collapsed.chars().take(SUBJECT_MAX - 3).collect();
        format!("{}...", truncated)
    } else if collapsed.is_empty() {
        "(no subject)".to_string()
    } else {
        collapsed
    }
}

/// Short display name for a sender: the part before the address, or the whole
/// thing if there's no angle-bracket form.
fn sender_short(from: &str) -> String {
    let name = from.split('<').next().unwrap_or(from).trim();
    // Quoted display names: "Alice Smith" <a@b> -> Alice Smith
    let name = name.trim_matches('"').trim();
    if name.is_empty() {
        // Fall back to the address itself.
        from.split('<')
            .nth(1)
            .map(|a| a.trim_end_matches('>').trim().to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        name.to_string()
    }
}

/// Pull one header out of a raw header block, case-insensitively.
fn header_value<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    raw.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case(name) {
            Some(value.trim())
        } else {
            None
        }
    })
}

/// Pop a desktop notification via libnotify's `notify-send`. Replaces the
/// `notify-rust` D-Bus stack (and its ~140 transitive crates) with a single
/// process spawn. No action buttons, but we don't use them.
fn notify(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .arg("--app-name=mail-notify")
        .arg("--icon=mail-unread")
        .arg("--expire-time=5000")
        .arg(summary)
        .arg(body)
        .spawn();
}

/// Highest unseen UID, or 0 if the mailbox has no unseen mail. UIDs are
/// monotonically increasing per mailbox, so this is a stable watermark.
fn highest_unseen_uid(session: &mut imap::Session<native_tls::TlsStream<TcpStream>>) -> u32 {
    session
        .uid_search("UNSEEN")
        .map(|set| set.into_iter().max().unwrap_or(0))
        .unwrap_or(0)
}

fn watch_account(acc: &Account) -> Result<(), Box<dyn std::error::Error>> {
    let tls = TlsConnector::builder().build()?;
    let stream = TcpStream::connect(("imap.gmail.com", 993))?;
    let tls_stream = tls.connect("imap.gmail.com", stream)?;

    let mut client = Client::new(tls_stream);
    client.read_greeting()?;
    let mut session = client
        .login(&acc.username, &acc.password)
        .map_err(|(e, _)| e)?;
    log_info!("[{}] logged in", acc.label);

    session.select("INBOX")?;
    log_info!("[{}] watching INBOX", acc.label);

    let mut last_uid: u32 = highest_unseen_uid(&mut session);
    log_info!("[{}] initial unseen watermark: uid {}", acc.label, last_uid);

    loop {
        log_debug!("[{}] IDLE...", acc.label);
        {
            let mut handle = session.idle();
            handle.timeout(IDLE_TIMEOUT);
            let _ = handle.wait_while(|resp| {
                log_debug!("[{}] IDLE resp: {:?}", acc.label, resp);
                false
            })?;
        }

        match session.uid_search("UNSEEN") {
            Ok(set) => {
                let mut new_uids: BTreeSet<u32> =
                    set.into_iter().filter(|u| *u > last_uid).collect();

                if !new_uids.is_empty() {
                    // Advance the watermark before doing I/O, so a failed fetch
                    // doesn't cause the same mail to notify again next cycle.
                    last_uid = *new_uids.iter().max().unwrap();

                    log_info!("[{}] {} new message(s)", acc.label, new_uids.len());

                    // Fetch FROM + SUBJECT for each new message (newest first).
                    let mut items: Vec<(String, String)> = Vec::new();
                    for uid in std::mem::take(&mut new_uids).into_iter().rev() {
                        if let Ok(fetched) = session
                            .uid_fetch(uid.to_string(), "(BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])")
                        {
                            if let Some(msg) = fetched.iter().next() {
                                let raw = String::from_utf8_lossy(msg.header().unwrap_or_default());
                                let from = header_value(&raw, "From").unwrap_or("Unknown");
                                let subj = header_value(&raw, "Subject").unwrap_or("");
                                items.push((sender_short(from), clean_subject(subj)));
                            }
                        }
                    }

                    let n = items.len();
                    let body = if n == 1 {
                        let (from, subj) = items
                            .first()
                            .cloned()
                            .unwrap_or_else(|| ("Unknown".into(), "(no subject)".into()));
                        format!("{}\n{}", from, subj)
                    } else {
                        let names: Vec<String> = items
                            .iter()
                            .take(MAX_SENDERS_SHOWN)
                            .map(|(from, _)| from.clone())
                            .collect();
                        let mut line = format!("{} new: {}", n, names.join(", "));
                        if n > MAX_SENDERS_SHOWN {
                            line.push_str(&format!(", +{} more", n - MAX_SENDERS_SHOWN));
                        }
                        line
                    };

                    notify(&format!("Gmail — {}", acc.label), &body);
                }
            }
            Err(e) => log_error!("[{}] search error: {}", acc.label, e),
        }

        let _ = session.noop();
    }
}
