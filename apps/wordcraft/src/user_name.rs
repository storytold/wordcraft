//! The user name a first start takes (#2209): the system account's, so the account badge,
//! comments and tracked changes carry the writer's name from the start rather than the default
//! "WordCraft User". A name set in File › Options is saved and wins from then on (`main`).
//!
//! - Windows: the account's first and last name, else its display name (only a domain or work
//!   account has one), read through the Windows Runtime's `Windows.System.User`: a safe API,
//!   where the Win32 `GetUserNameExW` needs unsafe, which our workspace forbids. Without either
//!   (an account with no name, or account info access turned off in Settings › Privacy), the
//!   logon name.
//! - Linux and the BSDs: the full name in the running account's `/etc/passwd` entry — the entry
//!   with our uid where the system tells us one, else the one named by the logon name (the GECOS
//!   field up to its first comma, as `chfn` sets it). Accounts from a directory service (LDAP,
//!   SSSD) have no entry there and take the logon name.
//! - macOS: the full name Directory Services keeps for the account, from the system's own lookup
//!   (`id -F`), else the logon name. `objc2-app-kit` binds no `NSFullUserName`, and reading
//!   Directory Services directly needs unsafe, which our workspace forbids.
//! - Runs without preferences (`WORDCRAFT_NO_PREFS`: agents' and screenshot runs) keep the
//!   default, so what they save or show never carries the name of whoever ran them (`main` checks).
//!
//! Every source is untrusted: the name still goes through `file.setAuthor`, which trims it, drops
//! control characters (they can't go into the saved XML) and caps its length.

use std::ffi::OsString;

/// The variables holding the logon name; the first with a usable one wins.
#[cfg(target_os = "windows")]
const LOGON_VARS: &[&str] = &["USERNAME"];
#[cfg(not(target_os = "windows"))]
const LOGON_VARS: &[&str] = &["USER", "LOGNAME"];

/// How long the Windows account lookup may take before the logon name is used instead.
#[cfg(target_os = "windows")]
const LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// The most of `/etc/passwd` read: far more than any real one holds.
#[cfg(all(unix, not(target_os = "macos")))]
const MAX_PASSWD_BYTES: u64 = 4 << 20;

/// The name of the account running WordCraft: its full name where the system has one, else its
/// logon name; `None` without either.
pub fn system() -> Option<String> {
    let login = logon_name(|var| std::env::var_os(var));
    #[cfg(target_os = "windows")]
    {
        if let Some(name) = windows_account_name() {
            return Some(name);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(name) = passwd_full_name(login.as_deref()) {
            return Some(name);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(name) = directory_services_full_name() {
            return Some(name);
        }
    }
    login
}

/// The logon name: the first of [`LOGON_VARS`] that holds a Unicode, non-blank one.
fn logon_name(var: impl Fn(&str) -> Option<OsString>) -> Option<String> {
    LOGON_VARS.iter().find_map(|name| Some(var(name)?.into_string().ok()?.trim().to_owned()).filter(|v| !v.is_empty()))
}

/// The account's name ([`account_name`]), asked on a thread of its own so a lookup that never
/// answers can't hold up the start. `None` when there is none, or on any failure (logged).
#[cfg(target_os = "windows")]
fn windows_account_name() -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    // A lookup still running after the timeout ends on its own; nobody waits for its answer.
    if let Err(e) = std::thread::Builder::new().name("user-name".into()).spawn(move || {
        let _ = tx.send(account_name().map_err(|e| e.to_string()));
    }) {
        log::debug!("user name: no lookup thread: {e}");
        return None;
    }
    match rx.recv_timeout(LOOKUP_TIMEOUT) {
        Ok(Ok(name)) => Some(name).filter(|n| !n.trim().is_empty()),
        Ok(Err(e)) => {
            log::debug!("user name: account lookup failed: {e}");
            None
        }
        Err(_) => {
            log::debug!("user name: account lookup gave no answer within {LOOKUP_TIMEOUT:?}");
            None
        }
    }
}

/// The account's name ([`chosen`] from its first, last and display names). Empty when the account
/// has none of them.
#[cfg(target_os = "windows")]
fn account_name() -> windows::core::Result<String> {
    use windows::System::{KnownUserProperties, User};
    let users = User::FindAllAsync()?.join()?;
    // A desktop app sees the one account it runs under; any other answer can't say which is ours.
    if users.Size()? != 1 {
        return Ok(String::new());
    }
    let user = users.GetAt(0)?;
    let first = user_property(&user, "first name", KnownUserProperties::FirstName);
    let last = user_property(&user, "last name", KnownUserProperties::LastName);
    Ok(chosen(&first, &last, || user_property(&user, "display name", KnownUserProperties::DisplayName)).unwrap_or_default())
}

/// One of `user`'s text properties (`what` names it in the log); empty when the account doesn't
/// have it, or when the lookup fails (logged), so the next source is still tried.
#[cfg(target_os = "windows")]
fn user_property(user: &windows::System::User, what: &str, name: fn() -> windows::core::Result<windows::core::HSTRING>) -> String {
    use windows::Foundation::IReference;
    use windows::core::{HSTRING, Interface};
    let read = || -> windows::core::Result<String> {
        let value = user.GetPropertyAsync(&name()?)?.join()?;
        Ok(value.cast::<IReference<HSTRING>>()?.Value()?.to_string())
    };
    read().unwrap_or_else(|e| {
        log::debug!("user name: account {what}: {e}");
        String::new()
    })
}

/// Which of the account's names is the user name: its first and last name ([`joined`]), else its
/// display name (asked only then) — Windows has a display name only for a domain or work account,
/// a first and last name for any account. `None` when all are blank.
#[cfg(any(test, target_os = "windows"))]
fn chosen(first: &str, last: &str, display: impl FnOnce() -> String) -> Option<String> {
    joined(first, last).or_else(|| Some(display().trim().to_owned()).filter(|n| !n.is_empty()))
}

/// A first and a last name as one name (`Ada` + `Lovelace` → `Ada Lovelace`), either alone when
/// the other is blank; `None` when both are.
#[cfg(any(test, target_os = "windows"))]
fn joined(first: &str, last: &str) -> Option<String> {
    let name = [first.trim(), last.trim()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ");
    Some(name).filter(|n| !n.is_empty())
}

/// The running account's full name in `/etc/passwd` ([`full_name`]); `None` when the file can't be
/// read, or has no usable entry for it. `login`, when the environment gives one, is the fallback
/// for a uid the file doesn't list.
#[cfg(all(unix, not(target_os = "macos")))]
fn passwd_full_name(login: Option<&str>) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    // Our own uid where the system tells us one: the environment's logon name can say anything,
    // the uid of the process can't. Without it (no procfs), the entry named by the logon name.
    let uid = std::fs::metadata("/proc/self").ok().map(|m| m.uid().to_string());
    let mut passwd = Vec::new();
    match std::fs::File::open("/etc/passwd").and_then(|f| f.take(MAX_PASSWD_BYTES).read_to_end(&mut passwd)) {
        Ok(_) => full_name(&passwd, uid.as_deref(), login),
        Err(e) => {
            log::debug!("user name: /etc/passwd: {e}");
            None
        }
    }
}

/// The full name in the running account's entry of a `passwd` file (`name:password:UID:GID:GECOS:
/// home:shell` lines): the entry with `uid`, else the first named `login`, as the system's own
/// lookup reads them. The name is the GECOS field up to its first comma (the rest is office and
/// phone numbers), with `&` standing for the entry's name capitalised, a BSD convention. `None`
/// without an entry, with an empty full name or with one that isn't UTF-8.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn full_name(passwd: &[u8], uid: Option<&str>, login: Option<&str>) -> Option<String> {
    let entries: Vec<Vec<&[u8]>> = passwd.split(|&b| b == b'\n').map(|line| line.split(|&b| b == b':').collect()).collect();
    // Our own uid when the file lists it; the first entry named by the logon name otherwise (no
    // procfs, or an account whose uid isn't listed). A uid is matched whole, not as a prefix, so
    // `100` never takes `1000`'s entry.
    let by_uid = |uid: &str| entries.iter().find(|fields| fields.get(2) == Some(&uid.as_bytes()));
    let by_name = |login: &str| entries.iter().find(|fields| fields.first() == Some(&login.as_bytes()));
    let entry = uid.and_then(by_uid).or_else(|| login.and_then(by_name))?;
    let account = std::str::from_utf8(entry.first()?).ok()?;
    let gecos = std::str::from_utf8(entry.get(4)?).ok()?;
    let name = gecos.split(',').next().unwrap_or_default();
    let name = if name.contains('&') { name.replace('&', &capitalised(account)) } else { name.to_owned() };
    Some(name.trim().to_owned()).filter(|n| !n.is_empty())
}

/// The macOS account's full name, from the system's own Directory Services lookup (`id -F`, which
/// prints the login name when the account has no real name); `None` when it fails.
#[cfg(target_os = "macos")]
fn directory_services_full_name() -> Option<String> {
    match std::process::Command::new("id").arg("-F").output() {
        Ok(out) if out.status.success() => std::str::from_utf8(&out.stdout).ok().map(|name| name.trim().to_owned()).filter(|n| !n.is_empty()),
        Ok(out) => {
            log::debug!("user name: id -F: {}", String::from_utf8_lossy(&out.stderr).trim());
            None
        }
        Err(e) => {
            log::debug!("user name: id -F: {e}");
            None
        }
    }
}

/// `word` with its first letter upper case (`ada` → `Ada`).
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &[u8] = b"# local accounts\n\
        root:x:0:0:root:/root:/bin/bash\n\
        ada:x:1000:1000:Ada Lovelace,Room 1,+44 20,,:/home/ada:/bin/zsh\n\
        anna:x:1001:1001:&  Smith:/home/anna:/bin/sh\r\n\
        bob:x:1002:1002::/home/bob:/bin/sh\n\
        carl:x:1003:1003:   ,office:/home/carl:/bin/sh\n\
        dee:x:1004:1004:Dee \xe9:/home/dee:/bin/sh\n\
        eve:x:1005\n\
        zoe:x:1006:1006:Zo\xc3\xab Kowalska:/home/zoe:/bin/sh\n\
        zoe:x:1007:1007:Someone Else:/home/zoe2:/bin/sh\n\
        +ldapuser::::::";

    #[test]
    fn the_passwd_full_name_is_the_gecos_name_without_office_and_phones() {
        assert_eq!(full_name(PASSWD, None, Some("ada")).as_deref(), Some("Ada Lovelace"));
        assert_eq!(full_name(PASSWD, None, Some("root")).as_deref(), Some("root"));
        // Our own uid wins over the name: an account renamed in the environment still gets its own
        // entry, and a uid is matched whole, not as a prefix.
        assert_eq!(full_name(PASSWD, Some("1000"), Some("someone-else")).as_deref(), Some("Ada Lovelace"));
        assert_eq!(full_name(PASSWD, Some("100"), Some("ada")).as_deref(), Some("Ada Lovelace"), "no uid 100 listed: the named entry");
        // No logon name in the environment (a service run straight from init) still has an entry.
        assert_eq!(full_name(PASSWD, Some("1000"), None).as_deref(), Some("Ada Lovelace"));
        // `&` is the entry's own name, capitalised; a Windows line ending is no part of the name.
        assert_eq!(full_name(PASSWD, None, Some("anna")).as_deref(), Some("Anna  Smith"));
        assert_eq!(full_name(PASSWD, Some("1001"), Some("anna")).as_deref(), Some("Anna  Smith"));
        // The first entry for a uid or a name is the one, as `getpwnam` reads it.
        assert_eq!(full_name(PASSWD, None, Some("zoe")).as_deref(), Some("Zoë Kowalska"));
        assert_eq!(full_name(PASSWD, Some("1007"), Some("zoe")).as_deref(), Some("Someone Else"));
    }

    #[test]
    fn no_usable_passwd_name_falls_back_to_the_logon_name() {
        for (uid, login, why) in [
            (None, Some("bob"), "an empty GECOS field"),
            (Some("1002"), Some("bob"), "an empty GECOS field by uid"),
            (None, Some("carl"), "a full name of blanks"),
            (None, Some("dee"), "a full name that isn't UTF-8 (Latin-1)"),
            (None, Some("eve"), "a cut-off entry"),
            (Some("1005"), Some("eve"), "a cut-off entry by uid"),
            (None, Some("ad"), "a prefix of another account's name"),
            (None, Some("mallory"), "no entry"),
            (Some("4242"), Some("mallory"), "no entry for our uid"),
            (None, Some("ldapuser"), "a NIS entry for someone else"),
            (None, Some(""), "an empty name"),
            (None, Some("# local accounts"), "a comment"),
            (None, None, "no logon name and no uid"),
            (Some("4242"), None, "no entry for our uid and no logon name"),
        ] {
            assert_eq!(full_name(PASSWD, uid, login), None, "{why} ({uid:?}, {login:?})");
        }
        assert_eq!(full_name(b"", None, Some("ada")), None, "an empty file");
        assert_eq!(full_name(b"ada:x:1000:1000:Ada Lovelace", None, Some("ada")).as_deref(), Some("Ada Lovelace"), "no shell or home");
    }

    #[test]
    fn the_logon_name_skips_blank_and_non_unicode_variables() {
        let env =
            |vars: &'static [(&'static str, &'static str)]| move |name: &str| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(*v));
        let first = LOGON_VARS.first().copied().unwrap_or_default();
        assert_eq!(logon_name(env(&[])), None);
        assert_eq!(logon_name(|name| (name == first).then(|| OsString::from(" ada \t"))).as_deref(), Some("ada"));
        assert_eq!(logon_name(|name| (name == first).then(|| OsString::from(" \t "))), None);
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(logon_name(env(&[("USER", "ada"), ("LOGNAME", "root")])).as_deref(), Some("ada"), "USER first");
            assert_eq!(logon_name(env(&[("USER", "  "), ("LOGNAME", "ada")])).as_deref(), Some("ada"));
            use std::os::unix::ffi::OsStringExt;
            let bad = |name: &str| match name {
                "USER" => Some(OsString::from_vec(b"caf\xe9".to_vec())),
                "LOGNAME" => Some(OsString::from("ada")),
                _ => None,
            };
            assert_eq!(logon_name(bad).as_deref(), Some("ada"), "a name that isn't UTF-8 is skipped");
        }
    }

    #[test]
    fn a_first_and_last_name_join_into_one() {
        assert_eq!(joined("Ada", "Lovelace").as_deref(), Some("Ada Lovelace"));
        assert_eq!(joined(" Zoë ", "\tKowalska ").as_deref(), Some("Zoë Kowalska"));
        // Either alone is the name; with neither the display name is tried.
        assert_eq!(joined("Ada", "").as_deref(), Some("Ada"));
        assert_eq!(joined(" ", "Lovelace").as_deref(), Some("Lovelace"));
        assert_eq!(joined("", ""), None);
        assert_eq!(joined(" \t", " "), None);
    }

    #[test]
    fn the_first_and_last_name_win_over_the_display_name() {
        let asked = std::cell::Cell::new(false);
        let display = |name: &'static str| {
            let asked = &asked;
            move || {
                asked.set(true);
                name.to_owned()
            }
        };
        assert_eq!(chosen("Ada", "Lovelace", display("Contoso Admin")).as_deref(), Some("Ada Lovelace"));
        assert!(!asked.get(), "the display name is asked only without a first or last name");
        assert_eq!(chosen("", "Lovelace", display("Contoso Admin")).as_deref(), Some("Lovelace"));
        // Without either (a domain account may have only a display name), the display name.
        assert_eq!(chosen(" ", "", display(" Ada Lovelace ")).as_deref(), Some("Ada Lovelace"));
        assert!(asked.get());
        assert_eq!(chosen("", "", display(" \t")), None, "no name at all: the logon name follows");
    }

    #[test]
    fn capitalised_upper_cases_the_first_letter_only() {
        assert_eq!(capitalised("ada"), "Ada");
        assert_eq!(capitalised("éva"), "Éva");
        assert_eq!(capitalised("ß"), "SS");
        assert_eq!(capitalised(""), "");
    }
}
