// A release build is a Windows GUI program, so no console window opens when it starts at sign-in or from the
// Start menu. Debug builds stay console programs: `cargo run`, `pnpm gen:api` and the tests print to the terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tokio::main]
async fn main() {
    if let Err(error) = td_companion::run().await {
        eprintln!("Companion: {error}");
        std::process::exit(1);
    }
}
