use imap::Client;
use native_tls::TlsConnector;
use notify_rust::Notification;
use std::net::TcpStream;
use std::time::Duration;

const RECONNECT_DELAY: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 29);

fn main() {
    env_logger::init();
    let _ = dotenvy::dotenv();

    let imap_host = std::env::var("MAIL_IMAP_HOST").unwrap_or_else(|_| "imap.gmail.com".into());
    let imap_port: u16 = std::env::var("MAIL_IMAP_PORT")
        .unwrap_or_else(|_| "993".into())
        .parse()
        .expect("MAIL_IMAP_PORT must be a number");
    let username = std::env::var("MAIL_USERNAME").expect("MAIL_USERNAME not set");
    let password = std::env::var("MAIL_PASSWORD").expect("MAIL_PASSWORD not set");

    log::info!("mail-notify starting — watching {}:{}", imap_host, imap_port);

    loop {
        match watch(&imap_host, imap_port, &username, &password) {
            Ok(()) => log::info!("Session ended cleanly, reconnecting..."),
            Err(e) => log::error!("Error: {}, reconnecting in {:?}...", e, RECONNECT_DELAY),
        }
        std::thread::sleep(RECONNECT_DELAY);
    }
}

fn watch(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let tls = TlsConnector::builder().build()?;
    let addr = format!("{host}:{port}");
    let stream = TcpStream::connect(&addr)?;
    let tls_stream = tls.connect(host, stream)?;

    let mut client = Client::new(tls_stream);
    client.read_greeting()?;
    let mut session = client.login(username, password).map_err(|(e, _)| e)?;
    log::info!("Logged in as {username}");

    session.select("INBOX")?;
    log::info!("Watching INBOX via IDLE");

    let mut last_unseen: usize = session.search("UNSEEN")?.len();
    log::info!("Initial unseen count: {last_unseen}");

    loop {
        log::debug!("Entering IDLE...");
        {
            let mut handle = session.idle();
            handle.timeout(IDLE_TIMEOUT);
            let outcome = handle.wait_while(|resp| {
                log::debug!("IDLE got response: {:?}", resp);
                false
            })?;
            log::debug!("IDLE outcome: {:?}", outcome);
        }

        match session.search("UNSEEN") {
            Ok(ids) => {
                let current = ids.len();
                if current > last_unseen {
                    let new = current - last_unseen;
                    log::info!("{} new message(s)", new);

                    let snippet = ids.iter().max().copied().and_then(|id| {
                        let id_str = id.to_string();
                        let f = session
                            .fetch(id_str, "(BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])")
                            .ok()?;
                        f.iter().next().map(|msg| {
                            let raw = msg.header().unwrap_or_default();
                            let text = String::from_utf8_lossy(raw);
                            let subj = text
                                .lines()
                                .find(|l| l.starts_with("Subject:"))
                                .map(|l| l.trim_start_matches("Subject:").trim())
                                .unwrap_or("No subject");
                            let from = text
                                .lines()
                                .find(|l| l.starts_with("From:"))
                                .map(|l| l.trim_start_matches("From:").trim())
                                .unwrap_or("Unknown");
                            (from.to_string(), subj.to_string())
                        })
                    });

                    let (from, subject) =
                        snippet.unwrap_or_else(|| ("Unknown".into(), "No subject".into()));
                    let from_short = from.split('<').next().unwrap_or(&from).trim().to_string();
                    let subj_short = if subject.len() > 80 {
                        format!("{}...", &subject[..77])
                    } else {
                        subject.clone()
                    };

                    log::info!("From: {from_short} — {subj_short}");
                    let body = if new == 1 {
                        format!("{from_short}\n{subj_short}")
                    } else {
                        format!("{new} new emails (last: {from_short})")
                    };
                    let notif = Notification::new()
                        .summary("Gmail")
                        .body(&body)
                        .icon("mail-unread")
                        .timeout(5000)
                        .action("open-gmail", "Open Gmail")
                        .show()
                        .ok();
                    if let Some(handle) = notif {
                        handle.wait_for_action(|action| {
                            if action == "open-gmail" || action == "default" {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg("https://mail.google.com")
                                    .spawn();
                            }
                        });
                    }
                }
                last_unseen = current;
            }
            Err(e) => log::error!("Search error: {}", e),
        }

        let _ = session.noop();
    }
}
