//! The echo Host and the echo Client on localhost, in one process (two threads).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn client_gets_each_ump_back_from_the_host() {
    let mut host = net_echo::host("127.0.0.1:0", "loopback host").unwrap();
    let addr = host.socket.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let t = std::thread::spawn(move || {
        net_echo::run_echo_host(&mut host, |_| {}, || s.load(Ordering::Relaxed)).unwrap();
    });
    let umps = net_echo::test_notes(16);
    let mut client = net_echo::client("loopback client").unwrap();
    let got = net_echo::run_echo_client(&mut client, addr, &umps, Duration::from_secs(5), |_| {})
        .unwrap();
    // Bye, and the Bye Reply of the Host closes the session (M2-124 6.16, 6.17).
    assert!(net_echo::bye(&mut client, addr, Duration::from_secs(3), |_| {}).unwrap());
    stop.store(true, Ordering::Relaxed);
    t.join().unwrap();
    let want: Vec<Vec<u32>> = umps.iter().map(|u| u.to_vec()).collect();
    assert_eq!(got, want);
}
