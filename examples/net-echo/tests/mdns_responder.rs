//! The mDNS responder of `examples/esp32s3-net-echo` (the same source file), read by the browser
//! of this example: so the two ends of DNS-SD are tested together on the host.

#[path = "../../esp32s3-net-echo/src/mdns.rs"]
#[allow(dead_code)]
mod mdns;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use zmidi_net::dns::{HostRecords, Hostname, ServiceInstanceName};
use zmidi_net::Identity;
use net_echo::discovery::{query, Answers, Found};

const IP: [u8; 4] = [10, 0, 0, 42];

fn responder() -> mdns::Responder {
    let records = HostRecords::new(
        ServiceInstanceName::new("zetaohm-midi esp32s3").unwrap(),
        Identity::new("zetaohm-midi esp32s3", "esp32s3-1").unwrap(),
        5673,
        Hostname::new("zetaohm-esp32s3.local.").unwrap(),
    );
    mdns::Responder::new(&records, &Hostname::new("zetaohm-esp32s3.local.").unwrap(), IP).unwrap()
}

fn expected() -> Vec<Found> {
    vec![Found {
        instance: "zetaohm-midi esp32s3".into(),
        endpoint_name: Some("zetaohm-midi esp32s3".into()),
        // The A record gives the address, not the sender of the reply.
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::from(IP)), 5673),
    }]
}

fn sender() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9))
}

fn first_ttl(m: &[u8]) -> u32 {
    // Skip the header, the questions and the first record name.
    let qd = u16::from_be_bytes([m[4], m[5]]);
    let mut at = 12;
    for _ in 0..qd {
        at = zmidi_net::dns::read_name(m, at).unwrap().1 + 4;
    }
    let end = zmidi_net::dns::read_name(m, at).unwrap().1;
    u32::from_be_bytes([m[end + 4], m[end + 5], m[end + 6], m[end + 7]])
}

#[test]
fn announcement_gives_the_host() {
    let mut out = [0u8; mdns::MAX_REPLY];
    let n = responder().announcement(&mut out).unwrap();
    let mut a = Answers::default();
    a.read(&out[..n], sender());
    assert_eq!(a.found(), expected());
    assert_eq!(first_ttl(&out[..n]), 60); // REQ-NET-DISC-014
}

#[test]
fn one_shot_query_gets_a_unicast_reply() {
    // RFC 6762 6.7: the query ID, the question, a TTL of at most 10 s.
    let q = query();
    let mut out = [0u8; mdns::MAX_REPLY];
    let (n, unicast) = responder().answer(&q, 50000, &mut out).unwrap();
    assert!(unicast);
    assert_eq!(&out[..2], &q[..2]);
    assert_eq!(u16::from_be_bytes([out[4], out[5]]), 1);
    assert_eq!(first_ttl(&out[..n]), 10);
    let mut a = Answers::default();
    a.read(&out[..n], sender());
    assert_eq!(a.found(), expected());
}

#[test]
fn query_from_port_5353_gets_a_multicast_reply() {
    let mut out = [0u8; mdns::MAX_REPLY];
    let (n, unicast) = responder().answer(&query(), 5353, &mut out).unwrap();
    assert!(!unicast);
    assert_eq!(&out[..2], &[0, 0]);
    assert_eq!(u16::from_be_bytes([out[4], out[5]]), 0);
    let mut a = Answers::default();
    a.read(&out[..n], sender());
    assert_eq!(a.found(), expected());
}

#[test]
fn other_questions_and_replies_get_nothing() {
    let r = responder();
    let mut out = [0u8; mdns::MAX_REPLY];
    // A question for "_http._tcp.local." PTR.
    let mut q = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    q.extend_from_slice(b"\x05_http\x04_tcp\x05local\x00\x00\x0c\x00\x01");
    assert!(r.answer(&q, 50000, &mut out).is_none());
    // A reply (QR bit set) is not a query.
    let mut reply = query();
    reply[2] = 0x84;
    assert!(r.answer(&reply, 5353, &mut out).is_none());
    // Bytes that are not a DNS message.
    assert!(r.answer(&[1, 2, 3], 5353, &mut out).is_none());
    // The hostname A question is answered, in any case of letters.
    let mut q = vec![0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    q.extend_from_slice(b"\x0fZETAOHM-esp32s3\x05local\x00\x00\x01\x00\x01");
    assert!(r.answer(&q, 50000, &mut out).is_some());
}
