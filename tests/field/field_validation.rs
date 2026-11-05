//! Opt-in real-tool validation. Never included in release assets.
mod fixtures;
mod harness;

fn main() {
    std::process::exit(harness::main());
}
