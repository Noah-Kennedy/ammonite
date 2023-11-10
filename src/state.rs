use hyper::client::HttpConnector;
use std::net::SocketAddr;

#[derive(Clone)]
pub struct ProxyState {
    pub client: hyper::Client<HttpConnector>,
    pub remote: SocketAddr,
}
