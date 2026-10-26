// Fake printers for the scratch instance: `cargo build --release --example fake-printers`, then
//   target/release/examples/fake-printers moonraker 7126
// The servers themselves live in src/fakes.rs (also used by `cargo test`). Control a running fake
// with GET /__log and POST /__set (JSON), see fakes.rs. Not part of the shipped binary.
#[path = "../src/fakes.rs"]
mod fakes;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = args.first().map(String::as_str).unwrap_or("");
    let port: u16 = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let _keep = match kind {
        "moonraker" => {
            let m = fakes::FakeMoonraker::start(port);
            println!("fake moonraker on {}", m.addr);
            Box::new(m) as Box<dyn std::any::Any>
        }
        "prusalink" => {
            let user = args.get(2).map(String::as_str).unwrap_or("maker");
            let pass = args.get(3).map(String::as_str).unwrap_or("secret");
            let p = fakes::FakePrusaLink::start(port, user, pass);
            println!("fake prusalink on {} (user {user})", p.addr);
            Box::new(p) as Box<dyn std::any::Any>
        }
        "flashforge" => {
            // fake-printers flashforge HTTP_PORT TCP_PORT SERIAL CHECK_CODE
            let tcp: u16 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
            let serial = args.get(3).map(String::as_str).unwrap_or("SNFAKE0000001");
            let check = args.get(4).map(String::as_str).unwrap_or("0fake000");
            let f = fakes::FakeFlashForge::start(port, tcp, serial, check);
            if let Some(cam) = args.get(5) {
                f.state.lock().unwrap().camera_url = cam.clone();
            }
            {
                let mut s = f.state.lock().unwrap();
                s.status = "printing".into();
                s.machine_status = "BUILDING_FROM_SD".into();
                s.file = "Benchy_PLA.gcode.3mf".into();
                s.progress = 0.32;
                s.layer = 62;
                s.layers = 240;
                s.nozzle = 220.0;
                s.nozzle_target = 220.0;
                s.bed = 60.0;
                s.bed_target = 60.0;
            }
            println!("fake flashforge http {} tcp {}", f.http_addr, f.tcp_addr);
            Box::new(f) as Box<dyn std::any::Any>
        }
        "mjpeg" => {
            let f = fakes::FakeMjpeg::start(port, 5);
            println!("fake mjpeg camera {}", f.stream_url());
            Box::new(f) as Box<dyn std::any::Any>
        }
        _ => {
            eprintln!("usage: fake-printers moonraker PORT | prusalink PORT [USER PASSWORD] | flashforge HTTP TCP [SERIAL CHECK] | mjpeg PORT");
            std::process::exit(2);
        }
    };
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
