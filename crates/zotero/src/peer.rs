//! Who answered on Zotero's port. Zotero's protocol has no handshake that proves the other end
//! is Zotero, and whatever answers gets to read and rewrite the open document. Two checks:
//!
//! - **Every system:** WordCraft only connects to this computer (a loopback address).
//! - **Linux only:** the program listening there must not run as another user, from the
//!   kernel's socket table (`/proc/net/tcp`): another account on a shared machine that took the
//!   port while Zotero wasn't running is refused. A listener the tables don't show (WSL2
//!   mirrored networking with Zotero on Windows, a local proxy) leaves only the loopback rule,
//!   but only when the tables were read in full: a read error, or an open error other than
//!   "not found" (e.g. a sandbox profile that denies `/proc/net/tcp`), refuses.
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
/// A table that can't be read in full (I/O error) is a refusal, never "unknown".
#[cfg(any(target_os = "linux", target_os = "android"))]
fn owner_check(local: SocketAddr, peer: SocketAddr) -> Result<(), ZoteroError> {
    let rows = scan_tables(["/proc/net/tcp", "/proc/net/tcp6"], |p| std::fs::File::open(p), local, peer)?;
    // Our uid when our own socket's row is missing: `/proc/self` belongs to this process's user.
    let me = std::fs::metadata("/proc/self").ok().map(|m| std::os::unix::fs::MetadataExt::uid(&m));
    decide(&rows, peer, me)
}

/// Windows, macOS, the BSDs: no same-user check (no socket table to read without OS calls this
/// crate doesn't make, #3831); the loopback rule alone decides.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn owner_check(_local: SocketAddr, _peer: SocketAddr) -> Result<(), ZoteroError> {
    Ok(())
}

/// Scan the tables named by `paths` (opened by `open`) in order, stopping once both rows are
/// found. A table that doesn't exist (no IPv6) is skipped; any other open or read error refuses.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
fn scan_tables<R: std::io::Read>(
    paths: [&str; 2],
    open: impl Fn(&str) -> std::io::Result<R>,
    local: SocketAddr,
    peer: SocketAddr,
) -> Result<Rows, ZoteroError> {
    let mut rows = Rows::default();
    for path in paths {
        match open(path) {
            Ok(f) => scan_table(std::io::BufReader::new(f), local, peer, &mut rows)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(unreadable(&e)),
        }
        if rows.done() {
            break;
        }
    }
    Ok(rows)
}

/// The uids found for the two rows of one connection, and of the socket listening on the
/// listener's address (which speaks for a connection it hasn't accepted yet).
#[derive(Default)]
struct Rows {
    ours: Option<u32>,
    theirs: Option<u32>,
    listener: Option<u32>,
    /// Listening rows on that address with different uids.
    listeners_differ: bool,
}

impl Rows {
    fn done(&self) -> bool {
        self.ours.is_some() && self.theirs.is_some()
    }
}

fn unreadable(e: &std::io::Error) -> ZoteroError {
    ZoteroError::Refused(format!("couldn't read the system's socket table ({e}), so the program on the Zotero port can't be checked"))
}

/// Stream a `/proc/net/tcp`-format table line by line (nothing is kept, so the table's size
/// doesn't matter and nothing is truncated), recording the uids of the connection's two rows.
/// Stops once both are found. Any read error is a refusal: a table we couldn't finish reading
/// must never count as "the listener isn't listed".
fn scan_table(mut table: impl std::io::BufRead, local: SocketAddr, peer: SocketAddr, rows: &mut Rows) -> Result<(), ZoteroError> {
    let key = |a: SocketAddr| (a.ip().to_canonical(), a.port());
    let (local, peer) = (key(local), key(peer));
    let mut line = Vec::new();
    loop {
        line.clear();
        // Bounded line: a row is ~150 bytes; read at most 4 KiB of one (the rest is skipped).
        let n = read_line_capped(&mut table, &mut line).map_err(|e| unreadable(&e))?;
        if n == 0 {
            return Ok(());
        }
        let line = String::from_utf8_lossy(&line);
        let mut f = line.split_whitespace();
        let (Some(l), Some(r), Some(state), Some(uid), Some(inode)) =
            (f.nth(1).and_then(parse_addr), f.next().and_then(parse_addr), f.next(), f.nth(3), f.nth(1))
        else {
            continue;
        };
        let Ok(uid) = uid.parse::<u32>() else { continue };
        let (l, r) = (key(l), key(r));
        // The listening socket (`0A`) on the listener's port, at its address or every address.
        if state == "0A" {
            if l.1 == peer.1 && (l.0 == peer.0 || l.0.is_unspecified()) {
                rows.listeners_differ |= rows.listener.is_some_and(|u| u != uid);
                rows.listener = Some(uid);
            }
            continue;
        }
        // A row with no socket file (inode 0) belongs to no program: a connection still waiting
        // to be accepted, or one that ended (TIME_WAIT). Its uid column reads 0 whoever
        // listens, so it says nothing about the owner; the listening row speaks for it.
        if inode == "0" {
            continue;
        }
        if l == local && r == peer {
            rows.ours = Some(uid);
        } else if l == peer && r == local {
            rows.theirs = Some(uid);
        }
        if rows.done() {
            return Ok(());
        }
    }
}

/// Read one line into `out`, keeping at most 4 KiB of it; returns the bytes consumed (0 at the end).
fn read_line_capped(r: &mut impl std::io::BufRead, out: &mut Vec<u8>) -> std::io::Result<usize> {
    const CAP: usize = 4096;
    let mut total = 0;
    loop {
        let buf = match r.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            return Ok(total);
        }
        let (take, found) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (buf.len(), false),
        };
        let keep = take.min(CAP.saturating_sub(out.len()));
        out.extend_from_slice(buf.get(..keep).unwrap_or_default());
        r.consume(take);
        total += take;
        if found {
            return Ok(total);
        }
    }
}

/// Refuse when the connection's rows show another user. Our uid is our own socket's row, else
/// `me` (this process's). When the listener isn't listed in a table that was read in full (WSL2,
/// a proxy) or our uid is unknown, its owner is unknown and the loopback rule decides.
fn decide(rows: &Rows, peer: SocketAddr, me: Option<u32>) -> Result<(), ZoteroError> {
    if rows.listeners_differ && rows.theirs.is_none() {
        return Err(ZoteroError::Refused(format!("more than one user listens on port {}, so the program there may not be your Zotero", peer.port())));
    }
    match (rows.ours.or(me), rows.theirs.or(rows.listener)) {
        (Some(a), Some(b)) if a == b => Ok(()),
        (Some(_), Some(b)) => {
            Err(ZoteroError::Refused(format!("the program on port {} runs as another user (uid {b}), so it isn't your Zotero", peer.port())))
        }
        _ => {
            log::debug!(
                "Zotero: couldn't tell which user runs the program on port {} (not in the socket table); trusting the loopback address alone",
                peer.port()
            );
            Ok(())
        }
    }
}

/// From an in-memory `/proc/net/tcp`-format table (tests and callers with a string): see
/// [`scan_table`] and [`decide`].
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn same_owner(table: &str, local: SocketAddr, peer: SocketAddr, me: Option<u32>) -> Result<(), ZoteroError> {
    let mut rows = Rows::default();
    scan_table(table.as_bytes(), local, peer, &mut rows)?;
    decide(&rows, peer, me)
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

    /// A connection the listener hasn't accepted yet (or one that ended, TIME_WAIT) has no
    /// socket file: its row reads uid 0 whoever listens. The listening row speaks for it, so a
    /// busy Zotero isn't taken for another user's program, and another user's still is.
    #[test]
    fn an_unaccepted_connection_is_judged_by_its_listening_socket() {
        let (local, peer) = addrs();
        let unowned =
            |l: SocketAddr, r: SocketAddr, state: &str| row(l, r, 0).replacen(" 01 ", &format!(" {state} "), 1).replacen(" 12345 ", " 0 ", 1);
        let listening = |uid: u32, any: bool| {
            let at: SocketAddr = if any { SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), peer.port()) } else { peer };
            row(at, "0.0.0.0:0".parse().unwrap(), uid).replacen(" 01 ", " 0A ", 1)
        };
        for state in ["01", "06"] {
            let mine = format!("{HEADER}{}{}{}", listening(1000, false), row(local, peer, 1000), unowned(peer, local, state));
            assert_eq!(same_owner(&mine, local, peer, None), Ok(()), "{state}");
            let mine_any = format!("{HEADER}{}{}{}", row(local, peer, 1000), unowned(peer, local, state), listening(1000, true));
            assert_eq!(same_owner(&mine_any, local, peer, None), Ok(()), "{state}");
            let other = format!("{HEADER}{}{}{}", row(local, peer, 1000), unowned(peer, local, state), listening(1001, false));
            assert!(matches!(same_owner(&other, local, peer, None), Err(ZoteroError::Refused(e)) if e.contains("another user")), "{state}");
            // No listening row: unknown, so the loopback rule decides.
            let unlisted = format!("{HEADER}{}{}", row(local, peer, 1000), unowned(peer, local, state));
            assert_eq!(same_owner(&unlisted, local, peer, None), Ok(()), "{state}");
        }
        // An accepted connection's own row wins over the listening row.
        let accepted = format!("{HEADER}{}{}{}", listening(1000, false), row(local, peer, 1000), row(peer, local, 1001));
        assert!(matches!(same_owner(&accepted, local, peer, None), Err(ZoteroError::Refused(e)) if e.contains("another user")));
        // A listening row on another port is not the listener.
        let elsewhere = row("127.0.0.1:9".parse().unwrap(), "0.0.0.0:0".parse().unwrap(), 1001).replacen(" 01 ", " 0A ", 1);
        let table = format!("{HEADER}{elsewhere}{}{}", row(local, peer, 1000), unowned(peer, local, "01"));
        assert_eq!(same_owner(&table, local, peer, None), Ok(()));
        // Two users listening on the address: refused.
        let both = format!("{HEADER}{}{}{}{}", listening(1000, false), listening(1001, true), row(local, peer, 1000), unowned(peer, local, "01"));
        assert!(matches!(same_owner(&both, local, peer, None), Err(ZoteroError::Refused(_))));
    }

    /// WSL2 mirrored networking (Zotero on Windows) or a local proxy: the listener
    /// isn't in our table, so the loopback rule decides, as on other systems.
    #[test]
    fn an_unlisted_listener_falls_back_to_the_loopback_rule() {
        let (local, peer) = addrs();
        let missing = format!("{HEADER}{}", row(local, peer, 1000));
        assert_eq!(same_owner(&missing, local, peer, None), Ok(()));
        // An empty table: the listener isn't listed.
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

    /// A reader that yields `data`, then fails.
    struct Failing(std::io::Cursor<Vec<u8>>);
    impl std::io::Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.read(buf)? {
                0 => Err(std::io::Error::other("injected")),
                n => Ok(n),
            }
        }
    }

    /// Repeats `unit` for `left` bytes, then ends: a huge table without holding it in memory.
    struct Table {
        parts: Vec<(Vec<u8>, usize)>,
        part: usize,
        at: usize,
    }
    impl std::io::Read for Table {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some((unit, times)) = self.parts.get(self.part) else { return Ok(0) };
            if self.at >= unit.len() * times {
                self.part += 1;
                self.at = 0;
                return self.read(buf);
            }
            let n = buf.len().min(unit.len() - self.at % unit.len());
            let off = self.at % unit.len();
            buf[..n].copy_from_slice(&unit[off..off + n]);
            self.at += n;
            Ok(n)
        }
    }

    /// The real attack: a hostile user floods the table with listener rows so the connection's
    /// rows sit past any fixed cap. The real read path (`scan_tables`, which `owner_check`
    /// calls on `/proc/net/tcp*`) finds them wherever they are.
    #[test]
    fn rows_far_into_a_huge_table_are_still_found() {
        let (local, peer) = addrs();
        let filler = row("127.0.0.1:1".parse().unwrap(), "0.0.0.0:0".parse().unwrap(), 1001);
        // More than 64 MiB of rows (the old cap), then the connection's two rows.
        let times = 70 * 1024 * 1024 / filler.len();
        assert!(filler.len() * times > 64 * 1024 * 1024);
        let tail = format!("{}{}", row(local, peer, 1000), row(peer, local, 1001)).into_bytes();
        let open = |p: &str| match p {
            "t4" => {
                Ok(Table { parts: vec![(HEADER.as_bytes().to_vec(), 1), (filler.clone().into_bytes(), times), (tail.clone(), 1)], part: 0, at: 0 })
            }
            _ => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        };
        let rows = scan_tables(["t4", "t6"], open, local, peer).unwrap();
        assert_eq!((rows.ours, rows.theirs), (Some(1000), Some(1001)));
        assert!(matches!(decide(&rows, peer, None), Err(ZoteroError::Refused(e)) if e.contains("another user")));
    }

    /// A signal interrupting a read is retried, not taken for an unreadable table.
    #[test]
    fn an_interrupted_read_is_retried() {
        struct Eintr(bool, std::io::Cursor<Vec<u8>>);
        impl std::io::Read for Eintr {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if !std::mem::replace(&mut self.0, true) {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                self.1.read(buf)
            }
        }
        let (local, peer) = addrs();
        let data = format!("{HEADER}{}{}", row(local, peer, 7), row(peer, local, 7)).into_bytes();
        let open = |_: &str| Ok(Eintr(false, std::io::Cursor::new(data.clone())));
        let rows = scan_tables(["a", "b"], open, local, peer).unwrap();
        assert_eq!((rows.ours, rows.theirs), (Some(7), Some(7)));
    }

    /// A sandbox that denies the table (not "not found") refuses.
    #[test]
    fn a_table_that_cannot_be_opened_is_refused() {
        let (local, peer) = addrs();
        let open = |_: &str| -> std::io::Result<&[u8]> { Err(std::io::ErrorKind::PermissionDenied.into()) };
        assert!(matches!(scan_tables(["a", "b"], open, local, peer), Err(ZoteroError::Refused(_))));
    }

    /// A table that fails mid-read is a refusal, not "listener unknown, trust loopback".
    #[test]
    fn a_table_that_fails_to_read_is_refused() {
        let (local, peer) = addrs();
        let data = format!("{HEADER}{}", row(local, peer, 1000)).into_bytes();
        let mut rows = Rows::default();
        let r = scan_table(std::io::BufReader::new(Failing(std::io::Cursor::new(data))), local, peer, &mut rows);
        assert!(matches!(r, Err(ZoteroError::Refused(ref e)) if e.contains("socket table")), "{r:?}");
    }

    #[test]
    fn an_overlong_line_is_skipped_without_gluing_to_the_next() {
        let (local, peer) = addrs();
        let table = format!("{HEADER}{}\n{}{}", "x".repeat(100_000), row(local, peer, 1000), row(peer, local, 1001));
        assert!(matches!(same_owner(&table, local, peer, None), Err(ZoteroError::Refused(_))));
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
