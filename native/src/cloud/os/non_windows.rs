pub fn open_url(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}
