//! Private credentials. Management initialization is explicit; optional LAN
//! credentials are initialized only by an explicitly enabled service startup.
use axum::http::HeaderValue;
use std::{
    fmt,
    io::{self, Read},
    path::Path,
};
use subtle::ConstantTimeEq;
#[cfg(unix)]
#[path = "token/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "token/windows.rs"]
mod platform;

pub struct SecretToken([u8; 64]);
impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken([REDACTED])")
    }
}
impl SecretToken {
    pub fn generate() -> io::Result<Self> {
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| io::Error::other("secure random source unavailable"))?;
        let hex = crate::proof::encode_hex(&random);
        let mut token = [0; 64];
        token.copy_from_slice(hex.as_bytes());
        Ok(Self(token))
    }
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() != 64
            || !bytes
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
        {
            return Err(invalid());
        }
        let mut value = [0; 64];
        value.copy_from_slice(bytes);
        Ok(Self(value))
    }
    pub(crate) fn key_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn bearer_header_value(&self) -> HeaderValue {
        let mut buffer = [0u8; 71];
        buffer[..7].copy_from_slice(b"Bearer ");
        buffer[7..].copy_from_slice(&self.0);
        let mut header = HeaderValue::from_bytes(&buffer).expect("fixed ASCII token");
        header.set_sensitive(true);
        header
    }
    pub fn matches_authorization(&self, value: &[u8]) -> bool {
        // Length/prefix are public protocol properties; the fixed secret comparison
        // always runs and never performs data-dependent early exit.
        let mut candidate = [0u8; 64];
        let valid = value.len() == 71 && value.get(..7) == Some(b"Bearer ");
        if valid {
            candidate.copy_from_slice(&value[7..]);
        }
        bool::from(candidate.ct_eq(&self.0)) & valid
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "credential path, permissions, or format is invalid",
    )
}
/// Every newly created directory is private from its first observable instant.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    platform::create_private_dir(path)
}
/// Atomic no-clobber publication after a private temporary file is fully synced.
pub fn write_private_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    platform::write_private_new(path, bytes)
}
pub fn init_private_token(root: &Path) -> io::Result<SecretToken> {
    init_named_token(root, "api-token")
}
/// Only call on explicitly enabled LAN service startup, never on read/browse/init.
pub fn init_private_lan_token(root: &Path) -> io::Result<SecretToken> {
    init_named_token(root, "lan-api-token")
}
pub fn load_private_lan_token(root: &Path) -> io::Result<SecretToken> {
    load_named_token(root, "lan-api-token")
}
fn init_named_token(root: &Path, name: &str) -> io::Result<SecretToken> {
    create_private_dir(root)?;
    create_private_dir(&root.join("secrets"))?;
    let path = root.join("secrets").join(name);
    match load_named_token(root, name) {
        Ok(value) => return Ok(value),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let token = SecretToken::generate()?;
    match write_private_new(&path, token.key_bytes()) {
        Ok(()) => Ok(token),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => load_named_token(root, name),
        Err(error) => Err(error),
    }
}
pub fn load_private_token(root: &Path) -> io::Result<SecretToken> {
    load_named_token(root, "api-token")
}
fn load_named_token(root: &Path, name: &str) -> io::Result<SecretToken> {
    platform::validate_private_dir(root)?;
    let mut file = platform::open_private_file(&root.join("secrets").join(name))?;
    let mut bytes = Vec::with_capacity(65);
    file.by_ref().take(65).read_to_end(&mut bytes)?;
    SecretToken::from_bytes(&bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_init_preserves_secret_and_redacts_it() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let first = init_private_token(&root).unwrap();
        let second = init_private_token(&root).unwrap();
        assert!(
            bool::from(first.key_bytes().ct_eq(second.key_bytes())),
            "repeated init must preserve the credential"
        );
        assert!(!format!("{first:?}").contains(std::str::from_utf8(first.key_bytes()).unwrap()));
        let h = first.bearer_header_value();
        assert!(h.is_sensitive());
        assert!(first.matches_authorization(h.as_bytes()));
        assert!(!first.matches_authorization(b"Bearer wrong"));
    }
    #[cfg(unix)]
    #[test]
    fn modes_and_symlinks_fail_closed() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        init_private_token(&root).unwrap();
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let path = root.join("secrets/api-token");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(init_private_token(&root).is_err());
        std::fs::remove_file(&path).unwrap();
        symlink(temp.path().join("victim"), &path).unwrap();
        assert!(init_private_token(&root).is_err());
        assert!(!temp.path().join("victim").exists());
    }
    #[cfg(unix)]
    #[test]
    fn hardlinks_and_nonprivate_existing_directories_are_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        init_private_token(&root).unwrap();
        let token = root.join("secrets/api-token");
        let alias = root.join("alias");
        std::fs::hard_link(&token, &alias).unwrap();
        assert!(load_private_token(&root).is_err());
        std::fs::remove_file(alias).unwrap();
        std::fs::set_permissions(root.join("secrets"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert!(load_private_token(&root).is_err());
        assert!(init_private_token(&root).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn protected_dacl_hardlink_and_reparse_boundaries_are_real() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let secret = init_private_token(&root).unwrap();
        assert!(platform::validate_private_dir(&root).is_ok());
        assert!(platform::open_private_file(&root.join("secrets/api-token")).is_ok());
        let alias = root.join("alias");
        std::fs::hard_link(root.join("secrets/api-token"), &alias).unwrap();
        assert!(load_private_token(&root).is_err());
        std::fs::remove_file(&alias).unwrap();
        let junction = temp.path().join("junction");
        let result = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "temporary junction creation failed"
        );
        assert!(load_private_token(&junction).is_err());
        assert!(create_private_dir(&junction).is_err());
        std::fs::remove_dir(junction).unwrap();
        // Default-created files have inherited (unprotected) DACLs. Moving one
        // into the private directory must not silently bless its old permissions.
        let inherited = temp.path().join("inherited");
        std::fs::write(&inherited, secret.key_bytes()).unwrap();
        std::fs::remove_file(root.join("secrets/api-token")).unwrap();
        std::fs::rename(inherited, root.join("secrets/api-token")).unwrap();
        assert!(load_private_token(&root).is_err());
    }
}

#[cfg(test)]
mod lan_tests {
    use super::*;
    #[test]
    fn lan_credentials_are_independent_explicit_private_and_no_clobber() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let local = init_private_token(&root).unwrap();
        assert!(!root.join("secrets/lan-api-token").exists());
        assert_eq!(
            load_private_lan_token(&root).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let lan = init_private_lan_token(&root).unwrap();
        assert_ne!(lan.bearer_header_value(), local.bearer_header_value());
        assert_eq!(
            init_private_lan_token(&root).unwrap().bearer_header_value(),
            lan.bearer_header_value()
        );
        assert_eq!(
            load_private_token(&root).unwrap().bearer_header_value(),
            local.bearer_header_value()
        );
        assert!(!lan.matches_authorization(local.bearer_header_value().as_bytes()));
        assert!(!local.matches_authorization(lan.bearer_header_value().as_bytes()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = root.join("secrets/lan-api-token");
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(init_private_lan_token(&root).is_err());
            assert!(load_private_lan_token(&root).is_err());
            assert!(load_private_token(&root).is_ok());
        }
    }
}

/// Bounded callers may read legacy non-secret configuration through this safe
/// regular-file open. New configuration is always published with private ACLs.
pub fn open_regular_file(path: &Path) -> io::Result<std::fs::File> {
    platform::open_regular_file(path)
}
pub fn open_private_file(path: &Path) -> io::Result<std::fs::File> {
    platform::open_private_file(path)
}
pub fn atomic_replace_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    platform::atomic_replace(path, bytes)
}
