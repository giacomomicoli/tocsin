//! Headless check: `cargo run -p tocsin-core --example check -- <AddOns dir>`
//!
//! Prints what Tocsin found for each addon and whether an update exists.

use std::path::PathBuf;
use std::sync::Arc;

use tocsin_core::http::Http;
use tocsin_core::{check_all, Library, Manifest, UpdateStatus};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: check <path to Interface/AddOns>"))?;

    let manifest = Manifest::default_path()
        .map(|p| Manifest::load(&p))
        .transpose()?
        .unwrap_or_default();

    let lib = Library::load(&dir, &manifest)?;
    println!("{} addons in {}\n", lib.addons.len(), dir.display());

    for a in &lib.addons {
        for i in &a.issues {
            println!("{:<32} !! {}", "", i.describe());
        }
        println!(
            "{:<32} {:<14} {}",
            a.display_name,
            a.installed.as_ref().map_or("-", |v| v.as_str()),
            if a.sources.is_empty() {
                "no source".to_string()
            } else {
                a.sources
                    .iter()
                    .map(|s| format!("{} → {}", s.provider(), s.web_url()))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        );
    }

    println!();
    for fam in tocsin_core::family::group(&lib.addons) {
        let members: Vec<&str> = fam
            .members
            .iter()
            .map(|&i| lib.addons[i].folder.as_str())
            .collect();
        if !members.is_empty() {
            println!(
                "family {:<24} + {}",
                lib.addons[fam.head].folder,
                members.join(", ")
            );
        }
    }
    for (i, deps) in tocsin_core::family::missing_dependencies(&lib.addons) {
        println!("missing for {:<20} {}", lib.addons[i].folder, deps.join(", "));
    }

    if std::env::var_os("TOCSIN_OFFLINE").is_some() {
        return Ok(());
    }

    println!("\nChecking…\n");
    let http = Http::new();
    println!("client: {}", lib.client.describe());
    // Blocking keychain read: fine here, there is no UI thread to stall.
    let creds = tocsin_core::checker::credentials(&manifest);
    let addons = Arc::new(lib.addons);
    for r in check_all(&http, &manifest, &creds, &addons, lib.client.flavor).await {
        let extra = match &r.status {
            UpdateStatus::Outdated(rel)
            | UpdateStatus::UpToDate(rel)
            | UpdateStatus::Ahead(rel)
            | UpdateStatus::Unknown(rel) => {
                format!("latest {} ({})", rel.version, rel.url)
            }
            UpdateStatus::Manual(url) => url.clone(),
            UpdateStatus::Error(e) => e.clone(),
            _ => String::new(),
        };
        println!("{:<32} {:<18} {}", r.folder, r.status.label(), extra);
    }
    Ok(())
}
