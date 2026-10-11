//! `stella-link serve --key FILE --forward 127.0.0.1:7422 --status FILE --allow FILE`: the desktop side, started
//! by Stella Scope when remote access is on. `stella-link dial ID [--port N]`: a test client.
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("serve") => {
            let key = stella_link::load_or_create_key(&PathBuf::from(arg(&args, "--key").context("--key FILE")?))?;
            let forward = arg(&args, "--forward").unwrap_or_else(|| "127.0.0.1:7422".into()).parse()?;
            let status = arg(&args, "--status").map(PathBuf::from);
            let allow = arg(&args, "--allow").map(PathBuf::from);
            // Leave with Stella Scope: it holds our stdin open, and when it is gone the pipe closes.
            if std::env::var_os("STELLA_WATCH_STDIN").is_some() {
                std::thread::spawn(|| {
                    use std::io::Read;
                    let mut b = [0u8; 64];
                    while matches!(std::io::stdin().read(&mut b), Ok(n) if n > 0) {}
                    std::process::exit(0);
                });
            }
            stella_link::serve(key, forward, allow, move |s| {
                println!("{s}");
                if let Some(p) = &status {
                    let _ = std::fs::write(p, &s);
                }
            })
            .await
        }
        Some("dial") => {
            let id = stella_link::parse_id(args.get(2).context("dial ID")?)?;
            let key = iroh::SecretKey::generate();
            let t = stella_link::Tunnel::start(key, id).await?;
            let want = arg(&args, "--port");
            println!("http://127.0.0.1:{}/ (tunnel to {id}{})", t.port, want.map(|_| ", --port is ignored").unwrap_or(""));
            std::future::pending::<()>().await;
            Ok(())
        }
        _ => bail!("usage: stella-link serve --key FILE [--forward ADDR] [--status FILE] [--allow FILE] | stella-link dial ID"),
    }
}
