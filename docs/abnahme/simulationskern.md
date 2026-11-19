# Abnahme des Simulationskerns

Die Brückenstadt beginnt mit 280 Einwohnern, zwei Wohnvierteln, einem Ortskern und einem Arbeitsschwerpunkt am anderen Ufer. Der Bus überquert den Fluss, die Tram übernimmt einen Teil der Fahrgäste und eingesetzte Taxis bedienen einzelne Aufträge. Die separate Lastkarte beginnt mit 10.000 Einwohnern und verteilt den Berufsverkehr auf sieben Abfahrtswellen.

## Fachliche Abnahme

| Fall | Beobachtung | Automatisierte Prüfung |
| --- | --- | --- |
| Blockierter Brückenkopf und Abbiegetasche | Drei Linksabbieger halten Bus 12 auf. Regulär passiert er bei Tick 13. Eine Tasche von 15 m zieht seine Durchfahrt auf Tick 6 vor. Bei 10 m bleibt ein Pkw vor dem Bus. | `sim-runner --test acceptance`, `sim-core --test bridge` |
| Rückstau, Konkurrenz und Gleis | Ein voller Zielabschnitt hält bereite Mengen zurück. Zwei dauerhaft berechtigte Zufahrten wechseln sich bei gleicher Priorität ab. Das eigene Tramgleis folgt seiner eigenen Freigabe. | `sim-core --test bridge` |
| Kapazität und Anschluss | Acht von zwölf Wartenden fahren mit. Die verbleibenden vier behalten ihren Ankunftstick. Die genau passende Gehzeit ermöglicht den Anschluss. In der Brückenstadt steigen 52 Reisende von Bus auf Tram um. Der kürzere Tramtakt senkt die gemessene Gesamtreisezeit auf demselben Vergleichsfenster. | `sim-core --test service`, `sim-runner --test acceptance` |
| Taxi | Zwei Taxis konkurrieren um einen Auftrag. Ein Fahrzeug reserviert ihn und Rückstau verlängert die Anfahrt. Die Fortsetzung bewahrt Abholung und Ankunft. | `sim-core --test taxi`, `sim-runner --test replay` |
| Umbau, Trennung und Undo | Änderungen übernehmen belegte Spuren nur mit sicherer Zuordnung. Abgetrennte private Ströme erreichen einen möglichen Straßenausstieg oder bleiben mit Wartegrund auf der Spur. Undo nach Neustart stellt den Zeitpunkt und Zustand vor dem Umbau wieder her. | `sim-core --test edit`, `sim-core --test undo`, `sim-runner --test acceptance` |
| Wachstum | Die Linie erschließt die Wiese. Drei Bauperioden später ziehen die ersten Bewohner ein. Das nächste Tagesprofil erzeugt zusätzliche Arbeitswege. Die Wachstumspause hält das Vorhaben an, während die Busse weiterfahren. | `sim-core --test growth`, `sim-runner --test acceptance` |
| Laden und Fortsetzen | Frische Prozesse setzen den Zustand bei Rot sowie mitten im Halt, Umstieg und Bauvorhaben fort. Die eingefrorene V1-Datei hält den Brückenkopf bei Tick 10 fest. Nach dem Laden fahren die ersten Pkw bei Tick 11 links und Bus 12 fährt bei Tick 13 geradeaus. | `sim-runner --test replay`, `sim-core --test continuation` |
| Fehlerfälle und Dateiablage | Abgeschnittene Dateien, ungültige Prüfsummen, Versionen, Mengen, Referenzen und Undo-Stände liefern gezielte Fehler. Schreibfehler vor dem Austausch erhalten den letzten gültigen Stand. Ein unsicherer Verzeichnis-Sync nach Austausch bleibt sichtbar. | `sim-core --test save_format`, `sim-runner --test file_store` |
| Bilanz und Befehlsfolgen | Die Abnahme führt reproduzierbare variable Umbauten mit Seed 423 durch, prüft gezielt abgelehnte Befehle und vergleicht Einzelticks gegen Blockfortschritt mit wiederholtem Laden. Sie prüft Reisebilanz, eindeutige Fahrzeugpositionen, Kapazitäten und Undo. | `sim-runner --test acceptance` |

## Ausführen

```sh
cargo test --workspace --locked
cargo run --release --locked -p sim-runner -- run --scenario scenarios/bridge-city.json --until-tick 150 --save /tmp/brueckenstadt.json
cargo run --release --locked -p sim-runner -- run --load /tmp/brueckenstadt.json --until-tick 240 --trace /tmp/brueckenstadt.jsonl
cargo test --release --locked -p sim-runner --test load one_day -- --ignored --nocapture
cargo test --release --locked -p sim-runner --test load five_years -- --ignored --nocapture
```

Der Lasttag misst 86.400 Verkehrsticks. Der Langzeitlauf zählt je 86.400 Verkehrsticks als einen Entwicklungsmonat und vergleicht den durchgehenden Zustand nach jedem Monat mit dem Ergebnis eines separaten Runner-Prozesses. 60 Prozesse prüfen fünf Spieljahre mit 5.184.000 Ticks je Verlauf und 59 Fortsetzungen aus einer gespeicherten Datei.

Die JSON-Zusammenfassung und die Tick-Spur enthalten Uhr, Spurbelegung, Ausfahrten, Nachfragebilanz, Linienbetrieb, Taxiaufträge und Wachstumsentscheidungen. `ready_queue_mm` schätzt die Staulänge am Spurkopf. Bei Einsatzfahrzeugen auf einer Spur zeigt `remaining_travel_ticks` die verbleibende freie Fahrzeit. `total_travel_ticks` wird verlustfrei als Dezimalzeichenkette ausgegeben. Die Tick-Spur erfasst den Zustand vor den Befehlen derselben Grenze, die Zusammenfassung enthält den Fingerabdruck des erreichten Stands.

Der Seed setzt die Startphase der täglichen Vergabe knapper Arbeitsplätze. Das Wachstum erhält einen eigenen Zufallszustand für konfigurierbare zusätzliche Bauperioden. Beide Szenarien verwenden die feste Bauzeit des Regelpakets. V1-Stände aus der Zeit vor den Zufallszuständen laufen mit ihrem ursprünglichen Regelverhalten weiter.

## Messumgebung

- AMD Ryzen 5 5600, 6 Kerne und 12 Threads, 32 GB RAM
- CachyOS x86-64 mit Linux 7.3.4-1-cachyos
- Rust und Cargo 1.100.0, Cargo-Profil `release`, Quellstand `2ab19a8`
- Durchsatz des Kerns mit einer Stichprobe der aktiven Mengen an jedem Tick. Spitzen-RSS als `VmHWM` des jeweiligen Testprozesses. Speicherzeit einschließlich Datei- und Verzeichnissynchronisierung. Ladezeit einschließlich Dateizugriff und vollständiger Zustandsprüfung.

## Lasttag mit 10.000 Einwohnern

`load-10000.json`, Seed 423. Bei Tick 86.400 sind alle 20.500 erzeugten Wege angekommen. Die Bevölkerung bleibt bei 10.000 Einwohnern. Der neu geladene Endstand stimmt in Abfragen und Fingerabdruck überein.

| Messgröße | Wert |
| --- | ---: |
| Laufzeit | 0,148 s |
| Durchsatz | 581.890 Ticks/s |
| Maximale aktive Reisemengen | 2.170 |
| Spitzen-RSS | 6.340 KiB |
| Spielstand | 13.593 Bytes |
| Speichern | 0,298 ms |
| Laden | 0,396 ms |

Der Lauf übertrifft das vorläufige Ziel von 60 Verkehrsticks pro realer Sekunde.

## Fünf Jahre in der Brückenstadt

`bridge-city.json`, Seed 41. Die Bevölkerung steigt von 280 auf 304 im vierten Monat und auf 328 im siebten Monat. Am Ende sind alle 31.064 erzeugten Wege angekommen. An allen 60 Monatsgrenzen stimmen Entwicklungszeit, Wachstum, Nachfrage, Linienbetrieb, Taxi, Spurzustände und Fingerabdrücke beider Verläufe überein.

| Messgröße | Wert |
| --- | ---: |
| Durchgehender Kernlauf | 2,197 s |
| Durchsatz des Kerns | 2.359.743 Ticks/s |
| Runner mit Prozessstarts und Dateiablage | 2,302 s · 2.252.248 Ticks/s |
| Maximale aktive Reisemengen | 82 |
| Spitzen-RSS | 5.680 KiB |
| Größter Monatsstand des Runners | 14.902 Bytes |
| Spielstand am Ende | 14.899 Bytes |
| Speichern am Ende | 0,413 ms |
| Laden der 60 Kontrollstände im Testprozess | 31 ms gesamt |

| Spieljahr | Tick | Fingerabdruck |
| ---: | ---: | --- |
| 1 | 1.036.800 | `38e4b5a6bb62fde90e1f085e07fe995d6ec589d0e2e39e2d6845b387f947c016` |
| 2 | 2.073.600 | `44d075a12a6dc8b094bd610bd57a4ca990ea1f6d47e921042316f836d94bdb08` |
| 3 | 3.110.400 | `0581f51ed25c2b54943e38e2565c2c9bb7e4338e704c928200e14543a36b0d31` |
| 4 | 4.147.200 | `dad71cb7fb2cf6990fee50a60ed8d2c395e9246b3894796510cd379f90a295aa` |
| 5 | 5.184.000 | `a6257fff8bdb32f19598bc1303ac99cef78f7c6e87ebee031dbcf82b3337b19a` |

Der Workspace-Lauf bestand alle 185 regulären Tests. Beide Release-Abnahmen wurden separat ausgeführt und bestanden. Rustfmt, Clippy und Rustdoc bestanden ebenfalls.
