use std::{net::SocketAddr, sync::Arc};

use accounts_shared::account_server::{
    errors::AccountServerRequestError, result::AccountServerReqResult,
};
use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::Request,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use parking_lot::RwLock;
use reqwest::StatusCode;

#[derive(Debug, Default)]
pub struct IpDenyList {
    ipv4: iprange::IpRange<ipnet::Ipv4Net>,
    ipv6: iprange::IpRange<ipnet::Ipv6Net>,
}

impl IpDenyList {
    pub fn is_banned(&self, addr: SocketAddr) -> bool {
        match addr {
            SocketAddr::V4(ip) => self.ipv4.contains(&ipnet::Ipv4Net::from(*ip.ip())),
            SocketAddr::V6(ip) => self.ipv6.contains(&ipnet::Ipv6Net::from(*ip.ip())),
        }
    }
}

pub async fn ip_deny_layer(
    State(deny_list): State<Arc<RwLock<IpDenyList>>>,
    ConnectInfo(client_ip): ConnectInfo<SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Result<Response<Body>, StatusCode> {
    if deny_list.read().is_banned(client_ip) {
        Ok(Json(AccountServerReqResult::<(), ()>::Err(
            AccountServerRequestError::VpnBan(
                "VPN detected. Please deactivate the VPN and try again.".to_string(),
            ),
        ))
        .into_response())
    } else {
        Ok(next.run(req).await)
    }
}
