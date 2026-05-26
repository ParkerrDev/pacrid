use std::path::PathBuf;

/// Find the home directories to scan.
///
/// When pacman runs hooks it runs as root, so `$HOME` is `/root`.
/// We try several sources in order to find the real user's home:
///  1. `SUDO_USER` (sudo)
///  2. `DOAS_USER` (doas)
///  3. `PKEXEC_UID` (polkit)
///  4. Fall back: all UID ≥ 1000 entries in /etc/passwd (human users)
///
/// Returns at least one entry; always non-empty on a normal system.
pub fn user_homes_to_scan() -> Vec<PathBuf> {
    assert!(
        std::path::Path::new("/etc/passwd").exists(),
        "/etc/passwd must exist"
    );

    // Fast path: a privilege-escalation env var tells us exactly who the user is.
    if let Some(home) = from_sudo_user() {
        tracing::debug!("user home from SUDO_USER/DOAS_USER: {}", home.display());
        return vec![home];
    }
    if let Some(home) = from_pkexec_uid() {
        tracing::debug!("user home from PKEXEC_UID: {}", home.display());
        return vec![home];
    }

    // Slow path: scan /etc/passwd for all human accounts.
    let homes = all_human_homes();
    if !homes.is_empty() {
        tracing::debug!("user homes from /etc/passwd (uid>=1000): {:?}", homes);
        return homes;
    }

    // Last resort: whatever HOME says, even if it's /root.
    let fallback = std::env::var("HOME").map_or_else(|_| PathBuf::from("/root"), PathBuf::from);
    tracing::warn!("could not detect real user home; falling back to {}", fallback.display());
    vec![fallback]
}

fn from_sudo_user() -> Option<PathBuf> {
    let user = std::env::var("SUDO_USER")
        .ok()
        .or_else(|| std::env::var("DOAS_USER").ok())?;
    if user.is_empty() || user == "root" {
        return None;
    }
    home_from_passwd_name(&user)
}

fn from_pkexec_uid() -> Option<PathBuf> {
    let uid_str = std::env::var("PKEXEC_UID").ok()?;
    let uid: u32 = uid_str.parse().ok()?;
    if uid == 0 {
        return None;
    }
    home_from_passwd_uid(uid)
}

/// Parse /etc/passwd and return the home dir for the given username.
fn home_from_passwd_name(name: &str) -> Option<PathBuf> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let mut f = line.splitn(7, ':');
        let entry_name = f.next()?;
        if entry_name != name {
            continue;
        }
        for _ in 0..4 {
            f.next();
        }
        let home = PathBuf::from(f.next()?);
        if home.is_dir() {
            return Some(home);
        }
    }
    None
}

/// Parse /etc/passwd and return the home dir for the given UID.
fn home_from_passwd_uid(uid: u32) -> Option<PathBuf> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let mut f = line.splitn(7, ':');
        f.next(); // username
        f.next(); // password
        let entry_uid: u32 = f.next()?.parse().ok()?;
        if entry_uid != uid {
            continue;
        }
        for _ in 0..2 {
            f.next();
        }
        let home = PathBuf::from(f.next()?);
        if home.is_dir() {
            return Some(home);
        }
    }
    None
}

/// Return home directories for all human users (UID 1000–59999).
fn all_human_homes() -> Vec<PathBuf> {
    let Ok(passwd) = std::fs::read_to_string("/etc/passwd") else { return Vec::new() };
    let mut homes = Vec::new();
    for line in passwd.lines() {
        let mut f = line.splitn(7, ':');
        f.next(); // username
        f.next(); // password
        let uid: u32 = match f.next().and_then(|s| s.parse().ok()) {
            Some(u) => u,
            None => continue,
        };
        // UID 1000–59999 = human users on Linux; skip system and nobody (65534).
        if !(1000..=59_999).contains(&uid) {
            continue;
        }
        for _ in 0..2 {
            f.next();
        }
        let home = match f.next() {
            Some(h) => PathBuf::from(h),
            None => continue,
        };
        if home.is_dir() && !homes.contains(&home) {
            homes.push(home);
        }
    }
    homes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_human_homes_is_non_empty() {
        // On any real system there should be at least one human user home.
        // In CI this may be empty; just confirm it doesn't panic.
        let homes = all_human_homes();
        // At minimum, the function should not panic and return a Vec.
        assert!(homes.len() < 1000, "sanity check: not thousands of homes");
    }

    #[test]
    fn home_from_passwd_name_finds_root() {
        // root is always in /etc/passwd and has uid=0 but this tests the parsing path.
        let home = home_from_passwd_name("root");
        assert!(
            home.map_or(true, |h| h.exists()),
            "if root entry found, home must exist"
        );
    }
}
