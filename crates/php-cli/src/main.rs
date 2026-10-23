#![forbid(unsafe_code)] // memory safety is the point of this runtime; see SECURITY.md
//! The php command-line host for p8 (docs/design/engine-v2.md), named
//! php2 while p8 is built next to the legacy engine: `php2 script.php`.

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dump = args.first().is_some_and(|a| a == "--dump");
    if dump {
        args.remove(0);
    }
    let Some(path) = args.into_iter().next() else {
        eprintln!("usage: php2 [--dump] <file>");
        std::process::exit(64);
    };
    let source = match std::fs::read(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not open input file: {path} ({e})");
            std::process::exit(1);
        }
    };
    let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
    let file = std::fs::canonicalize(&path).map_or(path.clone(), |p| p.to_string_lossy().into_owned());
    if dump {
        match platform.dump(&source, &file) {
            Ok(text) => print!("{text}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(255);
            }
        }
        return;
    }
    let status = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || platform.run(&source, &file))
        .expect("interpreter thread")
        .join()
        .unwrap_or(255);
    std::process::exit(status);
}
