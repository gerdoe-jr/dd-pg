use std::{str::FromStr, sync::Arc};

use account_client::certs::{certs_to_pub_keys, download_certs};
use account_game_server::rename::RenameError;
use accounts_shared::{
    account_server::cert_account_ext::AccountCertExt,
    client::credential_auth_token::CredentialAuthTokenOperation, game_server,
};
use anyhow::anyhow;
use client_reqwest::client::ClientReqwestTokioFs;
use email_address::EmailAddress;
use parking_lot::Mutex;
use x509_cert::der::Decode;

use crate::tests::types::{TestAccServer, TestGameServer};

#[tokio::test]
async fn rename_hardening() {
    let test = async move {
        let secure_dir_client = tempfile::tempdir()?;

        // account server setup
        let token: Arc<Mutex<String>> = Default::default();
        let account_token: Arc<Mutex<String>> = Default::default();
        let acc_server =
            TestAccServer::new(token.clone(), account_token.clone(), false, true).await?;
        let pool = acc_server.pool.clone();

        let url = "http://localhost:4433";
        let client =
            ClientReqwestTokioFs::new(vec![url.try_into()?], secure_dir_client.path()).await?;

        let login = |email: EmailAddress| {
            Box::pin(async {
                account_client::credential_auth_token::credential_auth_token_email(
                    email,
                    CredentialAuthTokenOperation::Login,
                    None,
                    &*client,
                )
                .await?;

                // do actual login for client
                let token_hex = token.lock().clone();
                let account_data = account_client::login::login(token_hex, &*client).await?;
                anyhow::Ok(account_data)
            })
        };
        // the first login will also create the account
        login(EmailAddress::from_str("test@localhost")?)
            .await?
            .1
            .write(&*client)
            .await?;

        // create a current signed certificate on the account server
        let cert = account_client::sign::sign(&*client).await?;

        let Ok(Some((_, account_data))) = x509_cert::Certificate::from_der(&cert.certificate_der)?
            .tbs_certificate
            .get::<AccountCertExt>()
        else {
            return Err(anyhow!("no valid account data found."));
        };

        assert!(account_data.data.account_id >= 1);

        // now comes game server
        let game_server = TestGameServer::new(&pool).await?;
        let game_server_data = game_server.game_server_data.clone();

        let certs = download_certs(&*client).await?;
        let keys = certs_to_pub_keys(&certs);

        let user_id = game_server::user_id::user_id_from_cert(&keys, cert.certificate_der);
        assert!(user_id.account_id.is_some());

        // Login the user
        let auto_login_res =
            account_game_server::auto_login::auto_login(game_server_data.clone(), &pool, &user_id)
                .await;
        assert!(auto_login_res.is_ok_and(|v| v));

        account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &user_id,
            "nameless_tee",
        )
        .await?;
        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &user_id,
            "nameless+ee",
        )
        .await;
        assert!(matches!(res, Err(RenameError::InvalidAscii)));
        let res =
            account_game_server::rename::rename(game_server_data.clone(), &pool, &user_id, "name.")
                .await;
        assert!(matches!(res, Err(RenameError::InvalidAscii)));
        let res =
            account_game_server::rename::rename(game_server_data.clone(), &pool, &user_id, "name-")
                .await;
        assert!(matches!(res, Err(RenameError::InvalidAscii)));
        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &user_id,
            "name tee",
        )
        .await;
        assert!(matches!(res, Err(RenameError::InvalidAscii)));
        let res =
            account_game_server::rename::rename(game_server_data.clone(), &pool, &user_id, "name'")
                .await;
        assert!(matches!(res, Err(RenameError::InvalidAscii)));
        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &user_id,
            "name\"",
        )
        .await;
        assert!(matches!(res, Err(RenameError::InvalidAscii)));
        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &user_id,
            "autouser123",
        )
        .await;
        assert!(matches!(res, Err(RenameError::ReservedName)));
        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &user_id,
            "autouserre",
        )
        .await;
        assert!(matches!(res, Err(RenameError::ReservedName)));
        let res =
            account_game_server::rename::rename(game_server_data.clone(), &pool, &user_id, "a")
                .await;
        assert!(matches!(res, Err(RenameError::NameLengthInvalid)));
        let res =
            account_game_server::rename::rename(game_server_data.clone(), &pool, &user_id, "ab")
                .await;
        assert!(matches!(res, Err(RenameError::NameLengthInvalid)));
        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &user_id,
            "012345678901234567890123456789012",
        )
        .await;
        assert!(matches!(res, Err(RenameError::NameLengthInvalid)));

        // create another user
        login(EmailAddress::from_str("test2@localhost")?)
            .await?
            .1
            .write(&*client)
            .await?;
        let cert = account_client::sign::sign(&*client).await?;
        let mut new_user_id = game_server::user_id::user_id_from_cert(&keys, cert.certificate_der);
        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &new_user_id,
            "nameless_tee",
        )
        .await;
        assert!(matches!(res, Err(RenameError::NameAlreadyExists)));

        // Act as if the user has no account
        new_user_id.account_id = None;

        let res = account_game_server::rename::rename(
            game_server_data.clone(),
            &pool,
            &new_user_id,
            "nameless_tee2",
        )
        .await;
        assert!(matches!(res, Ok(false)));

        game_server.destroy().await?;
        // game server end

        acc_server.destroy().await?;

        anyhow::Ok(())
    };

    test.await.unwrap()
}
