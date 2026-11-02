pub fn open_url(url: &str) -> Result<(), String> {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    Ok(())
}
