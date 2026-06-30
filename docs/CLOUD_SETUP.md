# Google Drive einrichten (eigenes Google-Konto/Projekt)

Smart Explorer ist **kein Cloud-Dienst** — es speichert nichts auf fremden
Servern und es gibt keinen geteilten Zugang. Damit die App auf **dein** Google
Drive zugreifen darf, legst du in **deinem eigenen** Google-Konto einmalig einen
OAuth-Client an und trägst dessen **Client-ID** in der App ein. Deine Daten und
Tokens bleiben auf deinem Rechner (der Refresh-Token liegt im Windows Credential
Manager).

> Warum selbst anlegen? Eine frei verteilbare Desktop-App kann kein echtes
> geheimes Client-Secret ausliefern, und der Drive-Vollzugriff unterliegt Googles
> App-Verifizierung. Beides löst man sauber, indem **jede:r das eigene
> Google-Projekt** nutzt (so macht es z. B. auch rclone).

---

## Schritt für Schritt (ca. 5 Minuten, einmalig)

1. **Google Cloud Console öffnen:** <https://console.cloud.google.com>
   (oben links **Projekt anlegen**, z. B. „Smart Explorer“).
2. **Drive-API aktivieren:** Menü → **APIs & Dienste → Bibliothek** →
   „**Google Drive API**“ suchen → **Aktivieren**.
3. **OAuth-Zustimmungsbildschirm** (Menü → **APIs & Dienste → OAuth-Zustimmungsbildschirm**):
   - Nutzertyp **Extern** → **Erstellen**.
   - App-Name + deine E-Mail eintragen, Rest leer lassen, **Speichern**.
   - Unter **Testnutzer** **deine eigene Google-Adresse hinzufügen**.
   - Du kannst im Status **„Testing“** bleiben (keine Google-Prüfung nötig).
4. **OAuth-Client erstellen:** Menü → **APIs & Dienste → Anmeldedaten** →
   **Anmeldedaten erstellen → OAuth-Client-ID** → Anwendungstyp
   **Desktop-App** → **Erstellen**.
   - Es wird **keine Weiterleitungs-URI** benötigt — Desktop-Clients erlauben
     die lokale `127.0.0.1`-Rückleitung automatisch (Smart Explorer nutzt einen
     temporären lokalen Port).
5. **Client-ID kopieren** (Form `…apps.googleusercontent.com`). Google zeigt
   evtl. auch ein **Client-Secret** an — kopiere es mit (bei Desktop-Apps ist es
   nicht wirklich geheim, der Token-Endpunkt erwartet es aber).
6. **In Smart Explorer eintragen:** **⚙ Einstellungen → CLOUD (GOOGLE DRIVE)**:
   - **Client-ID** (und ggf. **Client-Secret**) einfügen.
   - **„Mit Google verbinden“** klicken → Browser öffnet sich → mit deinem
     (als Testnutzer hinterlegten) Google-Konto anmelden und zustimmen.
   - Danach steht oben **„● Verbunden“**. **„☁ Drive öffnen“** durchsucht dein
     Drive; in Sync-Setups erscheint Drive im Ordner-Picker als **„☁ Google
     Drive“**.

---

## Gut zu wissen

- **Token im „Testing“-Modus laufen nach ~7 Tagen ab.** Das ist eine
  Google-Vorgabe für sensible Bereiche (Drive-Vollzugriff). In dem Fall in der
  App einfach erneut **„Mit Google verbinden“** klicken. Wer das vermeiden will,
  kann den Zustimmungsbildschirm auf **„In Produktion“** setzen — dafür verlangt
  Google bei Vollzugriff aber eine Verifizierung. Für den persönlichen Gebrauch
  ist der Testing-Modus der einfachste Weg.
- **Berechtigung:** Smart Explorer fordert vollen Drive-Zugriff an, weil es
  bestehende Ordner durchsuchen und in beide Richtungen synchronisieren soll.
- **Wo liegt was?** Client-ID/Secret:
  `%APPDATA%\smart_explorer\cloud\gdrive.cfg`. Refresh-Token: Windows Credential
  Manager (Eintrag `cloud:gdrive`). **Trennen** in den Einstellungen löscht den
  Token.
- **Sync mit Drive:** Lege ein Sync-Setup an (⇄ Sync → Sync-Setups), wähle als
  Quelle/Ziel über **📂 → ☁ Google Drive** einen Drive-Ordner. Cloud-Sync-Jobs
  laufen auch im Hintergrund-Dienst, weil der Refresh-Token aus dem Credential
  Manager wiederverwendet wird. Einseitige Mirror-Jobs gegen Drive nutzen nach
  dem ersten Vollabgleich den lokalen Sync-Index
  `%APPDATA%\smart_explorer\sync\sync_state.sqlite` und die Drive Changes API;
  wenn der Cursor oder die Root-Identitaet nicht sicher passt, laeuft wieder der
  vollstaendige sichere Sync.

---

*Technischer Hintergrund (Entwicklung): [`CLOUD_OAUTH_PLAN.md`](CLOUD_OAUTH_PLAN.md).*
