fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(commentcleanup::check::run(&arguments));
}
