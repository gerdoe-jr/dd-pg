use std::{
    collections::HashMap,
    fmt::Debug,
    future::Future,
    ops::Deref,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime},
};

pub use account_client::login_token::LoginTokenResult;
use account_client::{interface::Io, sign::SignResult};
use accounts_shared::{
    cert::generate_self_signed,
    client::account_data::{key_pair, AccountDataForClient},
};
use anyhow::anyhow;
use either::Either;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use x509_cert::der::Decode;

pub use x509_cert::Certificate;

use crate::fs::Fs;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    pub name: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct ProfilesState {
    pub profiles: HashMap<String, ProfileData>,
    pub cur_profile: String,
}

impl ProfilesState {
    async fn load_or_default(fs: &Fs) -> Self {
        fs.read("profiles.json".as_ref())
            .await
            .map_err(|err| anyhow!(err))
            .and_then(|file| serde_json::from_slice(&file).map_err(|err| anyhow!(err)))
            .unwrap_or_default()
    }

    async fn save(&self, fs: &Fs) -> anyhow::Result<()> {
        let file_content = serde_json::to_vec_pretty(self)?;
        fs.write("".as_ref(), "profiles.json".as_ref(), file_content)
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ProfileCertAndKeys {
    pub cert: Certificate,
    pub key_pair: AccountDataForClient,
    pub valid_duration: Duration,
}

#[derive(Debug, Default, Clone)]
pub enum ProfileCert {
    #[default]
    None,
    Fetching(Arc<tokio::sync::Notify>),
    CertAndKeys(Box<ProfileCertAndKeys>),
    CertAndKeysAndFetch {
        cert_and_keys: Box<ProfileCertAndKeys>,
        notifier: Arc<tokio::sync::Notify>,
    },
}

#[derive(Debug)]
pub struct ActiveProfile<C: Io + Debug> {
    client: Arc<C>,
    cur_cert: Arc<Mutex<ProfileCert>>,

    profile_data: ProfileData,
}

#[derive(Debug, Default)]
pub struct ActiveProfiles<C: Io + Debug> {
    profiles: HashMap<String, ActiveProfile<C>>,
    cur_profile: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AccountlessKeysAndValidy {
    account_data: AccountDataForClient,
    valid_until: chrono::DateTime<chrono::Utc>,
}
// 3 months validy
fn accountless_validy_range() -> Duration {
    Duration::from_secs(60 * 60 * 24 * 30 * 3)
}
const ACCOUNTLESS_KEYS_FILE: &str = "accountless_keys_and_cert.json";

/// Helper for multiple account profiles.
#[derive(Debug)]
pub struct Profiles<
    C: Io + Debug,
    F: Deref<
            Target = dyn Fn(
                PathBuf,
            )
                -> Pin<Box<dyn Future<Output = anyhow::Result<C>> + Sync + Send>>,
        > + Debug
        + Sync
        + Send,
> {
    profiles: Arc<parking_lot::Mutex<ActiveProfiles<C>>>,
    factory: Arc<F>,
    secure_base_path: Arc<PathBuf>,
    fs: Fs,
}

impl<
        C: Io + Debug + 'static,
        F: Deref<
                Target = dyn Fn(
                    PathBuf,
                )
                    -> Pin<Box<dyn Future<Output = anyhow::Result<C>> + Sync + Send>>,
            > + Debug
            + Sync
            + Send,
    > Profiles<C, F>
{
    fn to_profile_states(profiles: &ActiveProfiles<C>) -> ProfilesState {
        let mut res = ProfilesState::default();

        res.profiles.extend(
            profiles
                .profiles
                .iter()
                .map(|(key, val)| (key.clone(), val.profile_data.clone())),
        );
        res.cur_profile.clone_from(&profiles.cur_profile);

        res
    }

    fn email_to_path_friendy(email: &email_address::EmailAddress) -> String {
        email.as_str().replace('@', "_at_").replace('.', "_dot_")
    }

    pub fn new(loading: ProfilesLoading<C, F>) -> Self {
        Self {
            profiles: Arc::new(loading.profiles),
            factory: loading.factory,
            secure_base_path: Arc::new(loading.secure_base_path),
            fs: loading.fs,
        }
    }

    /// generate a token for a new login attempt.
    pub async fn login_email_token(
        &self,
        email: email_address::EmailAddress,
        secret_key_base64: Option<String>,
    ) -> anyhow::Result<(), LoginTokenResult> {
        let profile_name = Self::email_to_path_friendy(&email);
        let path = self.secure_base_path.join(&profile_name);
        let account_client = Arc::new(
            (self.factory)(path)
                .await
                .map_err(LoginTokenResult::Other)?,
        );

        account_client::login_token::login_token_email(
            email,
            secret_key_base64,
            account_client.as_ref(),
        )
        .await?;

        Ok(())
    }

    /// generate a token for a new login attempt.
    pub async fn login_steam_token(
        &self,
        steam_ticket: Vec<u8>,
        secret_key_base64: Option<String>,
    ) -> anyhow::Result<(), LoginTokenResult> {
        let profile_name = "steam".to_string();
        let path = self.secure_base_path.join(&profile_name);
        let account_client = Arc::new(
            (self.factory)(path)
                .await
                .map_err(LoginTokenResult::Other)?,
        );

        account_client::login_token::login_token_steam(
            steam_ticket,
            secret_key_base64,
            account_client.as_ref(),
        )
        .await?;

        Ok(())
    }

    async fn read_accountless_keys(fs: &Fs) -> anyhow::Result<AccountlessKeysAndValidy> {
        fs.read(ACCOUNTLESS_KEYS_FILE.as_ref())
            .await
            .map_err(|err| anyhow!(err))
            .and_then(|file| {
                serde_json::from_slice::<AccountlessKeysAndValidy>(&file)
                    .map_err(|err| anyhow!(err))
            })
            .and_then(|accountless_keys_and_validy| {
                let now: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
                (now.signed_duration_since(accountless_keys_and_validy.valid_until)
                    < chrono::TimeDelta::new(
                        accountless_validy_range().as_secs() as i64,
                        accountless_validy_range().subsec_nanos(),
                    )
                    .unwrap_or(chrono::TimeDelta::max_value()))
                .then_some(accountless_keys_and_validy)
                .ok_or_else(|| anyhow!("accountless keys too old"))
            })
    }

    async fn take_accountless_keys(&self) -> anyhow::Result<AccountDataForClient> {
        let account_data = Self::read_accountless_keys(&self.fs).await?;

        self.fs.remove(ACCOUNTLESS_KEYS_FILE.as_ref()).await?;

        Ok(account_data.account_data)
    }

    async fn login_impl(
        &self,
        profile_name: &str,
        display_name: &str,
        login_token_b64: String,
    ) -> anyhow::Result<()> {
        let path = self.secure_base_path.join(profile_name);
        let account_client = Arc::new((self.factory)(path).await?);

        // first try to "upgrade" the accountless keys to a real account.
        if let Ok(account_data) = self.take_accountless_keys().await {
            account_client::login::login_with_account_data(
                login_token_b64,
                &account_data,
                account_client.as_ref(),
            )
            .await?;
        } else {
            let _ = account_client::login::login(login_token_b64, account_client.as_ref()).await?;
        }

        let profile = ActiveProfile {
            client: account_client,
            cur_cert: Default::default(),
            profile_data: ProfileData {
                name: display_name.to_string(),
            },
        };

        let profiles_state;
        {
            let mut profiles = self.profiles.lock();
            profiles.profiles.insert(profile_name.to_string(), profile);
            profiles.cur_profile = profile_name.to_string();
            profiles_state = Self::to_profile_states(&profiles);
            drop(profiles);
        }

        profiles_state.save(&self.fs).await?;

        self.signed_cert_and_key_pair().await;

        Ok(())
    }

    /// try to login via login token previously created with e.g. [`Self::login_email_token`]
    pub async fn login_email(
        &self,
        email: email_address::EmailAddress,
        login_token_b64: String,
    ) -> anyhow::Result<()> {
        let profile_name = Self::email_to_path_friendy(&email);
        self.login_impl(&profile_name, email.as_str(), login_token_b64)
            .await
    }

    /// try to login via login token previously created with e.g. [`Self::login_steam_token`]
    pub async fn login_steam(&self, login_token_b64: String) -> anyhow::Result<()> {
        let profile_name = "steam";
        self.login_impl(profile_name, profile_name, login_token_b64)
            .await
    }

    /// removes the profile
    async fn logout_impl(
        profiles: Arc<parking_lot::Mutex<ActiveProfiles<C>>>,
        fs: &Fs,
        profile_name: &str,
    ) -> anyhow::Result<()> {
        let profiles_state;
        {
            let mut profiles = profiles.lock();
            profiles.profiles.remove(profile_name);
            if profiles.cur_profile == profile_name {
                profiles.cur_profile = "".into();
            }
            profiles_state = Self::to_profile_states(&profiles);
            drop(profiles);
        }

        profiles_state.save(fs).await?;

        Ok(())
    }

    /// If no account was found, fall back to key-pair that
    /// is not account based, but could be upgraded
    async fn account_less_cert_and_key_pair(
        fs_or_account_data: Either<&Fs, AccountDataForClient>,
        err: Option<anyhow::Error>,
    ) -> (AccountDataForClient, Certificate, Option<anyhow::Error>) {
        match fs_or_account_data {
            Either::Left(fs) => {
                let (account_data, cert) = if let Ok((account_data, cert)) =
                    Self::read_accountless_keys(fs)
                        .await
                        .and_then(|accountless_keys_and_validy| {
                            generate_self_signed(
                                &accountless_keys_and_validy.account_data.private_key,
                            )
                            .map_err(|err| anyhow!(err))
                            .map(|cert| (accountless_keys_and_validy.account_data, cert))
                        }) {
                    (account_data, cert)
                } else {
                    let (private_key, public_key) = key_pair();

                    let cert = generate_self_signed(&private_key).unwrap();

                    // save the newely generated cert & account data
                    let accountless_keys_and_cert = AccountlessKeysAndValidy {
                        account_data: AccountDataForClient {
                            private_key,
                            public_key,
                        },
                        valid_until: (std::time::SystemTime::now() + accountless_validy_range())
                            .into(),
                    };

                    // ignore errors, can't recover anyway
                    if let Ok(file) = serde_json::to_vec(&accountless_keys_and_cert) {
                        let _ = fs
                            .write("".as_ref(), ACCOUNTLESS_KEYS_FILE.as_ref(), file)
                            .await;
                    }

                    (accountless_keys_and_cert.account_data, cert)
                };
                (account_data, cert, err)
            }
            Either::Right(account_data) => {
                let cert = generate_self_signed(&account_data.private_key).unwrap();
                (account_data, cert, err)
            }
        }
    }

    /// Gets a _recently_ signed cerificate from the accounts server
    /// and the key pair of the client.
    /// If an error occurred a self signed cert & key-pair will still be generated to
    /// allow playing at all cost.
    /// It's up to the implementation how it wants to inform the user about
    /// this error.
    pub async fn signed_cert_and_key_pair(
        &self,
    ) -> (AccountDataForClient, Certificate, Option<anyhow::Error>) {
        let mut cur_cert_der = None;
        let mut account_client = None;
        let mut cur_profile = None;
        {
            let profiles = self.profiles.lock();
            if let Some(profile) = profiles.profiles.get(&profiles.cur_profile) {
                cur_cert_der = Some(profile.cur_cert.clone());
                account_client = Some(profile.client.clone());
                cur_profile = Some(profiles.cur_profile.clone());
            }
            drop(profiles);
        }

        if let Some(((cur_cert, client), cur_profile)) =
            cur_cert_der.zip(account_client).zip(cur_profile)
        {
            let mut try_fetch = None;
            let mut try_wait = None;
            {
                let mut cert = cur_cert.lock();
                match &*cert {
                    ProfileCert::None => {
                        let notifier: Arc<tokio::sync::Notify> = Default::default();
                        *cert = ProfileCert::Fetching(notifier.clone());
                        try_fetch = Some((notifier, true));
                    }
                    ProfileCert::Fetching(notifier) => {
                        try_wait = Some(notifier.clone());
                    }
                    ProfileCert::CertAndKeys(cert_and_keys) => {
                        // check if cert is outdated
                        let expires_at = cert_and_keys
                            .cert
                            .tbs_certificate
                            .validity
                            .not_after
                            .to_system_time();
                        // if it is about to expire, fetch again replacing the old ones
                        if expires_at < SystemTime::now() + Duration::from_secs(60 * 10) {
                            let notifier: Arc<tokio::sync::Notify> = Default::default();
                            *cert = ProfileCert::Fetching(notifier.clone());
                            try_fetch = Some((notifier, true));
                        }
                        // else if the cert's lifetime already hit the half, try to fetch, but don't replace the existing one
                        else if expires_at < SystemTime::now() + cert_and_keys.valid_duration / 2
                        {
                            let notifier: Arc<tokio::sync::Notify> = Default::default();
                            *cert = ProfileCert::CertAndKeysAndFetch {
                                cert_and_keys: cert_and_keys.clone(),
                                notifier: notifier.clone(),
                            };
                            try_fetch = Some((notifier, false));
                        }
                    }
                    ProfileCert::CertAndKeysAndFetch {
                        cert_and_keys,
                        notifier,
                    } => {
                        // if fetching gets urgent, downgrade this to fetch operation
                        let expires_at = cert_and_keys
                            .cert
                            .tbs_certificate
                            .validity
                            .not_after
                            .to_system_time();
                        if expires_at < SystemTime::now() + Duration::from_secs(60 * 10) {
                            let notifier = notifier.clone();
                            *cert = ProfileCert::Fetching(notifier.clone());
                            try_wait = Some(notifier);
                        }
                        // else just ignore
                    }
                }
            }

            if let Some(notifier) = try_wait {
                notifier.notified().await;
                // notify the next one
                notifier.notify_one();
            }

            let should_wait = if let Some((notifier, should_wait)) = try_fetch {
                let fs = self.fs.clone();
                let profiles = self.profiles.clone();
                let cur_cert = cur_cert.clone();
                let res = tokio::spawn(async move {
                    let res = match account_client::sign::sign(client.as_ref()).await {
                        Ok(sign_data) => {
                            if let Ok(cert) = Certificate::from_der(&sign_data.certificate_der) {
                                *cur_cert.lock() =
                                    ProfileCert::CertAndKeys(Box::new(ProfileCertAndKeys {
                                        cert: cert.clone(),
                                        key_pair: sign_data.session_key_pair.clone(),
                                        valid_duration: cert
                                            .tbs_certificate
                                            .validity
                                            .not_after
                                            .to_system_time()
                                            .duration_since(SystemTime::now())
                                            .unwrap_or(Duration::ZERO),
                                    }));
                                (sign_data.session_key_pair, cert, None)
                            } else {
                                Self::account_less_cert_and_key_pair(
                                    Either::Left(&fs),
                                    Some(anyhow!(
                                        "account server did not return a valid certificate, \
                                        please contact a developer."
                                    )),
                                )
                                .await
                            }
                        }
                        Err(err) => {
                            *cur_cert.lock() = ProfileCert::None;
                            // if the error was a file system error
                            // or session was invalid for other reasons, then remove that profile.
                            match err {
                                SignResult::SessionWasInvalid | SignResult::FsLikeError(_) => {
                                    // try to remove that profile
                                    let _ = Self::logout_impl(profiles, &fs, &cur_profile).await;
                                    Self::account_less_cert_and_key_pair(
                                        Either::Left(&fs),
                                        Some(err.into()),
                                    )
                                    .await
                                }
                                SignResult::HttpLikeError {
                                    ref account_data, ..
                                }
                                | SignResult::Other {
                                    ref account_data, ..
                                } => {
                                    // tell the fallback key mechanism to try the account data,
                                    // even if self signed, this can allow a game server
                                    // to recover lost account related data. (But does not require to)
                                    Self::account_less_cert_and_key_pair(
                                        Either::Right(account_data.clone()),
                                        Some(err.into()),
                                    )
                                    .await
                                }
                            }
                        }
                    };
                    notifier.notify_one();
                    res
                });
                should_wait.then_some(res)
            } else {
                None
            };

            // if fetching was urgent, it must wait for the task to complete.
            let awaited_task = if let Some(task) = should_wait {
                task.await.ok()
            } else {
                None
            };

            if let Some(res) = awaited_task {
                res
            } else {
                let (ProfileCert::CertAndKeys(cert_and_keys)
                | ProfileCert::CertAndKeysAndFetch { cert_and_keys, .. }) = cur_cert.lock().clone()
                else {
                    return Self::account_less_cert_and_key_pair(
                        Either::Left(&self.fs),
                        Some(anyhow!("no cert or key found.")),
                    )
                    .await;
                };
                let ProfileCertAndKeys { cert, key_pair, .. } = *cert_and_keys;

                (key_pair, cert, None)
            }
        } else {
            Self::account_less_cert_and_key_pair(Either::Left(&self.fs), None).await
        }
    }

    /// Currently loaded profiles
    pub fn profiles(&self) -> (HashMap<String, ProfileData>, String) {
        let profiles = self.profiles.lock();
        let profiles = Self::to_profile_states(&profiles);
        (profiles.profiles, profiles.cur_profile)
    }
}

#[derive(Debug)]
pub struct ProfilesLoading<
    C: Io + Debug,
    F: Deref<
            Target = dyn Fn(
                PathBuf,
            )
                -> Pin<Box<dyn Future<Output = anyhow::Result<C>> + Sync + Send>>,
        > + Debug
        + Sync
        + Send,
> {
    pub profiles: parking_lot::Mutex<ActiveProfiles<C>>,
    pub factory: Arc<F>,
    pub secure_base_path: PathBuf,
    fs: Fs,
}

impl<
        C: Io + Debug,
        F: Deref<
                Target = dyn Fn(
                    PathBuf,
                )
                    -> Pin<Box<dyn Future<Output = anyhow::Result<C>> + Sync + Send>>,
            > + Debug
            + Sync
            + Send,
    > ProfilesLoading<C, F>
{
    pub async fn new(secure_base_path: PathBuf, factory: Arc<F>) -> anyhow::Result<Self> {
        let fs = Fs::new(secure_base_path.clone()).await?;
        let profiles_state = ProfilesState::load_or_default(&fs).await;
        let mut profiles: HashMap<String, ActiveProfile<C>> = Default::default();
        for (profile_key, profile) in profiles_state.profiles {
            profiles.insert(
                profile_key.clone(),
                ActiveProfile {
                    client: Arc::new(factory(secure_base_path.join(profile_key)).await?),
                    cur_cert: Default::default(),
                    profile_data: profile,
                },
            );
        }
        Ok(Self {
            profiles: parking_lot::Mutex::new(ActiveProfiles {
                profiles,
                cur_profile: profiles_state.cur_profile,
            }),
            factory,
            fs,
            secure_base_path,
        })
    }
}
