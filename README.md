# spacetrail

**Monitor, explain and clean disk space on macOS**: storage history over time, growth attribution by directory, real-time detection of who is writing, and community-maintained cleaners classified by risk (`safe` / `review` / `manual`). Local-only, zero telemetry, dry-run and Trash by default.

> 🚧 Early development. See [`docs/PLAN.md`](docs/PLAN.md) for the roadmap.

```
spacetrail status     # capacity and where the used space lives (per APFS volume)
spacetrail top        # largest folders and files (whole disk by default)
spacetrail suggest    # what you can delete, grouped by risk, with how-to
spacetrail doctor     # Full Disk Access and environment checks
```

Every command supports `--json`. Nothing is ever deleted by these commands.

- Architecture: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- Cleaner format: [`docs/CLEANER_SPEC.md`](docs/CLEANER_SPEC.md)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
