//! Network MIDI 2.0 echo Host: `cargo run --bin host -- [BIND] [INSTANCE]` (default
//! `0.0.0.0:5673` and "zetaohm-midi echo"). It accepts each Client, sends each received UMP back,
//! and announces itself with DNS-SD through the platform service (M2-124 section 4).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let bind = args.next().unwrap_or_else(|| "0.0.0.0:5673".into());
    let instance = args.next().unwrap_or_else(|| "zetaohm-midi echo".into());
    let mut node = net_echo::host(&bind, "zetaohm-midi echo host")?;
    let port = node.socket.local_addr()?.port();
    println!("host on {}", node.socket.local_addr()?);
    let hostname = format!("{}.local.", local_hostname());
    let records = node.endpoint.host_records(&instance, port, &hostname)?;
    let _announce = match net_echo::discovery::register(&records) {
        Some(child) => {
            println!("DNS-SD: \"{instance}\" _midi2._udp port {port}");
            Some(child)
        }
        None => {
            println!("DNS-SD: no platform tool (dns-sd, avahi-publish-service); not announced");
            None
        }
    };
    net_echo::run_echo_host(&mut node, |e| println!("{e:?}"), || false)?;
    Ok(())
}

/// The first label of the host name, for the SRV target text field.
fn local_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|h| h.trim().split('.').next().map(str::to_owned))
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".into())
}
