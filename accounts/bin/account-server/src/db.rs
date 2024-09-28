use sqlx::any::AnyStatement;

/// Shared data for a db connection
pub struct DbConnectionShared {
    pub credential_auth_token_statement: AnyStatement<'static>,
    pub credential_auth_token_qry_statement: AnyStatement<'static>,
    pub invalidate_credential_auth_token_statement: AnyStatement<'static>,
    pub try_create_account_statement: AnyStatement<'static>,
    pub account_id_from_last_insert_qry_statement: AnyStatement<'static>,
    pub account_id_from_email_qry_statement: AnyStatement<'static>,
    pub account_id_from_steam_qry_statement: AnyStatement<'static>,
    pub link_credentials_email_qry_statement: AnyStatement<'static>,
    pub link_credentials_steam_qry_statement: AnyStatement<'static>,
    pub create_session_statement: AnyStatement<'static>,
    pub logout_statement: AnyStatement<'static>,
    pub auth_attempt_statement: AnyStatement<'static>,
    pub account_token_email_statement: AnyStatement<'static>,
    pub account_token_steam_statement: AnyStatement<'static>,
    pub account_token_qry_statement: AnyStatement<'static>,
    pub invalidate_account_token_statement: AnyStatement<'static>,
    pub remove_sessions_except_statement: AnyStatement<'static>,
    pub remove_account_statement: AnyStatement<'static>,
    pub add_cert_statement: AnyStatement<'static>,
    pub get_certs_statement: AnyStatement<'static>,
    pub cleanup_credential_auth_tokens_statement: AnyStatement<'static>,
    pub cleanup_account_tokens_statement: AnyStatement<'static>,
    pub cleanup_certs_statement: AnyStatement<'static>,
    pub unlink_credential_email_statement: AnyStatement<'static>,
    pub unlink_credential_steam_statement: AnyStatement<'static>,
    pub unlink_credential_by_email_statement: AnyStatement<'static>,
    pub unlink_credential_by_steam_statement: AnyStatement<'static>,
    pub account_info: AnyStatement<'static>,
}
