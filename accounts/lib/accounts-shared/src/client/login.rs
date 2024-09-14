use anyhow::anyhow;
use base64::Engine;
use ed25519_dalek::{Signature, Signer};
use serde::{Deserialize, Serialize};

use super::account_data::{
    generate_account_data, generate_account_data_from_key_pair, AccountData, AccountDataForClient,
    AccountDataForServer,
};

/// A login token previously sent to email or generated
/// for a steam login attempt.
pub type LoginToken = [u8; 32];

/// Represents the data required for a login attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    /// The account data related to the login request.
    pub account_data: AccountDataForServer,
    /// A login token that was sent by
    /// email or generated for a steam based login etc.
    pub login_token: LoginToken,
    /// The signature for the login token,
    /// used to make sure the public key corresponds
    /// to a valid private key.
    pub login_token_signature: Signature,
}

fn login_from_account_data(
    account_data: AccountData,
    login_token_b64: String,
) -> anyhow::Result<(LoginRequest, AccountDataForClient)> {
    let login_token = base64::prelude::BASE64_URL_SAFE.decode(login_token_b64)?;
    let signature = account_data.for_client.private_key.sign(&login_token);

    Ok((
        LoginRequest {
            login_token_signature: signature,
            account_data: account_data.for_server,
            login_token: login_token
                .try_into()
                .map_err(|_| anyhow!("Invalid login token."))?,
        },
        account_data.for_client,
    ))
}

/// Prepares a login request for the account server.
pub fn login_from_client_account_data(
    account_data: &AccountDataForClient,
    login_token_b64: String,
) -> anyhow::Result<(LoginRequest, AccountDataForClient)> {
    let account_data = generate_account_data_from_key_pair(
        account_data.private_key.clone(),
        account_data.public_key,
    )?;

    login_from_account_data(account_data, login_token_b64)
}

/// Prepares a login request for the account server.
pub fn login(login_token_b64: String) -> anyhow::Result<(LoginRequest, AccountDataForClient)> {
    let account_data = generate_account_data()?;

    login_from_account_data(account_data, login_token_b64)
}
