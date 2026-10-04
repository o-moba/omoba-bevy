//! Release operator tool. JSON stdout, errors on stderr; no account or match allocation.
use shared::compatibility::{CompatibilityProbe, ReleaseContract};
use std::{
    env, fs,
    net::{ToSocketAddrs, UdpSocket},
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn read_contract(path: &str) -> Result<ReleaseContract, Box<dyn std::error::Error>> {
    let contract: ReleaseContract = serde_json::from_str(&fs::read_to_string(path)?)?;
    if !contract.valid() {
        return Err("invalid contract manifest".into());
    }
    Ok(contract)
}
fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [cmd] if cmd == "manifest" => {
            println!("{}", serde_json::to_string_pretty(&ReleaseContract::current())?);
            Ok(0)
        }
        [cmd, client, server] if cmd == "compare" => {
            report(read_contract(client)?, read_contract(server)?)
        }
        [cmd, addr, manifest] if cmd == "check" => {
            let client = read_contract(manifest)?;
            let peer = addr.to_socket_addrs()?.next().ok_or("no resolved endpoint")?;
            if peer.port() == 0 { return Err("server port must be nonzero".into()); }
            let socket = UdpSocket::bind(if peer.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
            socket.connect(peer)?;
            socket.set_nonblocking(true)?;
            // Correlation only, never advertised as an authentication secret.
            let nonce = format!("{:032x}", SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() ^ u128::from(std::process::id()));
            let probe = CompatibilityProbe::new(client.clone(), nonce);
            let started = Instant::now();
            let mut last_sent = None;
            let mut buffer = [0_u8; shared::compatibility::MAX_PROBE_BYTES + 1];
            while started.elapsed() < Duration::from_secs(4) {
                if last_sent.is_none_or(|at: Instant| at.elapsed() >= Duration::from_millis(500)) {
                    socket.send(&probe.request())?;
                    last_sent = Some(Instant::now());
                }
                match socket.recv(&mut buffer) {
                    Ok(len) => if let Some(server) = probe.accept(&buffer[..len]) { return report(client, server); },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                    Err(error) => return Err(error.into()),
                }
                std::thread::sleep(Duration::from_millis(16));
            }
            println!("{}", serde_json::json!({"compatible":false,"issue":"unavailable","client":client,"server":null,"endpoint":addr}));
            Ok(3)
        }
        _ => Err("usage: compatibility manifest | compare CLIENT.json SERVER.json | check HOST:PORT CLIENT.json".into()),
    }
}
fn report(
    client: ReleaseContract,
    server: ReleaseContract,
) -> Result<u8, Box<dyn std::error::Error>> {
    let issue = client.compare(&server).err();
    println!(
        "{}",
        serde_json::json!({"compatible":issue.is_none(),"issue":issue,"client":client,"server":server})
    );
    Ok(if issue.is_none() { 0 } else { 2 })
}
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            println!(
                "{}",
                serde_json::json!({"compatible":false,"issue":"unavailable","error":error.to_string()})
            );
            ExitCode::from(3)
        }
    }
}
