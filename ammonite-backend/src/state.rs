use crate::Resolver;
use hyper::client::HttpConnector;
use std::net::SocketAddr;

#[derive(Clone)]
pub struct ProxyState {
    pub client: hyper::Client<HttpConnector<Resolver>>,
    pub remote: SocketAddr,
}
