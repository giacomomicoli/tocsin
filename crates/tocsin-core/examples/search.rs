//! Headless search:
//! `cargo run -p tocsin-core --example search -- <query> [retail|classic|vanilla|forever] [build]`
//!
//! Prints the merged results and, per addon, every provider that offers it
//! with how the site says it fits the client, the icon URL it gave (`-` for
//! none) and the one the app would show. The build (`12.1.5`) is the game's
//! version; without it nothing can be confirmed for a build. Uses the same
//! keys as the app (environment, then the OS credential store).

use tocsin_core::addon::Flavor;
use tocsin_core::http::Http;
use tocsin_core::{search, Manifest};

const USAGE: &str = "usage: search <query> [retail|classic|vanilla|forever] [build, e.g. 12.1.5]";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let query = args.next().ok_or_else(|| anyhow::anyhow!(USAGE))?;
    let flavor = match args.next().as_deref() {
        Some("classic") => Flavor::Classic,
        Some("vanilla") => Flavor::Vanilla,
        Some("forever") => Flavor::Forever,
        _ => Flavor::Retail,
    };
    let client_interface = match args.next() {
        Some(build) => {
            let (ma, mi, pa) = tocsin_core::client::parse_version(&build)
                .ok_or_else(|| anyhow::anyhow!("build must look like 12.1.5\n{USAGE}"))?;
            Some(ma * 10000 + mi * 100 + pa)
        }
        None => None,
    };
    let manifest = Manifest::default_path()
        .map(|p| Manifest::load(&p))
        .transpose()?
        .unwrap_or_default();
    let creds = tocsin_core::checker::credentials(&manifest);

    let catalog = tocsin_core::sources::wowinterface::Catalog::default();
    let started = std::time::Instant::now();
    let out = search::search(&Http::new(), &creds, &catalog, &query, flavor, client_interface).await;
    eprintln!("searched in {:?}", started.elapsed());
    for f in &out.results {
        let confirmed = if f.confirmed() { "" } else { "  [unconfirmed]" };
        println!("{}  — {}{confirmed}", f.primary().name, f.authors().join(", "));
        println!("    icon shown: {}", f.icon().unwrap_or("- (monogram)"));
        for h in &f.hits {
            println!(
                "    {:<12} {:<12} {:<18} {:<6} {}",
                h.source.provider(),
                h.compat.label(flavor),
                h.popularity.map(|p| p.describe()).unwrap_or_default(),
                h.updated.map(|y| y.to_string()).unwrap_or_default(),
                h.url
            );
            println!("    {:<12} icon: {}", "", h.icon.as_deref().unwrap_or("-"));
        }
    }
    if !out.skipped.is_empty() {
        println!("\nskipped (no key): {}", out.skipped.join(", "));
    }
    for e in &out.errors {
        println!("error: {e}");
    }
    Ok(())
}
