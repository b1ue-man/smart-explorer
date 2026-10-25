fn main() {
    match smart_explorer::mcp_launcher::run_launcher() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("se-mcp-launcher: {error}");
            std::process::exit(1);
        }
    }
}
