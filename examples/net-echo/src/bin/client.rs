//! Network MIDI 2.0 echo Client: `cargo run --bin client -- [HOST:PORT | --browse] [COUNT]`.
//! `--browse` (the default) finds the Hosts with DNS-SD and takes the first one. It connects,
//! sends COUNT Note On and Note Off pairs (default 8) and checks that the Host sends each UMP
//! back, then ends the session with Bye. Exit status 1 when an UMP does not come back in 5 s.

use std::net::ToSocketAddrs;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let target = args.next().unwrap_or_else(|| "--browse".into());
    let host = if target == "--browse" {
        let found = net_echo::discovery::browse(Duration::from_secs(2))?;
        for f in &found {
            println!(
                "found \"{}\" ({:?}) at {}",
                f.instance, f.endpoint_name, f.addr
            );
        }
        found.first().ok_or("no Network MIDI 2.0 Host found")?.addr
    } else {
        target.to_socket_addrs()?.next().ok_or("no address")?
    };
    let count: u8 = args.next().map(|c| c.parse()).transpose()?.unwrap_or(8);
    let umps = net_echo::test_notes(count);
    let mut node = net_echo::client("zetaohm-midi echo client")?;
    let got = net_echo::run_echo_client(&mut node, host, &umps, Duration::from_secs(5), |e| {
        println!("{e:?}")
    })?;
    let closed = net_echo::bye(&mut node, host, Duration::from_secs(3), |e| {
        println!("{e:?}")
    })?;
    println!("session closed: {closed}");
    let want: Vec<Vec<u32>> = umps.iter().map(|u| u.to_vec()).collect();
    if got == want {
        println!("all {} UMPs came back", want.len());
        Ok(())
    } else {
        eprintln!("sent {want:08x?}\ngot  {got:08x?}");
        std::process::exit(1);
    }
}
