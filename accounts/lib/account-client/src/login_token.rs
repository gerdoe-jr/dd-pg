use accounts_shared::{
    account_server::{errors::AccountServerRequestError, login_token::LoginTokenError},
    client::login_token::{LoginTokenEmailRequest, LoginTokenSteamRequest},
};

use anyhow::anyhow;
use base64::Engine;
use thiserror::Error;

use crate::{
    errors::{FsLikeError, HttpLikeError},
    interface::Io,
    safe_interface::{IoSafe, SafeIo},
};

/// The result of a [`login_token_email`] request.
#[derive(Error, Debug)]
pub enum LoginTokenResult {
    /// A http like error occurred.
    #[error("{0}")]
    HttpLikeError(HttpLikeError),
    /// A fs like error occurred.
    #[error("{0}")]
    FsLikeError(FsLikeError),
    /// The account server responded with an error.
    #[error("{0:?}")]
    AccountServerRequstError(AccountServerRequestError<LoginTokenError>),
    /// Errors that are not handled explicitly.
    #[error("Login failed: {0}")]
    Other(anyhow::Error),
}

impl From<HttpLikeError> for LoginTokenResult {
    fn from(value: HttpLikeError) -> Self {
        Self::HttpLikeError(value)
    }
}

impl From<FsLikeError> for LoginTokenResult {
    fn from(value: FsLikeError) -> Self {
        Self::FsLikeError(value)
    }
}

fn get_secret_key(
    secret_key_base64: Option<String>,
) -> anyhow::Result<Option<[u8; 32]>, LoginTokenResult> {
    secret_key_base64
        .map(|secret_key_base64| base64::prelude::BASE64_URL_SAFE.decode(secret_key_base64))
        .transpose()
        .map_err(|err| LoginTokenResult::Other(err.into()))?
        .map(|secret_key| secret_key.try_into())
        .transpose()
        .map_err(|_| {
            LoginTokenResult::Other(anyhow!(
                "secret key had an invalid length. make sure you copied it correctly."
            ))
        })
}

/// Generate a token sent by email for a new session/account.
pub async fn login_token_email(
    email: email_address::EmailAddress,
    secret_key_base64: Option<String>,
    io: &dyn Io,
) -> anyhow::Result<(), LoginTokenResult> {
    login_token_email_impl(email, secret_key_base64, io.into()).await
}

async fn login_token_email_impl(
    email: email_address::EmailAddress,
    secret_key_base64: Option<String>,
    io: IoSafe<'_>,
) -> anyhow::Result<(), LoginTokenResult> {
    let secret_key = get_secret_key(secret_key_base64)?;
    if secret_key.is_some() {
        io.request_login_email_token_with_secret_key(LoginTokenEmailRequest { email, secret_key })
            .await?
            .map_err(LoginTokenResult::AccountServerRequstError)?;
    } else {
        io.request_login_email_token(LoginTokenEmailRequest { email, secret_key })
            .await?
            .map_err(LoginTokenResult::AccountServerRequstError)?;
    }

    Ok(())
}

/// Generate a token sent for a steam auth for a new session/account.
pub async fn login_token_steam(
    steam_ticket: Vec<u8>,
    secret_key_base64: Option<String>,
    io: &dyn Io,
) -> anyhow::Result<(), LoginTokenResult> {
    login_token_steam_impl(steam_ticket, secret_key_base64, io.into()).await
}

async fn login_token_steam_impl(
    steam_ticket: Vec<u8>,
    secret_key_base64: Option<String>,
    io: IoSafe<'_>,
) -> anyhow::Result<(), LoginTokenResult> {
    let secret_key = get_secret_key(secret_key_base64)?;
    if secret_key.is_some() {
        io.request_login_steam_token_with_secret_key(LoginTokenSteamRequest {
            steam_ticket,
            secret_key,
        })
        .await?
        .map_err(LoginTokenResult::AccountServerRequstError)?;
    } else {
        io.request_login_steam_token(LoginTokenSteamRequest {
            steam_ticket,
            secret_key,
        })
        .await?
        .map_err(LoginTokenResult::AccountServerRequstError)?;
    }

    Ok(())
}
