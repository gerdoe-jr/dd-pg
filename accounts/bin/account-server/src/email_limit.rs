use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct EmailDomainDenyList {
    domains: HashSet<url::Host>,
}

impl EmailDomainDenyList {
    pub fn is_banned(&self, email: &email_address::EmailAddress) -> bool {
        !url::Host::parse(&email.domain().to_lowercase())
            .is_ok_and(|host| !self.domains.contains(&host))
    }
}

/// Checks if a email domain is allowed.
/// If the list is empty, all domains are allowed.
#[derive(Debug, Default)]
pub struct EmailDomainAllowList {
    domains: HashSet<url::Host>,
}

impl EmailDomainAllowList {
    pub fn is_allowed(&self, email: &email_address::EmailAddress) -> bool {
        self.domains.is_empty()
            || url::Host::parse(&email.domain().to_lowercase())
                .is_ok_and(|host| self.domains.contains(&host))
    }
}
