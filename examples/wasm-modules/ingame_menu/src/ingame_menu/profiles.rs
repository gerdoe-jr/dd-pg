use std::collections::HashMap;

use anyhow::anyhow;
use async_trait::async_trait;
use client_ui::main_menu::profiles_interface::{
    AccountInfo, AccountTokenError, AccountTokenOperation, CredentialAuthTokenError,
    CredentialAuthTokenOperation, ProfileData, ProfilesInterface,
};

pub struct Profiles;

#[async_trait]
impl ProfilesInterface for Profiles {
    fn supports_steam(&self) -> bool {
        true
    }

    fn steam_id64(&self) -> i64 {
        -1
    }

    async fn credential_auth_email_token(
        &self,
        op: CredentialAuthTokenOperation,
        email: email_address::EmailAddress,
        secret_token: Option<String>,
    ) -> anyhow::Result<(), CredentialAuthTokenError> {
        Ok(())
    }

    async fn credential_auth_steam_token(
        &self,
        op: CredentialAuthTokenOperation,
        secret_token: Option<String>,
    ) -> anyhow::Result<String, CredentialAuthTokenError> {
        Ok("".to_string())
    }

    async fn account_email_token(
        &self,
        op: AccountTokenOperation,
        email: email_address::EmailAddress,
        secret_token: Option<String>,
    ) -> anyhow::Result<(), AccountTokenError> {
        Ok(())
    }

    async fn account_steam_token(
        &self,
        op: AccountTokenOperation,
        secret_token: Option<String>,
    ) -> anyhow::Result<String, AccountTokenError> {
        Ok("".to_string())
    }

    async fn login_email(
        &self,
        email: email_address::EmailAddress,
        token_hex: String,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn login_steam(&self, token_hex: String) -> anyhow::Result<()> {
        Ok(())
    }

    async fn link_credential(
        &self,
        account_token_hex: String,
        credential_auth_token_hex: String,
        name: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn unlink_credential(
        &self,
        credential_auth_token_hex: String,
        name: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn logout(&self, name: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn logout_all(&self, account_token_hex: String, name: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn delete(&self, account_token_hex: String, name: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn user_interaction(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn account_info(&self, name: &str) -> anyhow::Result<AccountInfo> {
        Err(anyhow!("No account info fetched"))
    }

    /// Currently loaded profiles
    fn profiles(&self) -> (HashMap<String, ProfileData>, String) {
        Default::default()
    }

    async fn set_profile(&self, name: &str) {}

    async fn set_profile_display_name(&self, profile_name: &str, display_name: String) {}
}
