//! xtask: repository automation, run through the "cargo xtask" alias.

mod arch;
mod check;
mod deps;
mod fixtures;
mod metadata;
mod smoke;

fn usage() {
    eprintln!("usage: cargo xtask check-arch");
    eprintln!(
        "       enforce the workspace layering rules via cargo metadata (exit 1 on a violation)"
    );
    eprintln!("usage: cargo xtask check-deps");
    eprintln!("       reject dependency declarations that can drift from the workspace templates");
    eprintln!("usage: cargo xtask smoke [--reuse-state]");
    eprintln!(
        "       launch the built binary, close its window with WM_CLOSE, prove it exits by itself"
    );
    eprintln!(
        "       (opens a real window; a pre-existing session.json is moved aside and put back;"
    );
    eprintln!("       exits 3 with no desktop, or when foreign state sits on the session path)");
    eprintln!("usage: cargo xtask check [--quick]");
    eprintln!(
        "       run the AGENTS.md gate with per-step verdicts (--quick skips the slow steps)"
    );
    eprintln!("usage: cargo xtask fixtures generate|verify [--against-generator]");
    eprintln!(
        "       write / check the byte-exact round-trip fixtures (verify exits 1 on any difference;"
    );
    eprintln!(
        "       the generator comparison is always on; the flag merely states it explicitly)"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => {
            let rest = &args[1..];
            let quick = rest.iter().any(|a| a == "--quick");
            if rest.iter().any(|a| a != "--quick") {
                eprintln!("xtask: check accepts only --quick");
                usage();
                std::process::exit(2);
            }
            std::process::exit(check::run(quick));
        }
        Some("check-arch") => std::process::exit(arch::run()),
        Some("check-deps") => std::process::exit(deps::run()),
        Some("smoke") => std::process::exit(smoke::run(&args[1..])),
        Some("fixtures") => match args.get(1).map(String::as_str) {
            Some("generate") => std::process::exit(fixtures::run_generate()),
            Some("verify") => std::process::exit(fixtures::run_verify(&args[2..])),
            _ => {
                eprintln!("xtask: fixtures needs 'generate' or 'verify'");
                usage();
                std::process::exit(2);
            }
        },
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
