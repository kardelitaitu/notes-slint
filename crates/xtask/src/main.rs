//! xtask: repository automation, run through the "cargo xtask" alias.

mod arch;

fn usage() {
    eprintln!("usage: cargo xtask check-arch");
    eprintln!(
        "       enforce the workspace layering rules via cargo metadata (exit 1 on a violation)"
    );
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("check-arch") => std::process::exit(arch::run()),
        Some(command) => {
            eprintln!("xtask: unknown command '{command}'");
            usage();
            std::process::exit(2);
        }
        None => {
            usage();
            std::process::exit(2);
        }
    }
}
