//! Headless install/rollback: exercises the swap logic on a zip you already have.
//!
//! ```sh
//! cargo run -p tocsin-core --example install -- <AddOns dir> <zip> [key]
//! cargo run -p tocsin-core --example install -- <AddOns dir> --rollback <key>
//! ```

use std::path::PathBuf;

use tocsin_core::addon::Flavor;
use tocsin_core::client::GameClient;
use tocsin_core::install::{self, Request};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, rest @ ..] = args.as_slice() else {
        anyhow::bail!("usage: install <AddOns dir> <zip> [key] | <AddOns dir> --rollback <key>");
    };
    let addons = PathBuf::from(dir);
    let client = GameClient::detect(&addons);
    println!("client: {}", client.describe());

    match rest {
        [flag, key] if flag == "--rollback" => {
            let o = install::rollback(&addons, key)?;
            println!(
                "rolled back {key}: now {:?}, parked {:?}",
                o.installed, o.replaced
            );
        }
        [zip, key @ ..] => {
            let zip = PathBuf::from(zip);
            let manifest = tocsin_core::archive::inspect(&zip)?;
            let key = key
                .first()
                .cloned()
                .or_else(|| manifest.folders.keys().next().cloned())
                .unwrap_or_default();
            let version = manifest
                .folders
                .values()
                .find_map(|t| t.version().map(str::to_string))
                .unwrap_or_else(|| "unknown".into());
            println!(
                "zip: {} folders, {} entries, {} bytes unpacked",
                manifest.folders.len(),
                manifest.entries,
                manifest.unpacked
            );
            let req = Request {
                key: &key,
                provider: "local",
                source: None,
                version: &version,
                sha256: "",
                flavor: client.flavor,
                client_interface: client.interface(),
                replacing_version: None,
            };
            let o = install::install(&addons, &zip, &req)?;
            println!("installed {:?}, replaced {:?}", o.installed, o.replaced);
        }
        _ => anyhow::bail!("usage: install <AddOns dir> <zip> [key] | <AddOns dir> --rollback <key>"),
    }
    let _ = Flavor::Retail;
    Ok(())
}
