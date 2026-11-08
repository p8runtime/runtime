#![forbid(unsafe_code)]
//! V4a, "dumb AOT": see the crate's documentation.

fn main() -> std::process::ExitCode {
    aot::main_with(aot::Flavour::A)
}
