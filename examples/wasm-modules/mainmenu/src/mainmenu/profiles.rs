use std::collections::HashMap;

use async_trait::async_trait;
use client_ui::main_menu::profiles_interface::{LoginTokenError, ProfileData, ProfilesInterface};

pub struct Profiles;

#[async_trait]
impl ProfilesInterface for Profiles {
    async fn login_email_token(
        &self,
        email: email_address::EmailAddress,
        secret_token: Option<String>,
    ) -> anyhow::Result<(), LoginTokenError> {
        Ok(())
    }

    async fn login_email(
        &self,
        email: email_address::EmailAddress,
        token_b64: String,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn user_interaction(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Currently loaded profiles
    fn profiles(&self) -> (HashMap<String, ProfileData>, String) {
        Default::default()
    }
}
