pub(super) fn missing(
    scheme: crate::vfs::Scheme,
    capabilities: crate::vfs::StagedWriteCapabilities,
) -> String {
    let mut missing = Vec::new();
    if !capabilities.create {
        missing.push("sicheres Erstellen neuer Dateien");
    }
    if !capabilities.replace {
        missing.push("sicheres Speichern ueber vorhandene Dateien");
    }
    if !capabilities.namespace_replace {
        missing.push("atomares Temp-Datei-zu-Zieldatei-Ersetzen");
    }
    format!(
        "Schreibbares Laufwerk abgelehnt: Das aktive {scheme:?}-Backend garantiert nicht {}. Das kann nach einem Verbindungs-Fallback anders ausfallen; bitte schreibgeschuetzt einbinden.",
        missing.join(", ")
    )
}
