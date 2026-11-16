//! The sign-in from end to end, as a desktop and a browser go through it:
//! the start, the provider (the stand-in of `fake.rs`, over real HTTP), the
//! callback, the loopback redirect, the exchange, and the token at work.
//! The tests play the browser: they follow each `Location` by hand, which is
//! also how they see exactly where the server sends people.

use std::collections::HashMap;

use axum::extract::Query;
use axum::http::Uri;

use super::fake::{self, Person};
use super::*;

/// A Hangar waiting on its loopback port for a sign-in to come back.
struct Desktop {
    port: u16,
    state: &'static str,
    verifier: &'static str,
    machine: &'static str,
}

impl Desktop {
    fn new() -> Self {
        Self {
            port: 49_152,
            // Every kind of character a state may hold.
            state: "desk-state_1.~A",
            verifier: VERIFIER,
            machine: "Atelier",
        }
    }

    fn start_path(&self, provider: &str) -> String {
        format!(
            "{}?provider={provider}&port={}&state={}&challenge={}&machine={}",
            paths::AUTH_START,
            self.port,
            self.state,
            challenge(self.verifier),
            self.machine
        )
    }
}

impl Answer {
    /// Where a `302` sends the browser.
    #[track_caller]
    fn location(&self) -> String {
        assert_eq!(
            self.status,
            StatusCode::FOUND,
            "{}",
            String::from_utf8_lossy(&self.body)
        );
        self.headers[header::LOCATION].to_str().unwrap().to_string()
    }

    /// The answer is one of the server's pages, with `status`, saying
    /// `words`, and sends the browser nowhere.
    #[track_caller]
    fn is_page(&self, status: u16, words: &str) {
        let body = String::from_utf8_lossy(&self.body);
        assert_eq!(self.status.as_u16(), status, "{body}");
        assert_eq!(
            self.headers[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert!(!self.headers.contains_key(header::LOCATION));
        assert!(body.contains(words), "{body}");
    }

    /// The answer sends the browser to the desktop's listener and nowhere
    /// else: `http://127.0.0.1:<port>/callback`. Gives the query's fields.
    #[track_caller]
    fn to_loopback(&self, desktop: &Desktop) -> HashMap<String, String> {
        let location = self.location();
        let prefix = format!("http://127.0.0.1:{}/callback?", desktop.port);
        assert!(location.starts_with(&prefix), "{location}");
        let fields = query_of(&location);
        assert_eq!(fields["state"], desktop.state);
        fields
    }

    /// The loopback redirect carries `error` and no code.
    #[track_caller]
    fn is_loopback_error(&self, desktop: &Desktop, error: &str) {
        let fields = self.to_loopback(desktop);
        assert_eq!(fields.get("error").map(String::as_str), Some(error));
        assert!(!fields.contains_key("code"));
        assert_eq!(fields.len(), 2, "{fields:?}");
    }
}

/// The decoded fields of a URL's query.
fn query_of(url: &str) -> HashMap<String, String> {
    let uri: Uri = url.parse().unwrap();
    Query::<HashMap<String, String>>::try_from_uri(&uri)
        .unwrap()
        .0
}

/// The path and query of an address of the server under test.
#[track_caller]
fn on_server(url: &str) -> &str {
    url.strip_prefix(BASE_URL)
        .unwrap_or_else(|| panic!("{url} is not on the server"))
}

async fn browse(server: &Server, path: &str) -> Answer {
    server.call(Method::GET, path, None, None).await
}

/// The browser at the provider's consent page, logged in there as `who`:
/// a real request to the stand-in. Gives where the provider sends it back.
async fn consent(authorize_url: &str, who: &str) -> String {
    let url = format!("{authorize_url}&login_as={who}");
    tokio::task::spawn_blocking(move || {
        let agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut response = agent.get(&url).call().unwrap();
        let body = response.body_mut().read_to_string().unwrap_or_default();
        assert_eq!(response.status().as_u16(), 302, "{body}");
        response.headers()["location"].to_str().unwrap().to_string()
    })
    .await
    .unwrap()
}

/// Start, consent: the address the provider sends the browser back to.
async fn back_from_provider(
    server: &Server,
    provider: &str,
    who: &str,
    desktop: &Desktop,
) -> String {
    let started = browse(server, &desktop.start_path(provider)).await;
    consent(&started.location(), who).await
}

/// Start, consent, callback: the server's answer to the browser coming back.
async fn arrive(server: &Server, provider: &str, who: &str, desktop: &Desktop) -> Answer {
    let back = back_from_provider(server, provider, who, desktop).await;
    browse(server, on_server(&back)).await
}

/// The whole sign-in of machine `machine_id`, to the exchange's answer.
async fn sign_in_as(
    server: &Server,
    provider: &str,
    who: &str,
    machine_id: &str,
) -> ExchangeResponse {
    let desktop = Desktop::new();
    let fields = arrive(server, provider, who, &desktop)
        .await
        .to_loopback(&desktop);
    let answer = server
        .exchange(
            &fields["code"],
            desktop.verifier,
            machine(machine_id, machine_id),
        )
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    answer.json()
}

async fn rows(server: &Server, sql: &'static str) -> i64 {
    server
        .state
        .db
        .call(move |conn| Ok(conn.query_row(sql, [], |row| row.get(0))?))
        .await
        .unwrap()
}

/// GitHub's two answers about somebody.
fn github(id: u64, login: &str, name: Option<&str>, emails: Value) -> Person {
    Person {
        github_user: json!({
            "id": id,
            "login": login,
            "name": name,
            "avatar_url": format!("https://avatars.test/u/{id}"),
            // The public address, which says nothing of who verified what.
            "email": "public@elsewhere.test",
        }),
        github_emails: emails,
        ..Person::default()
    }
}

fn github_email(email: &str, primary: bool, verified: bool) -> Value {
    json!({ "email": email, "primary": primary, "verified": verified, "visibility": null })
}

/// Google's answer about somebody.
fn google(sub: &str, name: &str, email: &str, verified: bool) -> Person {
    Person {
        google_userinfo: json!({
            "sub": sub,
            "name": name,
            "picture": format!("https://photos.test/{sub}"),
            "email": email,
            "email_verified": verified,
        }),
        ..Person::default()
    }
}

/// Ada, with an account at both providers under the same verified address.
fn ada() -> Person {
    Person {
        google_userinfo: google("g-ada", "Ada L.", "ada@example.org", true).google_userinfo,
        ..github(
            1815,
            "ada",
            Some("Ada Lovelace"),
            json!([
                github_email("old@example.org", false, true),
                github_email("ada@example.org", true, true),
            ]),
        )
    }
}

// ------------------------------------------------------------- the whole way

#[tokio::test]
async fn a_sign_in_with_github_ends_with_a_token_that_works() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();

    // The start sends the browser to GitHub's consent page.
    let started = browse(&server, &desktop.start_path("github")).await;
    let authorize = started.location();
    assert!(
        authorize.starts_with(&format!("{}/github/authorize?", fake::provider().base())),
        "{authorize}"
    );
    let asked = query_of(&authorize);
    assert_eq!(asked["response_type"], "code");
    assert_eq!(asked["client_id"], fake::GITHUB_CLIENT.0);
    assert_eq!(
        asked["redirect_uri"],
        "https://account.test/v1/auth/callback/github"
    );
    assert_eq!(asked["scope"], "read:user user:email");
    assert_eq!(asked["code_challenge_method"], "S256");
    assert_eq!(asked.len(), 7, "{asked:?}");
    // The server's own state and challenge: nothing of the desktop's, and
    // never the secret, goes to the provider through the browser.
    assert_eq!(asked["state"].len(), 43);
    assert_eq!(asked["code_challenge"].len(), 43);
    for private in [
        desktop.state.to_string(),
        challenge(desktop.verifier),
        desktop.port.to_string(),
        desktop.machine.to_string(),
        fake::GITHUB_CLIENT.1.to_string(),
    ] {
        assert!(!authorize.contains(&private), "{authorize}");
    }
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 1);

    // The person consents; GitHub sends the browser back to the server.
    let back = consent(&authorize, &who).await;
    assert!(back.starts_with("https://account.test/v1/auth/callback/github?code="));

    // The server sends it on to the desktop's listener, with a link code.
    let arrived = browse(&server, on_server(&back)).await;
    let fields = arrived.to_loopback(&desktop);
    assert_eq!(fields.len(), 2, "{fields:?}");
    assert_eq!(fields["code"].len(), 43);
    assert_eq!(arrived.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(arrived.headers[header::REFERRER_POLICY], "no-referrer");
    // The sign-in in progress is gone, and the code was exchanged once.
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 0);
    assert_eq!(fake::provider().exchanges(&who), 1);

    // The desktop exchanges the code with its verifier.
    let answer = server
        .exchange(&fields["code"], desktop.verifier, machine("m1", "Atelier"))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    let signed_in: ExchangeResponse = answer.json();
    assert!(signed_in.token.starts_with("hgr_"));
    assert_eq!(signed_in.account.display_name, "Ada Lovelace");
    // The primary verified address: not the first, not the public one.
    assert_eq!(signed_in.account.email.as_deref(), Some("ada@example.org"));
    assert_eq!(
        signed_in.account.avatar_url.as_deref(),
        Some("https://avatars.test/u/1815")
    );
    assert_eq!(signed_in.account.providers, [Provider::Github]);

    // And the token works.
    let me = server.get(paths::ME, &signed_in.token).await;
    assert_eq!(me.status, StatusCode::OK);
    let me: MeResponse = me.json();
    assert_eq!(me.account, signed_in.account);
    assert_eq!(me.device.name, "Atelier");
}

#[tokio::test]
async fn a_sign_in_with_google_ends_with_a_token_that_works() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();

    let started = browse(&server, &desktop.start_path("google")).await;
    let authorize = started.location();
    assert!(
        authorize.starts_with(&format!("{}/google/authorize?", fake::provider().base())),
        "{authorize}"
    );
    let asked = query_of(&authorize);
    assert_eq!(asked["client_id"], fake::GOOGLE_CLIENT.0);
    assert_eq!(
        asked["redirect_uri"],
        "https://account.test/v1/auth/callback/google"
    );
    assert_eq!(asked["scope"], "openid email profile");
    assert!(!authorize.contains(fake::GOOGLE_CLIENT.1));

    let back = consent(&authorize, &who).await;
    let fields = browse(&server, on_server(&back))
        .await
        .to_loopback(&desktop);
    let answer = server
        .exchange(&fields["code"], desktop.verifier, machine("m1", "Atelier"))
        .await;
    let signed_in: ExchangeResponse = answer.json();
    assert_eq!(signed_in.account.display_name, "Ada L.");
    assert_eq!(signed_in.account.email.as_deref(), Some("ada@example.org"));
    assert_eq!(
        signed_in.account.avatar_url.as_deref(),
        Some("https://photos.test/g-ada")
    );
    assert_eq!(signed_in.account.providers, [Provider::Google]);
    let me = server.get(paths::ME, &signed_in.token).await;
    assert_eq!(me.status, StatusCode::OK);
}

#[tokio::test]
async fn the_link_code_is_only_good_with_the_verifier_of_the_desktop_that_started() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let fields = arrive(&server, "github", &who, &desktop)
        .await
        .to_loopback(&desktop);
    server
        .exchange(&fields["code"], OTHER_VERIFIER, machine("m1", "Thief"))
        .await
        .refused(ErrorCode::InvalidCode);
}

#[tokio::test]
async fn a_sign_in_may_take_ten_minutes_and_its_code_then_lives_five() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    // Nine minutes on the consent page.
    server.advance(9 * MINUTE);
    let fields = browse(&server, on_server(&back))
        .await
        .to_loopback(&desktop);
    server.advance(5 * MINUTE - 1);
    let answer = server
        .exchange(&fields["code"], desktop.verifier, machine("m1", "Atelier"))
        .await;
    assert_eq!(answer.status, StatusCode::OK);

    // The same, a millisecond later: the code is dead.
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    let fields = browse(&server, on_server(&back))
        .await
        .to_loopback(&desktop);
    server.advance(5 * MINUTE);
    server
        .exchange(&fields["code"], desktop.verifier, machine("m1", "Atelier"))
        .await
        .refused(ErrorCode::InvalidCode);
}

// ------------------------------------------------------- whose account it is

#[tokio::test]
async fn signing_in_again_is_the_same_account_with_today_s_profile() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let first = sign_in_as(&server, "github", &who, "m1").await;

    // Ada renames herself and moves to another address at GitHub.
    fake::provider().update(
        &who,
        github(
            1815,
            "ada",
            Some("Ada King"),
            json!([github_email("countess@example.org", true, true)]),
        ),
    );
    let again = sign_in_as(&server, "github", &who, "m2").await;
    assert_eq!(again.account.id, first.account.id);
    assert_eq!(again.account.display_name, "Ada King");
    assert_eq!(again.account.email.as_deref(), Some("countess@example.org"));
    assert_eq!(rows(&server, "SELECT count(*) FROM accounts").await, 1);
    assert_eq!(server.devices(&again.token).await.len(), 2);
}

#[tokio::test]
async fn the_same_verified_email_through_the_other_provider_is_the_same_account() {
    for (first, second) in [("github", "google"), ("google", "github")] {
        let server = Server::new();
        let who = fake::provider().add(ada());
        let one = sign_in_as(&server, first, &who, "m1").await;
        let two = sign_in_as(&server, second, &who, "m2").await;
        assert_eq!(two.account.id, one.account.id, "{first} then {second}");
        assert_eq!(two.account.providers.len(), 2);
        assert_eq!(
            two.account.providers[0].as_str(),
            first,
            "the first identity first"
        );
        // The account shows the profile of its first identity.
        assert_eq!(two.account.display_name, one.account.display_name);
        assert_eq!(rows(&server, "SELECT count(*) FROM accounts").await, 1);
        assert_eq!(rows(&server, "SELECT count(*) FROM identities").await, 2);
        // Both machines are the account's, whichever provider signed them in.
        assert_eq!(server.devices(&one.token).await.len(), 2);
    }
}

#[tokio::test]
async fn the_case_of_an_address_does_not_make_it_another() {
    let server = Server::new();
    let who = fake::provider().add(Person {
        google_userinfo: google("g-ada", "Ada", "Ada@Example.ORG", true).google_userinfo,
        ..ada()
    });
    let one = sign_in_as(&server, "github", &who, "m1").await;
    let two = sign_in_as(&server, "google", &who, "m2").await;
    assert_eq!(two.account.id, one.account.id);
}

#[tokio::test]
async fn an_address_the_new_provider_did_not_verify_joins_nothing() {
    // Google does not vouch for the address.
    let server = Server::new();
    let who = fake::provider().add(Person {
        google_userinfo: google("g-eve", "Eve", "ada@example.org", false).google_userinfo,
        ..ada()
    });
    let ada_s = sign_in_as(&server, "github", &who, "m1").await;
    let eve_s = sign_in_as(&server, "google", &who, "m2").await;
    assert_ne!(eve_s.account.id, ada_s.account.id);
    assert_eq!(eve_s.account.providers, [Provider::Google]);
    // An address nobody verified is not kept at all.
    assert_eq!(eve_s.account.email, None);
    assert_eq!(ada_s.account.providers, [Provider::Github]);

    // `email_verified` left out, or anything but true, is not verified.
    for verified in [json!(null), json!("false"), json!("yes"), json!(1)] {
        let server = Server::new();
        let mut userinfo = google("g-eve", "Eve", "ada@example.org", true).google_userinfo;
        userinfo["email_verified"] = verified.clone();
        let who = fake::provider().add(Person {
            google_userinfo: userinfo,
            ..ada()
        });
        let ada_s = sign_in_as(&server, "github", &who, "m1").await;
        let eve_s = sign_in_as(&server, "google", &who, "m2").await;
        assert_ne!(eve_s.account.id, ada_s.account.id, "{verified}");
    }

    // GitHub: the address is there and verified but is not the primary one,
    // or is the primary one and not verified.
    for emails in [
        json!([
            github_email("eve@example.org", true, true),
            github_email("ada@example.org", false, true),
        ]),
        json!([github_email("ada@example.org", true, false)]),
        json!([]),
    ] {
        let server = Server::new();
        let who = fake::provider().add(Person {
            github_emails: emails.clone(),
            ..ada()
        });
        let ada_s = sign_in_as(&server, "google", &who, "m1").await;
        let eve_s = sign_in_as(&server, "github", &who, "m2").await;
        assert_ne!(eve_s.account.id, ada_s.account.id, "{emails}");
        assert_ne!(eve_s.account.email.as_deref(), Some("ada@example.org"));
    }
}

#[tokio::test]
async fn an_address_the_first_provider_did_not_verify_is_joined_by_nobody() {
    // The account was made with an address GitHub did not vouch for; Google
    // then vouches for the same one. Not the same person as far as anybody
    // knows.
    let server = Server::new();
    let who = fake::provider().add(Person {
        github_emails: json!([github_email("ada@example.org", true, false)]),
        ..ada()
    });
    let first = sign_in_as(&server, "github", &who, "m1").await;
    assert_eq!(first.account.email, None);
    let second = sign_in_as(&server, "google", &who, "m2").await;
    assert_ne!(second.account.id, first.account.id);
    assert_eq!(rows(&server, "SELECT count(*) FROM accounts").await, 2);

    // The other way round.
    let server = Server::new();
    let who = fake::provider().add(Person {
        google_userinfo: google("g-ada", "Ada", "ada@example.org", false).google_userinfo,
        ..ada()
    });
    let first = sign_in_as(&server, "google", &who, "m1").await;
    let second = sign_in_as(&server, "github", &who, "m2").await;
    assert_ne!(second.account.id, first.account.id);
}

#[tokio::test]
async fn an_address_that_stops_being_verified_stops_linking() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let first = sign_in_as(&server, "github", &who, "m1").await;
    // GitHub no longer vouches for it at the next sign-in.
    fake::provider().update(
        &who,
        Person {
            github_emails: json!([github_email("ada@example.org", true, false)]),
            ..ada()
        },
    );
    let again = sign_in_as(&server, "github", &who, "m1").await;
    assert_eq!(again.account.id, first.account.id);
    assert_eq!(again.account.email, None);
    let other = sign_in_as(&server, "google", &who, "m2").await;
    assert_ne!(other.account.id, first.account.id);
}

#[tokio::test]
async fn a_second_identity_of_the_same_provider_never_joins_an_account() {
    let server = Server::new();
    // Ada, at GitHub only, then at Google too.
    let ada_key = fake::provider().add(ada());
    let ada_s = sign_in_as(&server, "github", &ada_key, "m1").await;
    // Somebody else at GitHub, later, with the address Ada's row still holds.
    let other = fake::provider().add(github(
        2024,
        "newcomer",
        None,
        json!([github_email("ada@example.org", true, true)]),
    ));
    let other_s = sign_in_as(&server, "github", &other, "m2").await;
    assert_ne!(other_s.account.id, ada_s.account.id);
    // Without a name, the login.
    assert_eq!(other_s.account.display_name, "newcomer");

    // Nor through Ada's Google identity, once her account has both.
    let server = Server::new();
    let ada_s = sign_in_as(&server, "github", &ada_key, "m1").await;
    let both = sign_in_as(&server, "google", &ada_key, "m1").await;
    assert_eq!(both.account.id, ada_s.account.id);
    let other_s = sign_in_as(&server, "github", &other, "m2").await;
    assert_ne!(other_s.account.id, ada_s.account.id);
    assert_eq!(other_s.account.providers, [Provider::Github]);
}

// ------------------------------------------------------- the server's state

#[tokio::test]
async fn a_callback_works_once() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    browse(&server, on_server(&back))
        .await
        .to_loopback(&desktop);
    // The same address again: the back button, or somebody who saw it.
    browse(&server, on_server(&back))
        .await
        .is_page(400, "This sign-in has expired");
    assert_eq!(rows(&server, "SELECT count(*) FROM link_codes").await, 1);
}

#[tokio::test]
async fn a_state_the_server_does_not_know_goes_nowhere() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    let code = &query_of(&back)["code"];
    let callback = paths::auth_callback(Provider::Github);
    let drawn = secrets::new_oauth_secret().unwrap();

    for query in [
        // Never drawn by this server, missing, empty, not shaped like one.
        format!("?code={code}&state={drawn}"),
        format!("?code={code}"),
        format!("?code={code}&state="),
        format!("?code={code}&state=short"),
        format!("?code={code}&state={}", "a".repeat(4000)),
        // Nothing at all.
        String::new(),
    ] {
        let answer = browse(&server, &format!("{callback}{query}")).await;
        answer.is_page(400, "This sign-in has expired");
    }
    // None of them reached the provider, or cost the real sign-in its state.
    assert_eq!(fake::provider().exchanges(&who), 0);
    assert_eq!(rows(&server, "SELECT count(*) FROM link_codes").await, 0);
    browse(&server, on_server(&back))
        .await
        .to_loopback(&desktop);
}

#[tokio::test]
async fn a_state_twice_in_the_query_is_not_a_callback() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    let drawn = secrets::new_oauth_secret().unwrap();
    let answer = browse(&server, &format!("{}&state={drawn}", on_server(&back))).await;
    answer.is_page(400, "This sign-in has expired");
    assert_eq!(fake::provider().exchanges(&who), 0);
}

#[tokio::test]
async fn a_sign_in_expires_ten_minutes_after_its_start() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    server.advance(10 * MINUTE);
    browse(&server, on_server(&back))
        .await
        .is_page(400, "This sign-in has expired");
    // The provider's code was not even presented.
    assert_eq!(fake::provider().exchanges(&who), 0);
    assert_eq!(rows(&server, "SELECT count(*) FROM link_codes").await, 0);
    assert_eq!(rows(&server, "SELECT count(*) FROM accounts").await, 0);

    // A millisecond earlier it would have passed.
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    server.advance(10 * MINUTE - 1);
    browse(&server, on_server(&back))
        .await
        .to_loopback(&desktop);
}

#[tokio::test]
async fn sign_ins_nobody_finished_leave_the_database() {
    let server = Server::new();
    let desktop = Desktop::new();
    browse(&server, &desktop.start_path("github")).await;
    browse(&server, &desktop.start_path("google")).await;
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 2);
    server.advance(10 * MINUTE);
    // What the housekeeping runs once a minute.
    let now = server.state.now();
    server
        .state
        .db
        .call(move |conn| auth::purge_expired(conn, now))
        .await
        .unwrap();
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 0);
}

#[tokio::test]
async fn a_state_of_one_provider_is_refused_at_the_other_s_callback_and_spent() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    let at_google = on_server(&back).replace("/callback/github", "/callback/google");
    browse(&server, &at_google)
        .await
        .is_page(400, "This sign-in has expired");
    assert_eq!(fake::provider().exchanges(&who), 0);
    // Spent: the right callback does not find it any more.
    browse(&server, on_server(&back))
        .await
        .is_page(400, "This sign-in has expired");
}

#[tokio::test]
async fn the_database_holds_the_server_s_state_as_its_hash() {
    let server = Server::new();
    let desktop = Desktop::new();
    let started = browse(&server, &desktop.start_path("github")).await;
    let server_state = query_of(&started.location())["state"].clone();
    let stored: (Vec<u8>, i64, String, String, String, i64, i64) = server
        .state
        .db
        .call(|conn| {
            Ok(conn.query_row(
                "SELECT state_hash, port, desktop_state, challenge, machine, created_at, \
                 expires_at FROM auth_states",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )?)
        })
        .await
        .unwrap();
    assert_eq!(stored.0, Sha256::digest(server_state.as_bytes()).to_vec());
    assert_eq!(stored.1, i64::from(desktop.port));
    assert_eq!(stored.2, desktop.state);
    assert_eq!(stored.3, challenge(desktop.verifier));
    assert_eq!(stored.4, desktop.machine);
    assert_eq!(stored.5, T0);
    assert_eq!(stored.6, T0 + 10 * MINUTE);
    let anywhere = rows(
        &server,
        "SELECT count(*) FROM auth_states WHERE provider_verifier = desktop_state \
         OR provider_verifier = challenge",
    )
    .await;
    assert_eq!(anywhere, 0);
}

// ------------------------------------------------- what can go wrong there

#[tokio::test]
async fn a_person_who_says_no_is_sent_back_with_access_denied() {
    let server = Server::new();
    let who = fake::provider().add(Person {
        denies: true,
        ..ada()
    });
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "google", &who, &desktop).await;
    assert!(back.contains("error=access_denied"), "{back}");
    browse(&server, on_server(&back))
        .await
        .is_loopback_error(&desktop, "access_denied");
    assert_eq!(rows(&server, "SELECT count(*) FROM accounts").await, 0);
    assert_eq!(rows(&server, "SELECT count(*) FROM link_codes").await, 0);
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 0);

    // The same answer once more, or for a sign-in the server never started:
    // there is no desktop to tell, so the person is told.
    browse(&server, on_server(&back))
        .await
        .is_page(400, "Sign-in cancelled");
}

#[tokio::test]
async fn a_provider_that_fails_is_said_to_the_desktop_as_provider_error() {
    for (provider, step) in [
        ("github", "token"),
        ("github", "user"),
        ("github", "emails"),
        ("google", "token"),
        ("google", "userinfo"),
    ] {
        let server = Server::new();
        let who = fake::provider().add(Person {
            breaks_at: Some(step),
            ..ada()
        });
        let desktop = Desktop::new();
        let arrived = arrive(&server, provider, &who, &desktop).await;
        arrived.is_loopback_error(&desktop, "provider_error");
        assert_eq!(rows(&server, "SELECT count(*) FROM accounts").await, 0);
        assert_eq!(rows(&server, "SELECT count(*) FROM link_codes").await, 0);
    }
}

#[tokio::test]
async fn an_error_from_the_provider_or_a_code_it_refuses_is_a_provider_error() {
    let callback = |back: &str, query: &str| {
        let state = &query_of(back)["state"];
        format!(
            "{}?state={state}{query}",
            on_server(back).split('?').next().unwrap()
        )
    };
    for provider in ["github", "google"] {
        for query in [
            // The provider's own error, whatever it is called.
            "&error=server_error&error_description=oops",
            "&error=",
            // A code the provider never gave, and none.
            "&code=forged",
            "&code=",
            "",
        ] {
            let server = Server::new();
            let who = fake::provider().add(ada());
            let desktop = Desktop::new();
            let back = back_from_provider(&server, provider, &who, &desktop).await;
            let answer = browse(&server, &callback(&back, query)).await;
            answer.is_loopback_error(&desktop, "provider_error");
            assert_eq!(rows(&server, "SELECT count(*) FROM accounts").await, 0);
        }
    }
}

#[tokio::test]
async fn a_profile_without_an_id_is_a_provider_error() {
    for (provider, person) in [
        (
            "github",
            Person {
                github_user: json!({ "login": "ada" }),
                ..ada()
            },
        ),
        (
            "google",
            Person {
                google_userinfo: json!({ "sub": "", "email": "ada@example.org" }),
                ..ada()
            },
        ),
        (
            "google",
            Person {
                google_userinfo: json!("nonsense"),
                ..ada()
            },
        ),
    ] {
        let server = Server::new();
        let who = fake::provider().add(person);
        let desktop = Desktop::new();
        arrive(&server, provider, &who, &desktop)
            .await
            .is_loopback_error(&desktop, "provider_error");
    }
}

#[tokio::test]
async fn an_avatar_that_is_not_https_is_not_kept() {
    let server = Server::new();
    let mut person = ada();
    person.google_userinfo["picture"] = json!("javascript:alert(1)");
    let who = fake::provider().add(person);
    let signed_in = sign_in_as(&server, "google", &who, "m1").await;
    assert_eq!(signed_in.account.avatar_url, None);
}

// ------------------------------------------- where the browser may be sent

#[tokio::test]
async fn the_browser_is_only_ever_sent_to_the_port_the_start_stored() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let desktop = Desktop::new();
    let back = back_from_provider(&server, "github", &who, &desktop).await;
    // A callback dressed up to go elsewhere: nothing of it is read but the
    // code and the server's state.
    let dressed = format!(
        "{}&port=4444&redirect_uri=https%3A%2F%2Fevil.example%2F&redirect=https%3A%2F%2Fevil.example\
         &host=evil.example&desktop_state=other",
        on_server(&back)
    );
    let request = Request::builder()
        .method(Method::GET)
        .uri(dressed)
        .header(header::HOST, "evil.example")
        .header("x-forwarded-host", "evil.example")
        .header(header::REFERER, "https://evil.example/")
        .body(Body::empty())
        .unwrap();
    let answer = server.send(request).await;
    let location = answer.location();
    let fields = answer.to_loopback(&desktop);
    assert_eq!(fields.len(), 2, "{fields:?}");
    assert!(!location.contains("evil"), "{location}");
    assert!(!location.contains("4444"), "{location}");
}

#[tokio::test]
async fn the_start_only_ever_sends_the_browser_to_the_provider() {
    let server = Server::new();
    let desktop = Desktop::new();
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "{}&redirect_uri=https%3A%2F%2Fevil.example%2F&client_id=evil&scope=repo",
            desktop.start_path("github")
        ))
        .header(header::HOST, "evil.example")
        .body(Body::empty())
        .unwrap();
    let location = server.send(request).await.location();
    assert!(location.starts_with(&format!("{}/github/authorize?", fake::provider().base())));
    let asked = query_of(&location);
    assert_eq!(
        asked["redirect_uri"],
        "https://account.test/v1/auth/callback/github"
    );
    assert_eq!(asked["client_id"], fake::GITHUB_CLIENT.0);
    assert_eq!(asked["scope"], "read:user user:email");
    assert!(!location.contains("evil"), "{location}");
}

#[tokio::test]
async fn two_sign_ins_at_once_each_come_back_to_their_own_desktop() {
    let server = Server::new();
    let who = fake::provider().add(ada());
    let one = Desktop::new();
    let two = Desktop {
        port: 50_001,
        state: "another-state",
        verifier: OTHER_VERIFIER,
        machine: "Portable",
    };
    let back_one = back_from_provider(&server, "github", &who, &one).await;
    let back_two = back_from_provider(&server, "google", &who, &two).await;
    let fields_two = browse(&server, on_server(&back_two))
        .await
        .to_loopback(&two);
    let fields_one = browse(&server, on_server(&back_one))
        .await
        .to_loopback(&one);
    // Each code with its own desktop's verifier, and not the other's.
    server
        .exchange(&fields_one["code"], two.verifier, machine("m2", "Portable"))
        .await
        .refused(ErrorCode::InvalidCode);
    let answer = server
        .exchange(&fields_two["code"], two.verifier, machine("m2", "Portable"))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
}

// ------------------------------------------------------------------ the start

#[tokio::test]
async fn a_start_the_server_would_not_follow_is_refused_and_stores_nothing() {
    let server = Server::new();
    let good_challenge = challenge(VERIFIER);
    let start = |provider: &str, port: &str, state: &str, challenge: &str, machine: &str| {
        format!(
            "{}?provider={provider}&port={port}&state={state}&challenge={challenge}\
             &machine={machine}",
            paths::AUTH_START
        )
    };
    let long_state = "s".repeat(201);
    let long_machine = "m".repeat(101);
    let c = good_challenge.as_str();
    let mut refused = vec![
        // A provider this protocol does not know.
        start("gitlab", "49152", "s", c, "A"),
        start("GitHub", "49152", "s", c, "A"),
        start("", "49152", "s", c, "A"),
        // A port that is privileged, or is not one.
        start("github", "0", "s", c, "A"),
        start("github", "80", "s", c, "A"),
        start("github", "1023", "s", c, "A"),
        start("github", "65536", "s", c, "A"),
        start("github", "-1", "s", c, "A"),
        start("github", "49152x", "s", c, "A"),
        start("github", "", "s", c, "A"),
        // A state that is empty, too long, or would not go back as it came.
        start("github", "49152", "", c, "A"),
        start("github", "49152", &long_state, c, "A"),
        start("github", "49152", "a%20b", c, "A"),
        start("github", "49152", "a%26code%3Dx", c, "A"),
        start("github", "49152", "a%0D%0Ab", c, "A"),
        start("github", "49152", "%C3%A9", c, "A"),
        // A challenge that is not 32 bytes in base64url.
        start("github", "49152", "s", "", "A"),
        start("github", "49152", "s", &c[..42], "A"),
        start("github", "49152", "s", &format!("{c}A"), "A"),
        start("github", "49152", "s", &"%2B".repeat(43), "A"),
        start("github", "49152", "s", VERIFIER, "A").replace(VERIFIER, &"=".repeat(43)),
        // A machine name the server would not keep.
        start("github", "49152", "s", c, ""),
        start("github", "49152", "s", c, &long_machine),
        start("github", "49152", "s", c, "a%0Ab"),
        // A field twice.
        format!("{}&port=50000", start("github", "49152", "s", c, "A")),
        format!("{}&provider=google", start("github", "49152", "s", c, "A")),
        // No query at all.
        paths::AUTH_START.to_string(),
    ];
    // Each field missing.
    for field in ["provider", "port", "state", "challenge", "machine"] {
        let all = start("github", "49152", "s", c, "A");
        let (path, query) = all.split_once('?').unwrap();
        let without: Vec<&str> = query
            .split('&')
            .filter(|pair| !pair.starts_with(&format!("{field}=")))
            .collect();
        refused.push(format!("{path}?{}", without.join("&")));
    }
    for path in &refused {
        let answer = browse(&server, path).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{path}");
        answer.is_page(400, "This sign-in link is not valid");
        // Nothing of the request is in the page.
        let body = String::from_utf8_lossy(&answer.body);
        assert!(
            !body.contains("49152") && !body.contains("gitlab"),
            "{path}"
        );
    }
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 0);

    // The bounds themselves pass: the lowest and highest ports, the longest
    // state and machine name.
    for path in [
        start("github", "1024", "s", c, "A"),
        start("google", "65535", "s", c, "A"),
        start("github", "49152", &"s".repeat(200), c, &"é".repeat(100)),
        start("github", "49152", "s", c, "Ada%27s%20MacBook%20Pro"),
    ] {
        assert_eq!(
            browse(&server, &path).await.status,
            StatusCode::FOUND,
            "{path}"
        );
    }
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 4);
}

#[tokio::test]
async fn a_provider_the_server_has_no_client_for_is_unavailable() {
    let server = Server::with_sign_in(
        Db::in_memory(),
        Rates::default(),
        SignIn {
            google: None,
            ..sign_in()
        },
    );
    let desktop = Desktop::new();
    browse(&server, &desktop.start_path("google"))
        .await
        .is_page(503, "This sign-in method is not available");
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 0);
    // The other one works.
    browse(&server, &desktop.start_path("github"))
        .await
        .location();
}

#[tokio::test]
async fn a_callback_for_a_provider_that_does_not_exist_is_no_route() {
    let server = Server::new();
    browse(&server, "/v1/auth/callback/gitlab?code=a&state=b")
        .await
        .refused(ErrorCode::NotFound);
}

#[tokio::test]
async fn starts_have_a_limit_of_their_own_per_address() {
    let server = tight(|rates| rates.start = Rate::per_minute(2));
    let path = Desktop::new().start_path("github");
    for _ in 0..2 {
        let answer = server
            .send(from("203.0.113.7", Method::GET, &path, None))
            .await;
        assert_eq!(answer.status, StatusCode::FOUND);
    }
    let answer = server
        .send(from("203.0.113.7", Method::GET, &path, None))
        .await;
    answer.is_page(429, "Too many sign-ins");
    assert_eq!(answer.headers[header::RETRY_AFTER], "60");
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 2);
    // Somebody else is not held back by it.
    let answer = server
        .send(from("203.0.113.8", Method::GET, &path, None))
        .await;
    assert_eq!(answer.status, StatusCode::FOUND);
}

#[tokio::test]
async fn the_server_holds_ten_thousand_sign_ins_in_progress_and_no_more() {
    let server = Server::new();
    server
        .state
        .db
        .call(|conn| {
            conn.execute(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10000) \
                 INSERT INTO auth_states (state_hash, provider, port, desktop_state, challenge, \
                 machine, provider_verifier, created_at, expires_at) \
                 SELECT randomblob(32), 'github', 49152, 's', 'c', 'm', 'v', ?1, ?2 FROM n",
                params![T0, T0 + 10 * MINUTE],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let path = Desktop::new().start_path("github");
    browse(&server, &path)
        .await
        .is_page(429, "Too many sign-ins");
    assert_eq!(
        rows(&server, "SELECT count(*) FROM auth_states").await,
        10_000
    );
    // They expire, and there is room again.
    server.advance(10 * MINUTE);
    assert_eq!(browse(&server, &path).await.status, StatusCode::FOUND);
    assert_eq!(rows(&server, "SELECT count(*) FROM auth_states").await, 1);
}

// ------------------------------------------------------------------ the pages

#[tokio::test]
async fn a_page_is_in_the_browser_s_language_and_loads_nothing() {
    let server = Server::new();
    let path = format!(
        "{}?state={}&code=x",
        paths::auth_callback(Provider::Github),
        secrets::new_oauth_secret().unwrap()
    );
    let ask = |language: Option<&'static str>| {
        let mut request = Request::builder().method(Method::GET).uri(&path);
        if let Some(language) = language {
            request = request.header(header::ACCEPT_LANGUAGE, language);
        }
        request.body(Body::empty()).unwrap()
    };

    let french = server.send(ask(Some("fr-FR,fr;q=0.9,en;q=0.8"))).await;
    french.is_page(400, "Cette connexion a expiré");
    assert_eq!(french.headers[header::CONTENT_LANGUAGE], "fr");
    let body = String::from_utf8_lossy(&french.body).into_owned();
    assert!(body.contains("<html lang=\"fr\">"));

    for language in [None, Some("en-GB"), Some("de")] {
        let english = server.send(ask(language)).await;
        english.is_page(400, "This sign-in has expired");
        assert_eq!(english.headers[header::CONTENT_LANGUAGE], "en");
    }

    // A policy of its own, stricter than nothing: its style sheet by hash,
    // and the router's headers for the rest.
    let policy = french.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(policy.starts_with("default-src 'none'; style-src 'sha256-"));
    assert!(policy.ends_with("base-uri 'none'; form-action 'none'; frame-ancestors 'none'"));
    assert!(!policy.contains("unsafe"));
    assert_eq!(french.headers[header::VARY], "Accept-Language");
    assert_eq!(french.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(french.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(french.headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(french.headers[header::REFERRER_POLICY], "no-referrer");
    // Nothing of the request is in the page.
    assert!(!body.contains(path.split("state=").nth(1).unwrap()));
}
