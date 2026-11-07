//! Headless addon page:
//! `cargo run -p tocsin-core --example page -- <curseforge:ID|github:OWNER/REPO|wowinterface:ID|tukui:SLUG>`
//!
//! Prints what the addon view would show: name, authors, summary, the
//! gallery, and the description one block a line, with each picture marked
//! as fetchable or shown as a link. Uses the same keys as the app
//! (environment, then the OS credential store). Fetches no picture.

use tocsin_core::http::Http;
use tocsin_core::page::{self, media, Block, Kind, Marker};
use tocsin_core::{Manifest, SourceRef};

const USAGE: &str = "usage: page <curseforge:ID | github:OWNER/REPO | wowinterface:ID | tukui:SLUG>";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!(USAGE))?;
    let source = parse(&arg).ok_or_else(|| anyhow::anyhow!(USAGE))?;
    let manifest = Manifest::default_path()
        .map(|p| Manifest::load(&p))
        .transpose()?
        .unwrap_or_default();
    let creds = tocsin_core::checker::credentials(&manifest);
    let p = page::fetch(&Http::new(), &creds, &source).await?;

    println!("{} · by {} · {}", p.name, p.authors.join(", "), p.url);
    if !p.summary.is_empty() {
        println!("{}", p.summary);
    }
    for s in &p.gallery {
        println!("shot {} {}", fetchable(&s.thumb), s.thumb);
    }
    println!("---");
    for b in &p.body.blocks {
        match b {
            Block::Text { kind, spans } => {
                let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                let lead = match kind {
                    Kind::Paragraph => String::new(),
                    Kind::Heading(l) => "#".repeat(usize::from(*l)) + " ",
                    Kind::Quote => "> ".into(),
                    Kind::Item { depth, marker } => {
                        let m = match marker {
                            Some(Marker::Bullet) => "* ".to_string(),
                            Some(Marker::Number(n)) => format!("{n}. "),
                            None => "  ".into(),
                        };
                        "  ".repeat(usize::from(*depth) - 1) + &m
                    }
                };
                println!("{lead}{}", text.replace('\n', " / "));
            }
            Block::Code(c) => println!("```{}```", c.replace('\n', " / ")),
            Block::Image(i) => println!("[img {} {}] {}", fetchable(&i.url), i.url, i.alt),
            Block::Rule => println!("----"),
        }
    }
    if p.body.truncated {
        println!("(truncated)");
    }
    Ok(())
}

fn fetchable(url: &str) -> &'static str {
    if media::is_allowed(url) {
        "fetch"
    } else {
        "link"
    }
}

fn parse(arg: &str) -> Option<SourceRef> {
    let (kind, rest) = arg.split_once(':')?;
    Some(match kind {
        "curseforge" => SourceRef::CurseForge {
            id: rest.parse().ok()?,
        },
        "wowinterface" => SourceRef::WowInterface {
            id: rest.parse().ok()?,
        },
        "tukui" => SourceRef::Tukui {
            slug: rest.to_string(),
        },
        "github" => {
            let (owner, repo) = rest.split_once('/')?;
            SourceRef::Github {
                owner: owner.to_string(),
                repo: repo.to_string(),
                tag_prefix: None,
            }
        }
        _ => return None,
    })
}
