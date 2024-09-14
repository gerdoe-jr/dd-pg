use serde::{Deserialize, Serialize};

/// A request for a token that is used for the
/// email login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginTokenEmailRequest {
    /// The email of the account to log into.
    pub email: email_address::EmailAddress,
    /// A secret key that was generated through
    /// a verification process (e.g. captchas).
    /// It is optional, since these verification
    /// processes differ from user to user.
    pub secret_key: Option<[u8; 32]>,
}

/// A request for a token that is used for the
/// steam login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginTokenSteamRequest {
    /// The session token generated on the steam client
    /// for the account to log into.
    pub steam_ticket: Vec<u8>,
    /// A secret key that was generated through
    /// a verification process (e.g. captchas).
    /// It is optional, since these verification
    /// processes differ from user to user.
    pub secret_key: Option<[u8; 32]>,
}
