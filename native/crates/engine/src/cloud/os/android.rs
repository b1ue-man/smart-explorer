/// Android has no URL-opening process an app could spawn: the Kotlin host
/// registers an opener (`set_url_opener`) that starts the browser. Reaching
/// this adapter means none is registered, so the sign-in cannot continue.
pub fn open_url(_url: &str) -> Result<(), String> {
    Err("Der Browser kann ohne App-Anbindung nicht geöffnet werden".into())
}
