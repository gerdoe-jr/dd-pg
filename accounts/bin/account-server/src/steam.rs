use std::{fmt::Debug, sync::Arc};

use url::Url;

pub trait SteamHook: Debug + Sync + Send {
    fn on_steam_code(&self, steam_code: &[u8]);
}

#[derive(Debug)]
struct SteamHookDummy {}
impl SteamHook for SteamHookDummy {
    fn on_steam_code(&self, _steam_code: &[u8]) {
        // empty
    }
}

/// Shared steam helper
#[derive(Debug)]
pub struct SteamShared {
    http: reqwest::Client,
    steam_hook: Arc<dyn SteamHook>,

    steam_auth_url: String,
    publisher_auth_key: String,
    app_id: u32,
}

/// https://partner.steamgames.com/doc/webapi/ISteamUserAuth#AuthenticateUserTicket
pub const OFFICIAL_STEAM_AUTH_URL: &str =
    "https://partner.steam-api.com/ISteamUserAuth/AuthenticateUserTicket/v1/";

impl SteamShared {
    pub fn new(steam_auth_url: Url, publisher_auth_key: &str, app_id: u32) -> anyhow::Result<Self> {
        let http = reqwest::Client::new();

        Ok(Self {
            http,
            steam_hook: Arc::new(SteamHookDummy {}),

            app_id,
            publisher_auth_key: publisher_auth_key.to_string(),
            steam_auth_url: steam_auth_url.to_string(),
        })
    }

    /// A hook that can see all sent steam token requests.
    /// Currently only useful for testing
    #[allow(dead_code)]
    pub fn set_hook<F: SteamHook + 'static>(&mut self, hook: F) {
        self.steam_hook = Arc::new(hook);
    }

    pub async fn verify_steamid64(&self, steam_ticket: Vec<u8>) -> anyhow::Result<i64> {
        self.steam_hook.on_steam_code(&steam_ticket);

        let ticket = hex::encode(steam_ticket);

        let steamid64_str: String = self
            .http
            .get(format!(
                "{}?key={}&appid={}&ticket={}",
                self.steam_auth_url, self.publisher_auth_key, self.app_id, ticket
            ))
            .send()
            .await?
            .text()
            .await?;

        Ok(steamid64_str.parse()?)
    }
}

#[cfg(test)]
mod test {
    use axum::{extract::Query, routing::get, Router};
    use serde::Deserialize;

    use crate::steam::SteamShared;

    #[tokio::test]
    async fn email_test() {
        // from https://partner.steamgames.com/doc/webapi/ISteamUserAuth#AuthenticateUserTicket
        #[derive(Debug, Deserialize)]
        struct SteamQueryParams {
            pub key: String,
            pub appid: u32,
            pub ticket: String,
            pub identity: Option<String>,
        }
        async fn steam_id_check(
            Query(q): Query<SteamQueryParams>,
        ) -> axum::response::Response<String> {
            dbg!(q.key, q.appid, q.ticket, q.identity);
            axum::http::Response::new(0.to_string())
        }
        let app = Router::new().route("/", get(steam_id_check));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:4433")
            .await
            .unwrap();
        axum::serve(listener, app).await.unwrap();

        let steam = SteamShared::new(
            "http://127.0.0.1:4433/".try_into().unwrap(),
            "the_secret_publisher_key",
            1337,
        )
        .unwrap();

        assert!(steam.verify_steamid64(vec![]).await.unwrap() == 0);
    }
}
