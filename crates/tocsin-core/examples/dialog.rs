//! Says whether this desktop can open a folder dialog, the way *Add game
//! folder* decides after the dialog came back empty.
//!
//! ```sh
//! cargo run -p tocsin-core --example dialog
//! ```

#[tokio::main]
async fn main() {
    match tocsin_core::desktop::folder_dialog_problem().await {
        None => println!("A folder dialog can open on this desktop."),
        Some(problem) => println!("{problem}"),
    }
}
