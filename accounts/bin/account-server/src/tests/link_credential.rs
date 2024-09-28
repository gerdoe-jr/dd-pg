use std::{str::FromStr, sync::Arc};

use account_client::{
    account_token::AccountTokenResult,
    certs::{certs_to_pub_keys, download_certs},
    link_credential::LinkCredentialResult,
};
use accounts_shared::{
    client::{
        account_token::AccountTokenOperation, credential_auth_token::CredentialAuthTokenOperation,
    },
    game_server,
};
use client_reqwest::client::ClientReqwestTokioFs;
use email_address::EmailAddress;
use parking_lot::Mutex;

use crate::tests::types::TestAccServer;

/// Tests related to verifying that link credential does
/// what it should and fails appropriately
#[tokio::test]
async fn link_credential_hardening() {
    let test = async move {
        let secure_dir_client = tempfile::tempdir()?;
        // account server setup
        let token: Arc<Mutex<String>> = Default::default();
        let account_token: Arc<Mutex<String>> = Default::default();
        let acc_server =
            TestAccServer::new(token.clone(), account_token.clone(), false, true).await?;

        let client = ClientReqwestTokioFs::new(
            vec!["http://localhost:4433".try_into()?],
            secure_dir_client.path(),
        )
        .await?;

        let certs = download_certs(&*client).await?;
        let keys = certs_to_pub_keys(&certs);

        // create an account
        account_client::credential_auth_token::credential_auth_token_email(
            EmailAddress::from_str("test@localhost")?,
            CredentialAuthTokenOperation::Login,
            None,
            &*client,
        )
        .await?;
        let token_hex = token.lock().clone();
        account_client::login::login(token_hex.clone(), &*client)
            .await?
            .1
            .write(&*client)
            .await?;

        let cert = account_client::sign::sign(&*client).await?;
        let user_id = game_server::user_id::user_id_from_cert(&keys, cert.certificate_der);

        // try to link the email against a non-existing steam account
        // must fail
        let account_token_hex = account_client::account_token::account_token_steam(
            b"justatest".to_vec(),
            AccountTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await;
        assert!(matches!(
            account_token_hex,
            Err(AccountTokenResult::AccountServerRequstError(_))
        ));

        // don't allow emails with display name or ips
        let res = account_client::account_token::account_token_email(
            EmailAddress::from_str("Name <test@localhost>")?,
            AccountTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await;
        assert!(matches!(
            res,
            Err(AccountTokenResult::AccountServerRequstError(_))
        ));
        let res = account_client::account_token::account_token_email(
            EmailAddress::from_str("test@[127.0.0.1]")?,
            AccountTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await;
        assert!(matches!(
            res,
            Err(AccountTokenResult::AccountServerRequstError(_))
        ));

        // rename the linked email
        account_client::account_token::account_token_email(
            EmailAddress::from_str("test@localhost")?,
            AccountTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await?;
        let account_token_hex = account_token.lock().clone();
        account_client::credential_auth_token::credential_auth_token_email(
            EmailAddress::from_str("test2@localhost")?,
            CredentialAuthTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await?;
        let credential_auth_token_hex = token.lock().clone();
        account_client::link_credential::link_credential(
            account_token_hex,
            credential_auth_token_hex,
            &*client,
        )
        .await?;

        // use a wrong account token operation
        account_client::account_token::account_token_email(
            EmailAddress::from_str("test2@localhost")?,
            AccountTokenOperation::Delete,
            None,
            &*client,
        )
        .await?;
        let account_token_hex = account_token.lock().clone();
        account_client::credential_auth_token::credential_auth_token_email(
            EmailAddress::from_str("test@localhost")?,
            CredentialAuthTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await?;
        let credential_auth_token_hex = token.lock().clone();
        let res = account_client::link_credential::link_credential(
            account_token_hex,
            credential_auth_token_hex,
            &*client,
        )
        .await;
        assert!(matches!(
            res,
            Err(LinkCredentialResult::AccountServerRequstError(_))
        ));

        // login with new email
        account_client::credential_auth_token::credential_auth_token_email(
            EmailAddress::from_str("test2@localhost")?,
            CredentialAuthTokenOperation::Login,
            None,
            &*client,
        )
        .await?;
        let token_hex = token.lock().clone();
        account_client::login::login(token_hex.clone(), &*client)
            .await?
            .1
            .write(&*client)
            .await?;

        // match old & new user_id
        let cert = account_client::sign::sign(&*client).await?;
        let new_user_id = game_server::user_id::user_id_from_cert(&keys, cert.certificate_der);
        assert!(user_id.account_id == new_user_id.account_id);

        // link steam to the account
        account_client::account_token::account_token_email(
            EmailAddress::from_str("test2@localhost")?,
            AccountTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await?;
        let account_token_hex = account_token.lock().clone();
        let credential_auth_token_hex =
            account_client::credential_auth_token::credential_auth_token_steam(
                b"justatest".to_vec(),
                CredentialAuthTokenOperation::LinkCredential,
                None,
                &*client,
            )
            .await?;
        account_client::link_credential::link_credential(
            account_token_hex,
            credential_auth_token_hex,
            &*client,
        )
        .await?;

        // login by steam
        let token_hex = account_client::credential_auth_token::credential_auth_token_steam(
            b"justatest".to_vec(),
            CredentialAuthTokenOperation::Login,
            None,
            &*client,
        )
        .await?;
        account_client::login::login(token_hex.clone(), &*client)
            .await?
            .1
            .write(&*client)
            .await?;

        // match old & new user_id
        let cert = account_client::sign::sign(&*client).await?;
        let new_user_id = game_server::user_id::user_id_from_cert(&keys, cert.certificate_der);
        assert!(user_id.account_id == new_user_id.account_id);

        // create an account on the old email
        account_client::credential_auth_token::credential_auth_token_email(
            EmailAddress::from_str("test@localhost")?,
            CredentialAuthTokenOperation::Login,
            None,
            &*client,
        )
        .await?;
        let token_hex = token.lock().clone();
        account_client::login::login(token_hex.clone(), &*client)
            .await?
            .1
            .write(&*client)
            .await?;

        // make sure the accounts differ
        let cert = account_client::sign::sign(&*client).await?;
        let new_user_id = game_server::user_id::user_id_from_cert(&keys, cert.certificate_der);
        assert!(user_id.account_id != new_user_id.account_id);

        // try to link steam against the new email
        account_client::account_token::account_token_email(
            EmailAddress::from_str("test@localhost")?,
            AccountTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await?;
        let account_token_hex = account_token.lock().clone();
        let credential_auth_token_hex =
            account_client::credential_auth_token::credential_auth_token_steam(
                b"justatest".to_vec(),
                CredentialAuthTokenOperation::LinkCredential,
                None,
                &*client,
            )
            .await?;
        let res = account_client::link_credential::link_credential(
            account_token_hex,
            credential_auth_token_hex,
            &*client,
        )
        .await;
        assert!(matches!(
            res,
            Err(LinkCredentialResult::AccountServerRequstError(_))
        ));

        // try to link the original email against the steam account
        // which should fail because the original email already has a different account
        let account_token_hex = account_client::account_token::account_token_steam(
            b"justatest".to_vec(),
            AccountTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await?;
        account_client::credential_auth_token::credential_auth_token_email(
            EmailAddress::from_str("test@localhost")?,
            CredentialAuthTokenOperation::LinkCredential,
            None,
            &*client,
        )
        .await?;
        let credential_auth_token_hex = token.lock().clone();
        let res = account_client::link_credential::link_credential(
            account_token_hex,
            credential_auth_token_hex,
            &*client,
        )
        .await;
        assert!(matches!(
            res,
            Err(LinkCredentialResult::AccountServerRequstError(_))
        ));

        acc_server.destroy().await?;

        anyhow::Ok(())
    };
    test.await.unwrap();
}
