#![forbid(unsafe_code)]
//! V4b, "kind-aware AOT": see the crate's documentation.

fn main() -> std::process::ExitCode {
    aot::main_with(aot::Flavour::B)
}
