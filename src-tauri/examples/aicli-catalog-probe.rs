//! CI-only projection of local rc.34 snapshots; never consumes a credential.
use cc_switch_lib::aicli::catalog::Catalog;
use std::io::{self, Read};

fn run() -> Result<(), &'static str> {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("HOME").ok().filter(|home| !home.is_empty())
            != std::env::var("CC_SWITCH_TEST_HOME").ok()
    {
        return Err("isolated CI only");
    }
    let mut body = String::new();
    io::stdin()
        .take(8 * 1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|_| "snapshot input failed")?;
    let input: serde_json::Value = serde_json::from_str(&body).map_err(|_| "invalid snapshots")?;
    let catalog = Catalog::from_snapshots(&input["groups"], &input["pricing"])
        .map_err(|_| "projection failed")?;
    println!(
        "{}",
        serde_json::to_string(&catalog).map_err(|_| "serialization failed")?
    );
    Ok(())
}

fn main() {
    if let Err(step) = run() {
        eprintln!("C1-07: FAIL; {step}");
        std::process::exit(1);
    }
}
