//! Signing in, through the system browser (docs/native/cloud.md, "Signing
//! in").
//!
//! Hangar listens on a loopback port, draws a state and a PKCE verifier, and
//! hands back the address of the server's start page for the browser to
//! open. The server runs the provider's consent and sends the browser back to
//! `http://127.0.0.1:<port>/callback` with a one-time link code and the
//! state. The state proves the visit is the answer to this sign-in; the code
//! is then exchanged, with the verifier that never left this process, for the
//! device token, which goes to the keychain.
//!
//! The listener is a few lines of HTTP on a thread of its own: one route, one
//! page, no runtime to borrow from whoever starts a sign-in. It serves until
//! the right visit comes, the sign-in is cancelled, or its time is up.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::api::{self, paths};
use super::client::url_value;
use super::{sync, Account, Cloud, Device, Error, ExchangeRequest, MachineInfo, Provider};

/// The longest a sign-in waits for the browser to come back. The server's
/// link code lives five minutes from the start page; the rest is room for a
/// person who opened the page late.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How often the listener looks up from its socket to see whether it was
/// cancelled or ran out of time.
const TICK: Duration = Duration::from_millis(25);

/// How long a visitor gets to send its request, and to take its answer.
const VISIT_TIMEOUT: Duration = Duration::from_secs(2);

/// The longest request head read: a browser's, with room to spare.
const MAX_HEAD_BYTES: usize = 16 * 1024;

/// Who signed in: the account, and this machine's device on it. The token is
/// not here: it went to the keychain and is read back from there only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIn {
    pub account: Account,
    pub device: Device,
}

/// A way to call a sign-in off, from any thread.
#[derive(Clone)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn asked(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A sign-in under way: the address to open in the browser, and the wait for
/// its outcome. Dropping it calls the sign-in off.
pub struct SignIn {
    url: String,
    port: u16,
    cancel: Cancel,
    outcome: Receiver<Result<SignedIn, Error>>,
}

impl SignIn {
    /// The page to open in the system browser. It carries the state and the
    /// challenge of this sign-in: it is for the browser, not for a log.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The loopback port the browser comes back to.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// A handle that cancels this sign-in from elsewhere, while `wait` holds
    /// the sign-in itself.
    pub fn canceller(&self) -> Cancel {
        self.cancel.clone()
    }

    /// Waits for the outcome: at most the sign-in's ten minutes, less when it
    /// is cancelled.
    pub fn wait(self) -> Result<SignedIn, Error> {
        self.outcome.recv().unwrap_or(Err(Error::Cancelled))
    }

    /// The outcome if it is known, without waiting: `None` while the browser
    /// has not come back.
    pub fn poll(&self) -> Option<Result<SignedIn, Error>> {
        match self.outcome.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(Error::Cancelled)),
        }
    }
}

impl Drop for SignIn {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Cloud {
    /// Starts a sign-in with `provider`. Nothing is sent yet: the caller
    /// opens `SignIn::url` in the system browser, and the first request to
    /// the server is the browser's.
    pub fn sign_in(&self, provider: Provider) -> Result<SignIn, Error> {
        self.sign_in_within(provider, SIGN_IN_TIMEOUT)
    }

    pub(super) fn sign_in_within(
        &self,
        provider: Provider,
        patience: Duration,
    ) -> Result<SignIn, Error> {
        // A keychain that does not answer fails the sign-in here, before the
        // person has gone through the provider's pages for nothing.
        self.inner.tokens.load().map_err(Error::Keychain)?;
        let machine = self.machine()?;
        let state = random_text()?;
        let verifier = random_text()?;

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
            .map_err(|err| Error::Local(format!("no loopback port to sign in through: {err}")))?;
        let port = listener
            .local_addr()
            .map_err(|err| Error::Local(format!("no loopback port to sign in through: {err}")))?
            .port();
        if port < api::MIN_LOOPBACK_PORT {
            return Err(Error::Local(format!(
                "the system gave port {port}, which the sign-in cannot come back to"
            )));
        }

        let url = format!(
            "{}{}?provider={}&port={}&state={}&challenge={}&machine={}",
            self.inner.client.base(),
            paths::AUTH_START,
            provider.as_str(),
            port,
            url_value(&state),
            url_value(&challenge_of(&verifier)),
            url_value(&machine.name),
        );

        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let (sender, outcome) = mpsc::channel();
        let cloud = self.clone();
        let asked = cancel.clone();
        std::thread::Builder::new()
            .name("hangar-sign-in".into())
            .spawn(move || {
                let deadline = Instant::now() + patience;
                let outcome = listen(listener, &state, &asked, deadline)
                    .and_then(|code| cloud.finish_sign_in(code, verifier, machine));
                let _ = sender.send(outcome);
            })
            .map_err(|err| Error::Local(format!("the sign-in could not start: {err}")))?;

        Ok(SignIn {
            url,
            port,
            cancel,
            outcome,
        })
    }

    /// The link code for the token, and the token to the keychain.
    fn finish_sign_in(
        &self,
        code: String,
        verifier: String,
        machine: MachineInfo,
    ) -> Result<SignedIn, Error> {
        let client = &self.inner.client;
        let answer = client.exchange(&ExchangeRequest {
            code,
            verifier,
            machine,
        })?;
        if answer.token.trim().is_empty() {
            return Err(Error::Answer("the exchange gave no token".into()));
        }
        if let Err(why) = self.inner.tokens.save(&answer.token) {
            // The token cannot be kept, so it must not stay valid either:
            // the device is signed out again, as well as that can be done.
            let _ = client.delete_device(&answer.token, &answer.device.id, false);
            return Err(Error::Keychain(why));
        }
        // What was sent under another token says nothing of this device's
        // rows: the next sync sends them all.
        sync::forget(&self.inner.data_dir);
        Ok(SignedIn {
            account: answer.account,
            device: answer.device,
        })
    }
}

/// 32 bytes from the system's random source, in base64url: 43 characters of
/// `[A-Za-z0-9-_]`, what RFC 7636 takes for a verifier and the protocol for a
/// state.
fn random_text() -> Result<String, Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|err| Error::Local(format!("the system's random source failed: {err}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The S256 challenge of a verifier: `base64url(sha256(verifier))`, unpadded.
pub(super) fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Serves the loopback port until the visit that answers this sign-in, and
/// gives its link code.
fn listen(
    listener: TcpListener,
    state: &str,
    cancel: &Cancel,
    deadline: Instant,
) -> Result<String, Error> {
    loop {
        if cancel.asked() {
            return Err(Error::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(Error::TimedOut);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(outcome) = visit(stream, state) {
                    return outcome;
                }
            }
            // Nobody yet (`WouldBlock`), or a connection that died before it
            // was taken: neither ends the sign-in.
            Err(_) => std::thread::sleep(TICK),
        }
    }
}

/// What one visit to the listener came to.
enum Visit {
    /// The answer to this sign-in, with its link code.
    Code(String),
    /// The answer to this sign-in: it failed before coming back.
    Failed(Error),
    /// The callback, without this sign-in's state: somebody else's, a stale
    /// tab, or a guess. The sign-in goes on waiting.
    Stranger,
    /// Anything else a browser or a scanner asks a port for.
    Elsewhere,
}

/// Answers one visitor. `Some` when the visit ends the sign-in.
fn visit(mut stream: TcpStream, state: &str) -> Option<Result<String, Error>> {
    // An accepted socket inherits the listener's non-blocking mode on some
    // systems; a visit is read and answered in order, within its time.
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(VISIT_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(VISIT_TIMEOUT)).ok()?;
    let head = read_head(&mut stream)?;
    let french = prefers_french(header(&head, "accept-language").unwrap_or(""));
    let visit = classify(&head, state);
    let (status, text) = match &visit {
        Visit::Code(_) => ("200 OK", Page::Done),
        Visit::Failed(_) => ("200 OK", Page::NotDone),
        Visit::Stranger => ("400 Bad Request", Page::Stranger),
        Visit::Elsewhere => ("404 Not Found", Page::Elsewhere),
    };
    let html = page(text, french);
    let answer = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
         Connection: close\r\n\r\n{html}",
        html.len()
    );
    // A browser that left before its page arrived changes nothing to what
    // its visit carried.
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
    match visit {
        Visit::Code(code) => Some(Ok(code)),
        Visit::Failed(why) => Some(Err(why)),
        Visit::Stranger | Visit::Elsewhere => None,
    }
}

/// The request's head, up to the blank line. `None` for a visitor that sent
/// none in time, or more than a head's worth.
fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            head.truncate(end);
            return String::from_utf8(head).ok();
        }
        if head.len() > MAX_HEAD_BYTES {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
}

/// The value of a header of the head, whatever its case.
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn classify(head: &str, state: &str) -> Visit {
    let mut request = head.lines().next().unwrap_or("").split_whitespace();
    let (Some("GET"), Some(target)) = (request.next(), request.next()) else {
        return Visit::Elsewhere;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != paths::LOOPBACK_CALLBACK {
        return Visit::Elsewhere;
    }
    let (mut code, mut error, mut given) = (None, None, None);
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let slot = match name {
            "code" => &mut code,
            "error" => &mut error,
            "state" => &mut given,
            _ => continue,
        };
        *slot = percent_encoding::percent_decode_str(&value.replace('+', " "))
            .decode_utf8()
            .ok()
            .map(|text| text.into_owned());
    }
    // In constant time: the answer must not tell a guesser how much of the
    // state it got right.
    let ours = given.is_some_and(|given| bool::from(given.as_bytes().ct_eq(state.as_bytes())));
    if !ours {
        return Visit::Stranger;
    }
    match (code, error) {
        (Some(code), None) if !code.is_empty() => Visit::Code(code),
        (_, Some(error)) if error == "access_denied" => Visit::Failed(Error::Denied),
        (_, Some(_)) => Visit::Failed(Error::Provider),
        // The right state and nothing with it: not an answer the server
        // gives. The sign-in keeps waiting for one.
        _ => Visit::Stranger,
    }
}

/// Whether the browser asks for French before English: the language of its
/// highest `q`, the first named when they tie.
fn prefers_french(accept_language: &str) -> bool {
    let mut best: Option<(f32, bool)> = None;
    for item in accept_language.split(',') {
        let mut parts = item.split(';');
        let tag = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let language = tag.split('-').next().unwrap_or("");
        let french = match language {
            "fr" => true,
            "en" => false,
            _ => continue,
        };
        let weight = parts
            .find_map(|part| part.trim().strip_prefix("q=")?.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        if weight > 0.0 && best.is_none_or(|(held, _)| weight > held) {
            best = Some((weight, french));
        }
    }
    best.is_some_and(|(_, french)| french)
}

#[derive(Clone, Copy)]
enum Page {
    Done,
    NotDone,
    Stranger,
    Elsewhere,
}

/// The page a visitor gets: one sentence, its own few styles, and nothing
/// fetched from anywhere.
fn page(page: Page, french: bool) -> String {
    let (lang, text) = match (page, french) {
        (Page::Done, false) => ("en", "You can close this tab and return to Hangar."),
        (Page::Done, true) => ("fr", "Vous pouvez fermer cet onglet et revenir à Hangar."),
        (Page::NotDone, false) => (
            "en",
            "The sign-in was not completed. You can close this tab and return to Hangar.",
        ),
        (Page::NotDone, true) => (
            "fr",
            "La connexion n'a pas abouti. Vous pouvez fermer cet onglet et revenir à Hangar.",
        ),
        (Page::Stranger, false) => (
            "en",
            "This is not the sign-in Hangar is waiting for. Start again from Hangar.",
        ),
        (Page::Stranger, true) => (
            "fr",
            "Ce n'est pas la connexion que Hangar attend. Recommencez depuis Hangar.",
        ),
        (Page::Elsewhere, false) => ("en", "There is nothing here."),
        (Page::Elsewhere, true) => ("fr", "Il n'y a rien ici."),
    };
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Hangar</title><style>:root{{color-scheme:light dark}}\
         body{{margin:0;min-height:100vh;display:grid;place-items:center;\
         font:16px/1.5 system-ui,sans-serif}}p{{margin:24px;max-width:28em;text-align:center}}\
         </style></head><body><p>{text}</p></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verifier_and_the_state_are_what_the_protocol_takes() {
        let verifier = random_text().unwrap();
        let length = verifier.chars().count();
        assert!((api::MIN_VERIFIER_CHARS..=api::MAX_VERIFIER_CHARS).contains(&length));
        assert!(length <= api::MAX_STATE_CHARS);
        assert!(verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(random_text().unwrap(), verifier);
        assert_eq!(challenge_of(&verifier).len(), api::CHALLENGE_CHARS);
    }

    /// RFC 7636, appendix B.
    #[test]
    fn the_challenge_is_the_rfc_s_s256() {
        assert_eq!(
            challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    fn get(target: &str) -> String {
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1")
    }

    #[test]
    fn only_the_callback_with_this_sign_in_s_state_is_an_answer() {
        assert!(matches!(
            classify(&get("/callback?code=abc&state=s3cret"), "s3cret"),
            Visit::Code(code) if code == "abc"
        ));
        // Escaped as a server would escape them.
        assert!(matches!(
            classify(&get("/callback?state=a%2Db&code=x%20y"), "a-b"),
            Visit::Code(code) if code == "x y"
        ));
        for stranger in [
            "/callback?code=abc&state=other",
            "/callback?code=abc&state=s3cre",
            "/callback?code=abc&state=s3cretx",
            "/callback?code=abc&state=",
            "/callback?code=abc",
            "/callback",
            // The right state with nothing to act on.
            "/callback?state=s3cret",
            "/callback?state=s3cret&code=",
        ] {
            assert!(
                matches!(classify(&get(stranger), "s3cret"), Visit::Stranger),
                "{stranger}"
            );
        }
        for elsewhere in ["/", "/favicon.ico", "/callback/x?state=s3cret&code=abc"] {
            assert!(
                matches!(classify(&get(elsewhere), "s3cret"), Visit::Elsewhere),
                "{elsewhere}"
            );
        }
        assert!(matches!(
            classify("POST /callback?code=abc&state=s3cret HTTP/1.1", "s3cret"),
            Visit::Elsewhere
        ));
    }

    #[test]
    fn a_refusal_at_the_provider_ends_the_sign_in_only_with_the_right_state() {
        assert!(matches!(
            classify(&get("/callback?error=access_denied&state=s"), "s"),
            Visit::Failed(Error::Denied)
        ));
        assert!(matches!(
            classify(&get("/callback?error=provider_error&state=s"), "s"),
            Visit::Failed(Error::Provider)
        ));
        assert!(matches!(
            classify(&get("/callback?error=access_denied&state=x"), "s"),
            Visit::Stranger
        ));
    }

    #[test]
    fn the_page_is_french_for_a_browser_that_asks_for_it_first() {
        assert!(prefers_french("fr"));
        assert!(prefers_french("fr-FR,fr;q=0.9,en;q=0.8"));
        assert!(prefers_french("en;q=0.5, fr-CA;q=0.7"));
        assert!(prefers_french("de, fr;q=0.4"));
        assert!(!prefers_french(""));
        assert!(!prefers_french("en-US,en;q=0.9,fr;q=0.8"));
        assert!(!prefers_french("de-DE,de;q=0.9"));
        assert!(!prefers_french("en, fr"));
        assert!(!prefers_french("fr;q=0, en;q=0.1"));
    }

    #[test]
    fn every_page_stands_alone() {
        for kind in [Page::Done, Page::NotDone, Page::Stranger, Page::Elsewhere] {
            for french in [false, true] {
                let html = page(kind, french);
                for outside in ["http:", "https:", "//", "src=", "href=", "url(", "@import"] {
                    assert!(!html.contains(outside), "{outside} in {html}");
                }
                assert!(html.contains(if french { "lang=\"fr\"" } else { "lang=\"en\"" }));
            }
        }
        assert!(page(Page::Done, false).contains("You can close this tab and return to Hangar."));
        assert!(
            page(Page::Done, true).contains("Vous pouvez fermer cet onglet et revenir à Hangar.")
        );
    }

    #[test]
    fn headers_are_found_whatever_their_case() {
        let head = "GET / HTTP/1.1\r\nHost: x\r\nACCEPT-language:  fr-FR \r\n";
        assert_eq!(header(head, "accept-language"), Some("fr-FR"));
        assert_eq!(header(head, "cookie"), None);
    }
}
