fn main() {
    match std::env::args().nth(1).as_deref() {
        Some(command) => println!("xtask: '{command}' is not implemented"),
        None => println!("xtask: not implemented"),
    }
}
