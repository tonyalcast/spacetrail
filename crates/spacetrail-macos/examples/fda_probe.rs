//! Spike S6: does a signed .app keep its permissions across rebuilds, and
//! which ones does macOS prompt for by itself?
//! Writes the probe result to /tmp/spacetrail-fda-probe.txt and exits.

const BUILD: &str = match option_env!("SPIKE_BUILD") {
    Some(b) => b,
    None => "dev",
};

fn main() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let mut out = format!("build={BUILD} pid={}\n", std::process::id());
    // Folders protected by "Files and Folders" consent: macOS prompts on first access.
    for dir in ["Documents", "Desktop", "Downloads"] {
        let r = std::fs::read_dir(home.join(dir)).map(|rd| rd.count());
        out.push_str(&format!("{dir}: {r:?}\n"));
    }
    // Full Disk Access: never prompted, only granted in System Settings.
    out.push_str(&format!("fda={:?}\n", spacetrail_macos::fda::probe(&home)));
    std::fs::write("/tmp/spacetrail-fda-probe.txt", out).ok();
}
