//! spacetrail background agent: volume sampling, scheduled and incremental
//! snapshots, live monitoring, alerts and autoclean, served over a local
//! JSON-RPC socket. Runs as `spacetrail daemon run`.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_matches_proto_crate() {
        assert_eq!(spacetrail_proto::PROTOCOL_VERSION, 1);
    }
}
