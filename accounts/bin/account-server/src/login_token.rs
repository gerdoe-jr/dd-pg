pub mod queries;

use std::sync::Arc;

use account_sql::query::Query;
use accounts_shared::{
    account_server::{
        errors::AccountServerRequestError, login_token::LoginTokenError, otp::generate_otp,
        result::AccountServerReqResult,
    },
    client::login_token::{LoginTokenEmailRequest, LoginTokenSteamRequest},
};
use axum::Json;
use base64::Engine;
use sqlx::{Acquire, AnyPool};

use crate::{login_token::queries::AddLoginToken, shared::Shared, types::TokenType};

pub async fn login_token_email(
    shared: Arc<Shared>,
    pool: AnyPool,
    requires_secret: bool,
    Json(data): Json<LoginTokenEmailRequest>,
) -> Json<AccountServerReqResult<(), LoginTokenError>> {
    // Check allow & deny lists
    if !shared.email.allow_list.read().is_allowed(&data.email) {
        return Json(AccountServerReqResult::Err(
            AccountServerRequestError::Other(
                "An email from that domain is not in the allowed list of email domains."
                    .to_string(),
            ),
        ));
    }
    if shared.email.deny_list.read().is_banned(&data.email) {
        return Json(AccountServerReqResult::Err(
            AccountServerRequestError::Other(
                "An email from that domain is banned and thus not allowed.".to_string(),
            ),
        ));
    }

    // Before this call a validation process could be added
    if requires_secret && data.secret_key.is_none() {
        return Json(AccountServerReqResult::Err(
            AccountServerRequestError::Other(
                "This function is only for requests with a secret verification token.".to_string(),
            ),
        ));
    }
    Json(
        login_token_email_impl(shared, pool, data)
            .await
            .map_err(|err| AccountServerRequestError::Unexpected {
                target: "login_token_email".into(),
                err: err.to_string(),
                bt: err.backtrace().to_string(),
            }),
    )
}

pub async fn login_token_email_impl(
    shared: Arc<Shared>,
    pool: AnyPool,
    data: LoginTokenEmailRequest,
) -> anyhow::Result<()> {
    // write the new account to the database
    // Add a login token and send it by email
    let token = generate_otp();
    let token_base_64 = base64::prelude::BASE64_URL_SAFE.encode(token);
    let query_add_login_token = AddLoginToken {
        token: &token,
        ty: &TokenType::Email,
        identifier: data.email.as_str(),
    };
    let mut connection = pool.acquire().await?;
    let con = connection.acquire().await?;

    let login_token_res = query_add_login_token
        .query(&shared.db.login_token_statement)
        .execute(&mut *con)
        .await?;
    anyhow::ensure!(
        login_token_res.rows_affected() >= 1,
        "No login token could be added."
    );

    shared
        .email
        .send_email(
            data.email.as_str(),
            "DDNet Account Login",
            format!(
                "Hello {},\nTo finish the login into your account \
                    please use the following code:\n```\n{}\n```",
                data.email.local_part(),
                token_base_64
            ),
        )
        .await?;

    Ok(())
}

pub async fn login_token_steam(
    shared: Arc<Shared>,
    pool: AnyPool,
    requires_secret: bool,
    Json(data): Json<LoginTokenSteamRequest>,
) -> Json<AccountServerReqResult<(), LoginTokenError>> {
    // Before this call a validation process could be added
    if requires_secret && data.secret_key.is_none() {
        return Json(AccountServerReqResult::Err(
            AccountServerRequestError::Other(
                "This function is only for requests with a secret verification token.".to_string(),
            ),
        ));
    }
    Json(
        login_token_steam_impl(shared, pool, data)
            .await
            .map_err(|err| AccountServerRequestError::Unexpected {
                target: "login_token_steam".into(),
                err: err.to_string(),
                bt: err.backtrace().to_string(),
            }),
    )
}

pub async fn login_token_steam_impl(
    shared: Arc<Shared>,
    pool: AnyPool,
    data: LoginTokenSteamRequest,
) -> anyhow::Result<()> {
    // write the new account to the database
    // Add a login token and send it by steam
    let token = generate_otp();
    let query_add_login_token = AddLoginToken {
        token: &token,
        ty: &TokenType::Steam,
        identifier: &hex::encode(&data.steam_ticket),
    };
    let mut connection = pool.acquire().await?;
    let con = connection.acquire().await?;

    let login_token_res = query_add_login_token
        .query(&shared.db.login_token_statement)
        .execute(&mut *con)
        .await?;
    anyhow::ensure!(
        login_token_res.rows_affected() >= 1,
        "No login token could be added."
    );

    shared.steam.verify_steamid64(data.steam_ticket).await?;

    Ok(())
}
