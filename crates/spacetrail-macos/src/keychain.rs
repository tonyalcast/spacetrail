//! Secrets in the user's login keychain (generic passwords).
//!
//! Items are created by the signed app, so only that app (same signature)
//! reads them without a prompt. Secrets never touch a config file.

use std::io;

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

/// Keychain service under which spacetrail stores its secrets.
pub const SERVICE: &str = "io.github.tonyalcast.spacetrail";

/// errSecItemNotFound.
const NOT_FOUND: i32 = -25300;

/// Stores (or replaces) a secret for `account`.
///
/// # Errors
/// Keychain failure.
pub fn set(account: &str, secret: &str) -> io::Result<()> {
    set_generic_password(SERVICE, account, secret.as_bytes()).map_err(io::Error::other)
}

/// Reads the secret for `account`, `None` if absent.
///
/// # Errors
/// Keychain failure other than "not found".
pub fn get(account: &str) -> io::Result<Option<String>> {
    match get_generic_password(SERVICE, account) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(io::Error::other(e)),
    }
}

/// Removes the secret for `account` (no error if absent).
///
/// # Errors
/// Keychain failure other than "not found".
pub fn delete(account: &str) -> io::Result<()> {
    match delete_generic_password(SERVICE, account) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => Ok(()),
        Err(e) => Err(io::Error::other(e)),
    }
}
