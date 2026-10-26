// Farm licences (farm board WO-09). One file, three jobs:
//   farm-licence keygen KEYFILE            make the private key once, outside any git work tree
//   STELLA_FARM_LICENCE_KEY=KEYFILE farm-licence issue --name "Maple Street" --printers 10 [--issued 2026-10-12]
//   farm-licence verify "SFL1...."         check a licence against the key built into the program
// The private key never goes in the repository; `keygen` refuses a path inside one, and `issue` reads
// the key only from the file the environment variable names. Build: cargo build --release --example farm-licence
// The service's licence module, whole; this tool uses only part of it.
#[allow(dead_code)]
#[path = "../src/licence.rs"]
mod licence;

use std::{fs, io::Write, path::Path};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}
fn inside_git(path: &Path) -> bool {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    abs.ancestors().skip(1).any(|a| a.join(".git").exists())
}
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86400)
        .unwrap_or(0) as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: farm-licence keygen KEYFILE   (a path outside the repository)");
                std::process::exit(2);
            };
            let p = Path::new(path);
            if inside_git(p) {
                eprintln!("refused: {path} is inside a git work tree. The private key must not be in a repository.");
                std::process::exit(1);
            }
            if p.exists() {
                eprintln!("refused: {path} already exists; a licence key is made once.");
                std::process::exit(1);
            }
            let mut seed = [0_u8; 32];
            getrandom::fill(&mut seed).expect("system randomness");
            let mut f = {
                use std::os::unix::fs::OpenOptionsExt;
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(p)
                    .expect("create the key file")
            };
            writeln!(f, "{}", hex(&seed)).expect("write the key");
            let public = licence::public_key_of(&seed);
            println!("private key written to {path} (mode 600). Back it up somewhere safe; it cannot be recovered.");
            println!("public key (paste into PUBLIC_KEY in src/licence.rs):");
            println!("pub const PUBLIC_KEY: [u8; 32] = {:?};", public);
            println!("hex: {}", hex(&public));
        }
        Some("issue") => {
            let Ok(keyfile) = std::env::var("STELLA_FARM_LICENCE_KEY") else {
                eprintln!(
                    "set STELLA_FARM_LICENCE_KEY to the private key file (outside the repository)"
                );
                std::process::exit(2);
            };
            let Some(seed) = fs::read_to_string(&keyfile).ok().and_then(|t| unhex(&t)) else {
                eprintln!("cannot read a private key from {keyfile}");
                std::process::exit(1);
            };
            let (Some(name), Some(n)) = (
                flag("--name"),
                flag("--printers").and_then(|n| n.parse::<u32>().ok()),
            ) else {
                eprintln!("usage: issue --name \"Who it is for\" --printers N [--issued YYYY-MM-DD] [--expiry YYYY-MM-DD]");
                std::process::exit(2);
            };
            if n <= licence::FREE_PRINTERS as u32 {
                eprintln!(
                    "note: up to {} printers need no licence",
                    licence::FREE_PRINTERS
                );
            }
            let expiry = flag("--expiry");
            let text = if expiry.is_some() {
                licence::issue(
                    &seed,
                    &name,
                    n,
                    &flag("--issued").unwrap_or_else(today),
                    Some(&expiry.unwrap()),
                )
            } else {
                licence::issue(
                    &seed,
                    &name,
                    n,
                    &flag("--issued").unwrap_or_else(today),
                    None,
                )
            };
            if licence::public_key_of(&seed) != licence::PUBLIC_KEY {
                eprintln!("warning: this key is not the one built into the program; the licence will not verify there");
            }
            println!("{text}");
        }
        Some("verify") => match licence::verify(args.get(1).map(String::as_str).unwrap_or("")) {
            Ok(l) => println!(
                "valid: {} ({} printers, issued {})",
                l.name, l.printers, l.issued
            ),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        _ => {
            eprintln!("farm-licence keygen KEYFILE | issue --name N --printers K | verify TEXT");
            std::process::exit(2);
        }
    }
}
