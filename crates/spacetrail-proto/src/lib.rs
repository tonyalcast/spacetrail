//! Wire types of the spacetrail daemon API (JSON-RPC 2.0 over a Unix domain
//! socket, newline-delimited). Shared by the daemon and every client.

#![forbid(unsafe_code)]

/// Protocol version negotiated in the `hello` handshake. Bump on breaking changes.
pub const PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_version() {
        assert_eq!(super::PROTOCOL_VERSION, 1);
    }
}
