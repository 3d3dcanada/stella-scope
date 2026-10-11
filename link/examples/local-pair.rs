//! Local end-to-end check of the phone tunnel, never shipped: for each `--forward ADDR` (a scratch
//! Stella Scope phone listener, optionally `ADDR=ALLOW_FILE` for its admission file) it binds a
//! desktop endpoint and a phone tunnel on 127.0.0.1 with relays, address lookup and port mapping off,
//! and prints one line `ADDR PHONE_URL LINK_ID PHONE_ID`.
//! Nothing leaves this computer. A real phone, relay and NAT remain separate checks.
use iroh::{
    endpoint::{presets, PortmapperConfig, RelayMode},
    Endpoint, EndpointAddr, SecretKey,
};
use std::net::SocketAddr;

async fn local(alpn: bool) -> anyhow::Result<Endpoint> {
    let mut b = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .portmapper_config(PortmapperConfig::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")?
        .secret_key(SecretKey::generate());
    if alpn {
        b = b.alpns(vec![stella_link::ALPN.to_vec()]);
    }
    Ok(b.bind().await?)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let forwards: Vec<(SocketAddr, Option<std::path::PathBuf>)> = args.windows(2).filter(|w| w[0] == "--forward")
        .map(|w| { let (a, f) = w[1].split_once('=').map_or((w[1].as_str(), None), |(a, f)| (a, Some(f.into()))); a.parse().map(|a| (a, f)) })
        .collect::<Result<_, _>>()?;
    anyhow::ensure!(!forwards.is_empty(), "usage: local-pair --forward 127.0.0.1:PORT [--forward ...]");
    for (forward, allow) in forwards {
        let desk = local(true).await?;
        let socket = desk.bound_sockets().into_iter().find(SocketAddr::is_ipv4).ok_or_else(|| anyhow::anyhow!("no loopback socket"))?;
        let addr = EndpointAddr::new(desk.id()).with_ip_addr(socket);
        let id = desk.id();
        tokio::spawn(stella_link::serve_endpoint(desk, forward, allow, |_| {}));
        let tunnel = stella_link::Tunnel::start_endpoint(local(false).await?, addr, 0).await?;
        println!("{forward} http://127.0.0.1:{}/ {id} {}", tunnel.port, tunnel.me);
    }
    std::future::pending::<()>().await;
    Ok(())
}
