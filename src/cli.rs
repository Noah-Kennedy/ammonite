use clap::Parser;
use std::net::SocketAddr;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long)]
    pub bind: SocketAddr,
    #[arg(short, long)]
    pub remote: SocketAddr,
    #[arg(short, long)]
    pub metrics: SocketAddr,
}
