# Umsetzung der ersten Öffi-Linie

> Für die Umsetzung gelten die in AGENTS.md festgehaltenen Skills. Jeder Schritt beginnt mit einer fehlschlagenden Probe und endet mit überprüftem Verhalten.

**Ziel** ist eine echte 3D-Brückenstadt, in der sich Verkehr beobachten und eine eigene Verbindung bauen lässt.

**Architektur** Godot 4.6 stellt Karte und Eingabe. Eine Rust-GDExtension liest und verändert dieselbe Simulation wie der Kommandozeilen-Runner. Der Kern bleibt unabhängig von der Engine.

**Entwurf** [Die erste eigene Öffi-Linie](../specs/2026-11-20-oberflaeche-erste-oeffi-linie-design.md)

- [x] Spielstart. Die Headless-Startprobe fordert die native Brücke an, prüft Weltgeometrie, Halte und die Weiterentwicklung der Uhr. Eine orthografische Kamera zeigt den Modellbau. Die Nahansicht stellt Bewegung, wartende Figuren und Haltestellen aus Kernbeobachtungen dar.
- [x] Herkunft und Halte. Die Kernabfrage liefert Quelle, Ziel, Menge und räumlichen Verlauf privater Fahrten. In der Karte überblendet die Auswahl den belasteten Abschnitt und dessen Herkunft. Halteentwürfe zeigen den Fußzugang. Prüfe leere Nachfrage, die Trennung durch den Fluss und Halte ohne Zugang.
- [x] Betrieb. Die Routenvorschau benutzt das Kernrouting. Bus und Tram teilen das Linienwerkzeug. Halte, Takt, Wegpunkte und Fahrzeuge werden gemeinsam übernommen. Prüfe den Hin- und Rückweg, gesperrte Spuren, verworfene Entwürfe und die Übereinstimmung zwischen Vorschau und Betrieb.
- [x] Spielprobe. Starte das Szenario mit einer Lücke im bestehenden Betrieb. Prüfe freies Erkunden, Kartenebenen ohne Handlungszwang, das Bauen und den sichtbaren Betrieb. Führe die bestehenden Rust-Tests und die neuen Godot-Proben aus.
