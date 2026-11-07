# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Cargo workspace scaffold (core, macos, store, cleaners, proto, daemon, cli, xtask).
- Safety rail: clippy `disallowed-methods` for deletion/process APIs + `cargo xtask check-safety`.
- CI: fmt, clippy, tests, x86_64 cross-check, MSRV, cargo-deny.
