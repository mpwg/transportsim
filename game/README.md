# Brückenstadt starten

Benötigt Godot 4.6 und Rust unter Linux x86-64.

## In VS Code

Öffne den Projektstamm `transportsim` und drücke **Strg+Umschalt+B**. Alternativ wählst du **Brückenstadt starten** im Menü **Terminal → Aufgabe ausführen**. Die Aufgabe baut die Rust-Erweiterung, stellt sie für Godot bereit und öffnet das Spiel. Godot muss als `godot` im Suchpfad oder unter `~/.local/bin/godot` verfügbar sein.

## Im Terminal

Führe die Befehle aus dem Projektstamm aus.

```sh
cargo build --locked -p sim-godot
mkdir -p game/native
cp target/debug/libsim_godot.so game/native/
godot --path game
```

Hast du Godot als Einzeldatei heruntergeladen, verwende ihren Pfad anstelle von `godot`.

## Bedienen

- **Mausrad** zoomt zur Mausposition. Ziehe die Karte mit der Maus. Beim Linienbau verschiebst du die Karte mit der rechten oder mittleren Maustaste.
- **Leertaste** pausiert und setzt den Betrieb fort. Die Kamera und der Linienentwurf bleiben verfügbar.
- **Nächster Morgen** springt zur nächsten Verkehrswelle. So kannst du die neue Linie beim Berufsverkehr beobachten.
- **Wartende** hebt die wartenden Gruppen auf der Karte hervor.
- Wähle eine Straße und schalte **Autowege** ein. Wähle einen hervorgehobenen Herkunftsblock, um dessen Wege zu verfolgen. Die Darstellung zeigt aktive Fahrten, die auf dem ausgewählten Abschnitt sind oder ihn noch vor sich haben.
- **Linie bauen** öffnet die Auswahl zwischen Bus und Straßenbahn. Fahre mit der Maus über mögliche Halte an Straßenabschnitten mit Fußzugang. Die Karte zeigt die erreichbaren Häuser und Fußverbindungen. Setze mindestens zwei Halte, ziehe den eingeblendeten Fahrweg bei Bedarf über eine andere Straße oder Gleisverbindung und wähle den Takt in Minuten.
- **Linie starten** übernimmt den Entwurf. Wähle im selben Werkzeug eine eigene Linie aus, um Strecke oder Takt zu ändern. **Esc** nimmt den letzten gesetzten Halt im Entwurf zurück.
- **?** blendet die Hinweise zur aktuellen Aktion ein oder aus.

## Prüfen

```sh
cargo test --workspace --locked
godot --headless --path game --script tests/world_smoke.gd
godot --headless --path game --script tests/play_smoke.gd
godot --headless --path game --script tests/onboarding_smoke.gd
godot --headless --path game --script tests/line_smoke.gd
godot --headless --path game --script tests/city_test.gd
```
