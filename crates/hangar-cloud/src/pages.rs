//! The pages a browser sees when a sign-in cannot go on, and cannot be sent
//! back to the desktop either (docs/native/cloud.md, "Signing in").
//!
//! One self-contained page, in English or French by `Accept-Language`. It
//! loads nothing (no script, no image, no font, no link to follow) and its
//! policy allows nothing but its own style sheet, named by its hash. Nothing
//! of the request is ever written into it: every word is one of this file's.

use std::sync::OnceLock;

use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

/// Why the sign-in stopped here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// The link that opened the sign-in is not one Hangar makes.
    Invalid,
    /// This server has no client for the provider asked for.
    Unavailable,
    /// The sign-in is unknown to the server: older than its ten minutes,
    /// already finished, or never started here.
    Expired,
    /// The person said no at the provider, for a sign-in the server no
    /// longer knows.
    Denied,
    /// Too many sign-ins from this address.
    Busy,
    /// The provider's fault, or the server's.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }
}

/// English or French, whichever `Accept-Language` prefers; English when it
/// names neither, or is not there.
pub fn language(headers: &HeaderMap) -> Lang {
    let mut best: Option<(Lang, f32)> = None;
    let ranges = headers
        .get_all(header::ACCEPT_LANGUAGE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','));
    for range in ranges {
        let mut parts = range.split(';');
        let tag = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let lang = match tag.split('-').next() {
            Some("fr") => Lang::Fr,
            Some("en") => Lang::En,
            _ => continue,
        };
        let quality = parts
            .find_map(|param| param.trim().strip_prefix("q="))
            .map_or(1.0, |q| q.trim().parse::<f32>().unwrap_or(0.0));
        // The first of two equal preferences wins, as it was written first.
        if quality > 0.0 && best.is_none_or(|(_, q)| quality > q) {
            best = Some((lang, quality));
        }
    }
    best.map_or(Lang::En, |(lang, _)| lang)
}

impl Page {
    fn status(self) -> StatusCode {
        match self {
            Page::Invalid | Page::Expired | Page::Denied => StatusCode::BAD_REQUEST,
            Page::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Page::Busy => StatusCode::TOO_MANY_REQUESTS,
            Page::Failed => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The title and the sentence under it.
    fn words(self, lang: Lang) -> (&'static str, &'static str) {
        match (self, lang) {
            (Page::Invalid, Lang::En) => (
                "This sign-in link is not valid",
                "The link that opened this page is incomplete or malformed. Start the sign-in \
                 again from Hangar.",
            ),
            (Page::Invalid, Lang::Fr) => (
                "Ce lien de connexion n\u{2019}est pas valide",
                "Le lien qui a ouvert cette page est incomplet ou mal formé. Relancez la \
                 connexion depuis Hangar.",
            ),
            (Page::Unavailable, Lang::En) => (
                "This sign-in method is not available",
                "This server is not set up to sign in with this provider. Try the other one \
                 from Hangar.",
            ),
            (Page::Unavailable, Lang::Fr) => (
                "Cette méthode de connexion n\u{2019}est pas disponible",
                "Ce serveur n\u{2019}est pas configuré pour ce fournisseur. Essayez l\u{2019}autre \
                 depuis Hangar.",
            ),
            (Page::Expired, Lang::En) => (
                "This sign-in has expired",
                "A sign-in must be finished within ten minutes, and its link works once. \
                 Start it again from Hangar.",
            ),
            (Page::Expired, Lang::Fr) => (
                "Cette connexion a expiré",
                "Une connexion doit être terminée en dix minutes, et son lien ne sert \
                 qu\u{2019}une fois. Relancez-la depuis Hangar.",
            ),
            (Page::Denied, Lang::En) => (
                "Sign-in cancelled",
                "You declined the sign-in, and nothing was shared with Hangar. You can start \
                 it again from Hangar whenever you like.",
            ),
            (Page::Denied, Lang::Fr) => (
                "Connexion annulée",
                "Vous avez refusé la connexion, et rien n\u{2019}a été partagé avec Hangar. \
                 Vous pouvez la relancer depuis Hangar quand vous voulez.",
            ),
            (Page::Busy, Lang::En) => (
                "Too many sign-ins",
                "Too many sign-ins were started in a short time. Wait a minute, then start \
                 again from Hangar.",
            ),
            (Page::Busy, Lang::Fr) => (
                "Trop de connexions",
                "Trop de connexions ont été lancées en peu de temps. Patientez une minute, \
                 puis relancez depuis Hangar.",
            ),
            (Page::Failed, Lang::En) => (
                "The sign-in could not be completed",
                "Something went wrong between the provider and the Hangar account server. \
                 Nothing was changed. Start the sign-in again from Hangar in a moment.",
            ),
            (Page::Failed, Lang::Fr) => (
                "La connexion n\u{2019}a pas pu aboutir",
                "Un problème est survenu entre le fournisseur et le serveur du compte Hangar. \
                 Rien n\u{2019}a été modifié. Relancez la connexion depuis Hangar dans un instant.",
            ),
        }
    }

    /// The page's HTML. Every word is a constant of this file, none of which
    /// holds a character HTML reads as markup.
    pub fn html(self, lang: Lang) -> String {
        let (title, text) = self.words(lang);
        let close = match lang {
            Lang::En => "You can close this window.",
            Lang::Fr => "Vous pouvez fermer cette fenêtre.",
        };
        let name = match lang {
            Lang::En => "Hangar account",
            Lang::Fr => "Compte Hangar",
        };
        format!(
            "<!doctype html>\n<html lang=\"{code}\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <meta name=\"robots\" content=\"noindex\">\n\
             <meta name=\"color-scheme\" content=\"dark light\">\n\
             <title>{title} · {name}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n\
             <p class=\"brand\">{name}</p>\n<h1>{title}</h1>\n<p>{text}</p>\n\
             <p class=\"close\">{close}</p>\n</main>\n</body>\n</html>\n",
            code = lang.code(),
        )
    }

    /// The page as an answer, in the language `headers` asks for.
    pub fn response(self, headers: &HeaderMap) -> Response {
        let lang = language(headers);
        let mut response = (self.status(), self.html(lang)).into_response();
        let out = response.headers_mut();
        out.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        out.insert(
            header::CONTENT_LANGUAGE,
            HeaderValue::from_static(lang.code()),
        );
        out.insert(header::VARY, HeaderValue::from_static("Accept-Language"));
        // The router's policy for JSON allows nothing at all; this one allows
        // the page's own style sheet, by its hash, and still nothing else.
        if let Ok(policy) = HeaderValue::from_str(content_security_policy()) {
            out.insert(header::CONTENT_SECURITY_POLICY, policy);
        }
        out.insert(
            HeaderName::from_static("x-robots-tag"),
            HeaderValue::from_static("noindex"),
        );
        if self == Page::Busy {
            out.insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}

/// The colours are Graphite's and GitHub Light's, the app's default themes.
const STYLE: &str = "\
:root{color-scheme:dark light;--bg:#202124;--card:#292a2d;--line:#3c4043;--fg:#e8eaed;\
--dim:#9aa0a6;--accent:#8ab4f8}\
@media (prefers-color-scheme:light){:root{--bg:#f0f0f0;--card:#fff;--line:#d0d7de;\
--fg:#1f2328;--dim:#59636e;--accent:#0969da}}\
*{box-sizing:border-box}\
body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px;\
background:var(--bg);color:var(--fg);\
font:16px/1.5 Inter,system-ui,-apple-system,\"Segoe UI\",Roboto,sans-serif}\
main{width:100%;max-width:440px;padding:28px;background:var(--card);\
border:1px solid var(--line);border-radius:10px}\
.brand{margin:0 0 20px;padding-left:10px;border-left:3px solid var(--accent);\
font-size:13px;font-weight:600;color:var(--dim)}\
h1{margin:0 0 12px;font-size:20px;line-height:1.3;font-weight:600}\
p{margin:0 0 12px}\
.close{margin:20px 0 0;font-size:14px;color:var(--dim)}";

/// Nothing is loaded, nothing is run, nothing is framed, no form is sent, and
/// the only style applied is the one whose SHA-256 this names.
fn content_security_policy() -> &'static str {
    static POLICY: OnceLock<String> = OnceLock::new();
    POLICY.get_or_init(|| {
        format!(
            "default-src 'none'; style-src 'sha256-{}'; base-uri 'none'; form-action 'none'; \
             frame-ancestors 'none'",
            STANDARD.encode(Sha256::digest(STYLE.as_bytes()))
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGES: [Page; 6] = [
        Page::Invalid,
        Page::Unavailable,
        Page::Expired,
        Page::Denied,
        Page::Busy,
        Page::Failed,
    ];

    fn accept(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_str(value).unwrap(),
        );
        headers
    }

    #[test]
    fn the_language_is_the_preferred_of_english_and_french() {
        assert_eq!(language(&HeaderMap::new()), Lang::En);
        for (value, lang) in [
            ("fr", Lang::Fr),
            ("fr-FR,fr;q=0.9,en;q=0.8", Lang::Fr),
            ("en-US,en;q=0.9,fr;q=0.8", Lang::En),
            ("de,fr;q=0.5,en;q=0.7", Lang::En),
            ("de, FR-ca;q=0.5", Lang::Fr),
            ("de,es", Lang::En),
            ("fr;q=0,en;q=0.1", Lang::En),
            ("fr;q=0", Lang::En),
            ("*", Lang::En),
            ("fr;q=nonsense", Lang::En),
        ] {
            assert_eq!(language(&accept(value)), lang, "{value}");
        }
    }

    #[test]
    fn every_page_is_self_contained_in_both_languages() {
        for page in PAGES {
            for lang in [Lang::En, Lang::Fr] {
                let html = page.html(lang);
                assert!(html.starts_with("<!doctype html>"), "{page:?}");
                assert!(html.contains(&format!("<html lang=\"{}\">", lang.code())));
                // Nothing that loads, runs or leads anywhere.
                for forbidden in [
                    "<script", "<link", "<img", "<a ", "<iframe", "<form", "http:", "https:",
                    "url(", "@import", "src=", "href=", " style=", "onload", "onclick",
                ] {
                    assert!(!html.contains(forbidden), "{page:?} {lang:?}: {forbidden}");
                }
                // The words hold nothing HTML would read as markup.
                let (title, text) = page.words(lang);
                for words in [title, text] {
                    assert!(!words.contains(['<', '>', '&', '"']), "{words}");
                }
                assert_eq!(html.matches("<style>").count(), 1);
            }
        }
    }

    #[test]
    fn the_two_languages_do_not_say_the_same_words() {
        for page in PAGES {
            assert_ne!(page.words(Lang::En), page.words(Lang::Fr), "{page:?}");
        }
    }

    #[test]
    fn the_policy_allows_the_page_s_own_style_and_nothing_else() {
        let response = Page::Expired.response(&accept("fr"));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(headers[header::CONTENT_LANGUAGE], "fr");
        assert_eq!(headers[header::VARY], "Accept-Language");
        let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        let hash = STANDARD.encode(Sha256::digest(STYLE.as_bytes()));
        assert_eq!(
            policy,
            format!(
                "default-src 'none'; style-src 'sha256-{hash}'; base-uri 'none'; \
                 form-action 'none'; frame-ancestors 'none'"
            )
        );
        assert!(!policy.contains("unsafe"));
    }

    #[test]
    fn each_page_has_the_status_of_what_went_wrong() {
        let status = |page: Page| page.response(&HeaderMap::new()).status().as_u16();
        assert_eq!(status(Page::Invalid), 400);
        assert_eq!(status(Page::Expired), 400);
        assert_eq!(status(Page::Denied), 400);
        assert_eq!(status(Page::Unavailable), 503);
        assert_eq!(status(Page::Busy), 429);
        assert_eq!(status(Page::Failed), 500);
        let busy = Page::Busy.response(&HeaderMap::new());
        assert_eq!(busy.headers()[header::RETRY_AFTER], "60");
    }
}
