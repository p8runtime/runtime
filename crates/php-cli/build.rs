//! A WASI build has no threads, so php2 cannot start a thread with a deep
//! stack for deep PHP recursion as it does natively: its main stack is
//! made that deep (64 MiB) when it is linked.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("wasi") {
        println!("cargo:rustc-link-arg-bin=php2=-zstack-size=67108864");
    }
}
