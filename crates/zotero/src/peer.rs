//! Who answered on Zotero's port. Zotero's protocol has no handshake that proves the other end
//! is Zotero, and whatever answers gets to read and rewrite the open document. Two checks:
//!
//! - **Every system:** WordCraft only connects to this computer (a loopback address).
//! - **Linux only:** the program listening there must not run as another user, from the
//!   kernel's socket table (`/proc/net/tcp`): another account on a shared machine that took the
//!   port while Zotero wasn't running is refused. A listener the table doesn't show (WSL2
//!   mirrored networking with Zotero on Windows, a local proxy, a sandbox) leaves only the
//!   loopback rule.
//!
//! On Windows, macOS and the BSDs there is **no same-user check**: the loopback rule is the
//! only one, so any local account's listener on the port is taken for Zotero. Those systems have
//! no such table to read without OS calls this crate doesn't make (see `docs/zotero.md`, #3831).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};

use crate::ZoteroError;

/// Whether `addr` is on this computer; anything else is refused before connecting.
pub(crate) fn check_addr(addr: &SocketAddr) -> Result<(), ZoteroError> {
    if addr.ip().to_canonical().is_loopback() {
        Ok(())
    } else {
        Err(ZoteroError::Refused(format!("{addr} is not on this computer; WordCraft only talks to Zotero on 127.0.0.1")))
    }
}

/// Refuse the program at the other end of `stream` when it may not be ours (see the module
/// documentation).
pub(crate) fn check(stream: &TcpStream) -> Result<(), ZoteroError> {
    let io = |e: std::io::Error| ZoteroError::Io(e.to_string());
    let (local, peer) = (stream.local_addr().map_err(io)?, stream.peer_addr().map_err(io)?);
    check_addr(&peer)?;
    owner_check(local, peer)
}

/// Refuse a listener at `peer` that the kernel's socket tables say belongs to another user.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn owner_check(local: SocketAddr, peer: SocketAddr) -> Result<(), ZoteroError> {
    use std::io::Read;
    /// Longest socket table read (a busy server's table is a few MB).
    const MAX_TABLE: u64 = 64 * 1024 * 1024;
    let mut table = String::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(f) = std::fs::File::open(path) {
            let mut part = String::new();
            if f.take(MAX_TABLE).read_to_string(&mut part).is_ok() {
                table.push_str(&part);
            }
        }
    }
    // Our uid when our own socket's row is missing: `/proc/self` belongs to this process's user.
    let me = std::fs::metadata("/proc/self").ok().map(|m| std::os::unix::fs::MetadataExt::uid(&m));
    same_owner(&table, local, peer, me)
}

/// Windows, macOS, the BSDs: no same-user check (no socket table to read without OS calls this
/// crate doesn't make, #3831); the loopback rule alone decides.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn owner_check(_local: SocketAddr, _peer: SocketAddr) -> Result<(), ZoteroError> {
    Ok(())
}

/// From a `/proc/net/tcp`-format socket table: refuse when the socket at `peer` connected to
/// `local` (the listener's side) belongs to another user than ours. Our uid is our own socket's
/// row, else `me` (this process's). When the listener isn't listed (or the table couldn't be
/// read, or our uid is unknown) its owner is unknown and the loopback rule decides.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
pub(crate) fn same_owner(table: &str, local: SocketAddr, peer: SocketAddr, me: Option<u32>) -> Result<(), ZoteroError> {
    let key = |a: SocketAddr| (a.ip().to_canonical(), a.port());
    let (local, peer) = (key(local), key(peer));
    let (mut ours, mut theirs) = (None, None);
    for line in table.lines() {
        let f: Vec<&str> = line.split_whitespace().take(8).collect();
        let (Some(l), Some(r), Some(uid)) = (f.get(1).and_then(|a| parse_addr(a)), f.get(2).and_then(|a| parse_addr(a)), f.get(7)) else {
            continue;
        };
        let Ok(uid) = uid.parse::<u32>() else { continue };
        let (l, r) = (key(l), key(r));
        if l == local && r == peer {
            ours = Some(uid);
        } else if l == peer && r == local {
            theirs = Some(uid);
        }
    }
    match (ours.or(me), theirs) {
        (Some(a), Some(b)) if a == b => Ok(()),
        (Some(_), Some(b)) => {
            Err(ZoteroError::Refused(format!("the program on port {} runs as another user (uid {b}), so it isn't your Zotero", peer.1)))
        }
        _ => {
            log::debug!(
                "Zotero: couldn't tell which user runs the program on port {} (not in the socket table); trusting the loopback address alone",
                peer.1
            );
            Ok(())
        }
    }
}

/// `0100007F:5A5C` (IPv4) or 32 hex digits and a port (IPv6): each 32-bit word as the kernel
/// prints it, in this machine's byte order.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
fn parse_addr(s: &str) -> Option<SocketAddr> {
    let (ip, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let word = |k: usize| ip.get(k * 8..k * 8 + 8).and_then(|w| u32::from_str_radix(w, 16).ok()).map(u32::to_ne_bytes);
    let ip = match ip.len() {
        8 => IpAddr::V4(Ipv4Addr::from(word(0)?)),
        32 => {
            let mut b = [0u8; 16];
            for (k, w) in b.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *w = word(k)?;
            }
            IpAddr::V6(Ipv6Addr::from(b))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `a` the way the kernel's socket table prints it.
    fn hex(a: SocketAddr) -> String {
        let ip = match a.ip() {
            IpAddr::V4(v4) => format!("{:08X}", u32::from_ne_bytes(v4.octets())),
            IpAddr::V6(v6) => v6.octets().as_chunks::<4>().0.iter().map(|w| format!("{:08X}", u32::from_ne_bytes(*w))).collect(),
        };
        format!("{ip}:{:04X}", a.port())
    }

    /// A socket table row for the connection from `l` to `r`, owned by `uid`.
    fn row(l: SocketAddr, r: SocketAddr, uid: u32) -> String {
        format!("   0: {} {} 01 00000000:00000000 00:00000000 00000000  {uid:>4}        0 12345 1 0000000000000000 20 4 30 10 -1\n", hex(l), hex(r))
    }

    const HEADER: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";

    /// Our side and Zotero's listening side of a loopback connection.
    fn addrs() -> (SocketAddr, SocketAddr) {
        ("127.0.0.1:41234".parse().unwrap(), "127.0.0.1:23116".parse().unwrap())
    }

    #[test]
    fn addresses_parse_as_the_kernel_prints_them() {
        for a in ["127.0.0.1:23116", "[::1]:23116", "[::ffff:127.0.0.1]:80"] {
            let a: SocketAddr = a.parse().unwrap();
            assert_eq!(parse_addr(&hex(a)), Some(a));
        }
        for bad in ["", "0100007F", "0100007F:", "XYZ00000:0050", "0100007F:12345", "0100:0050"] {
            assert_eq!(parse_addr(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_listener_of_another_user_is_refused() {
        let (local, peer) = addrs();
        let same = format!("{HEADER}{}{}", row(local, peer, 1000), row(peer, local, 1000));
        assert_eq!(same_owner(&same, local, peer, None), Ok(()));
        let other = format!("{HEADER}{}{}", row(local, peer, 1000), row(peer, local, 1001));
        assert!(matches!(same_owner(&other, local, peer, None), Err(ZoteroError::Refused(e)) if e.contains("another user")));
        // Junk lines are skipped.
        let junk = format!("{HEADER}garbage\n 1: zz:zz yy:yy 01 x x x x\n{}{}", row(local, peer, 7), row(peer, local, 7));
        assert_eq!(same_owner(&junk, local, peer, None), Ok(()));
    }

    /// WSL2 mirrored networking (Zotero on Windows), a local proxy or a sandbox: the listener
    /// isn't in our table, so the loopback rule decides, as on other systems.
    #[test]
    fn an_unlisted_listener_falls_back_to_the_loopback_rule() {
        let (local, peer) = addrs();
        let missing = format!("{HEADER}{}", row(local, peer, 1000));
        assert_eq!(same_owner(&missing, local, peer, None), Ok(()));
        // The table couldn't be read.
        assert_eq!(same_owner("", local, peer, None), Ok(()));
        assert_eq!(same_owner("", local, peer, Some(1000)), Ok(()));
    }

    /// Our own socket's row missing doesn't hide another user's listener: our uid comes from
    /// the process itself.
    #[test]
    fn a_listed_listener_is_checked_against_our_process_uid() {
        let (local, peer) = addrs();
        let only_theirs = format!("{HEADER}{}", row(peer, local, 1001));
        assert!(matches!(same_owner(&only_theirs, local, peer, Some(1000)), Err(ZoteroError::Refused(e)) if e.contains("another user")));
        assert_eq!(same_owner(&only_theirs, local, peer, Some(1001)), Ok(()));
        // Neither our row nor our process uid: unknown, so the loopback rule decides.
        assert_eq!(same_owner(&only_theirs, local, peer, None), Ok(()));
        // Our socket's row is ours by definition and wins over the process uid.
        let both = format!("{HEADER}{}{}", row(local, peer, 1001), row(peer, local, 1001));
        assert_eq!(same_owner(&both, local, peer, Some(1000)), Ok(()));
    }

    #[test]
    fn a_dual_stack_listener_is_matched_through_its_mapped_address() {
        let (local, peer) = addrs();
        let mapped = |a: SocketAddr| match a.ip() {
            IpAddr::V4(v4) => SocketAddr::new(IpAddr::V6(v4.to_ipv6_mapped()), a.port()),
            IpAddr::V6(_) => a,
        };
        let table = format!("{HEADER}{}{}", row(local, peer, 1000), row(mapped(peer), mapped(local), 1001));
        assert!(matches!(same_owner(&table, local, peer, None), Err(ZoteroError::Refused(e)) if e.contains("another user")));
    }

    #[test]
    fn only_this_computer_is_contacted() {
        for ok in ["127.0.0.1:23116", "127.9.9.9:1", "[::1]:23116", "[::ffff:127.0.0.1]:23116"] {
            assert_eq!(check_addr(&ok.parse().unwrap()), Ok(()), "{ok}");
        }
        for bad in ["192.0.2.1:23116", "0.0.0.0:23116", "[2001:db8::1]:23116", "[::]:23116"] {
            assert!(matches!(check_addr(&bad.parse().unwrap()), Err(ZoteroError::Refused(_))), "{bad}");
        }
    }

    #[test]
    fn another_computer_is_refused_before_connecting() {
        let opts = crate::client::Options { addr: "192.0.2.1:23116".parse().unwrap(), ..Default::default() };
        let r = crate::client::run_command(&opts, crate::Command::Refresh, &mut |_| Ok(serde_json::Value::Null));
        assert!(matches!(r, Err(ZoteroError::Refused(_))), "{r:?}");
    }

    /// A listener this process opened runs as us: the real socket table agrees.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn our_own_listener_passes_the_owner_check() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _accepted = listener.accept().unwrap();
        assert_eq!(check(&stream), Ok(()));
    }
}
