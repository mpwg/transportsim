# Umsetzungsplan für den vollständigen Simulationskern

> Bei der Ausführung Arbeitspakete nach delegate-coding-efficiently und Analyseaufgaben nach delegate-work-efficiently delegieren. Die Aufgaben verwenden Checkboxen als Fortschrittsanzeige.

**Ziel** ist die vollständig ohne Oberfläche betreibbare Brückenstadt für bis zu 10.000 Einwohner einschließlich Umbau, Wachstum und exakt fortsetzbarer Spielstände.

**Architektur.** Ein Rust-Kern besitzt Welt und Simulationszeit. Private Wege sind ganzzahlige Mengen. Eingesetzte Fahrzeuge haben individuelle IDs und teilen sich mit den Mengen die geordneten Warteschlangen der jeweiligen Spurabschnitte. Ein separates Kommandozeilenprogramm übernimmt Dateizugriff, Befehlspläne und Abnahmeläufe.

**Technik.** Cargo-Workspace, Edition 2024, zum Planungszeitpunkt Rust und Cargo 1.100.0. Serde und serde_json für Datentransfer, BLAKE3 für Prüfsummen, tempfile für die atomare Dateiablage, proptest für Invarianten. Abhängigkeiten erst beim jeweiligen Bedarf hinzufügen. Der Workspace versioniert Cargo.lock und verwendet AGPL-3.0-only entsprechend der Projektlizenz.

**Spezifikation.** [Freigegebenes technisches Konzept](../specs/2026-11-19-simulationskern-design.md)

## Globale Vorgaben

- Ein Tick entspricht einer Verkehrssekunde. Das Entwicklungsjahr umfasst zwölf volle Verkehrstage zu je 86.400 Ticks. Die Wachstumsuhr kann separat pausieren.
- Ganzzahlige Mengen, getypte IDs, Ticks und Längen in Millimetern. Zwischenprodukte auf Überlauf prüfen. Reihenfolge und Gleichstände sind explizit. Nachfrage und Wachstum erhalten getrennte versionierte Zufallszustände.
- Genau ein schreibender Zugriff auf die Welt. Der abgeschlossene Zustand ist die Grenze für Befehle, Abfragen und Spielstände. Pro Tick höchstens ein Abschnittswechsel pro Fahrzeug oder Teilmenge.
- Jede Änderung umsetzbarer Fachlogik beginnt mit einem gezielten Test. Erwartete fachliche Abweichung im roten Lauf prüfen. Minimal implementieren, gezielt und mit der gesamten Suite grün prüfen, dann refaktorieren und erneut prüfen.
- Jede grüne, zusammenhängende Etappe direkt auf `main` committen und nach `origin/main` pushen. Schreibbereiche delegierter Pakete trennen, öffentliche Verträge vorher festlegen. Der Hauptagent integriert und prüft. An diesen Plan nur nach überprüfter Etappe das entsprechende Häkchen setzen.
- Vor jedem Commit `cargo fmt --all -- --check`, `cargo clippy --workspace --locked --all-targets --all-features -- -D warnings`, `cargo test --workspace --locked --all-features` und `git diff --cached --check` ausführen, sobald der Workspace existiert. Bei Änderungen an der öffentlichen API zusätzlich `cargo doc --workspace --no-deps --locked`.
- Autobahnwerkzeuge und grafische Oberfläche folgen nach diesem Meilenstein. Gerichtete Abschnitte und Einfädelbeziehungen bleiben im Netzmodell darstellbar.

## Dateien und Verträge

| Pfad | Verantwortung |
| --- | --- |
| `Cargo.toml`, `crates/sim-core/Cargo.toml`, `crates/sim-runner/Cargo.toml` | Workspace, Kern und ausführbarer Adapter |
| `crates/sim-core/src/lib.rs`, `simulation.rs`, `ids.rs`, `world.rs` | Öffentliche Exporte, Ticksteuerung, typisierte Identitäten und validierte Welt |
| `crates/sim-core/src/network.rs`, `traffic.rs`, `junction.rs` | Abschnittsgraph, geordnete Belegung, Signalkonflikte und Bewegungsvergabe |
| `crates/sim-core/src/demand.rs`, `routing.rs`, `service.rs`, `taxi.rs` | Reisen, Etappenplanung, Halte und individuelle Dienste |
| `crates/sim-core/src/edit.rs`, `growth.rs`, `observe.rs` | Bauvorschau und Undo, Stadtentwicklung, Diagnose und Messfenster |
| `crates/sim-core/src/save.rs`, `crates/sim-core/src/save/v1.rs` | Hülle und versionierte Ein-/Ausgabestrukturen, getrennt vom Domänenmodell |
| `crates/sim-runner/src/main.rs`, `file_store.rs`, `replay.rs` | Kommandozeile, haltbare Dateiablage und Befehlspläne |
| `crates/{sim-core,sim-runner}/tests/`, `scenarios/` | API- und Prozesstests, kleine Fixtures und spielbares Brückenstadt-Szenario |

`Simulation` exportiert folgende Kernoperationen. Öffentlich sichtbar sind die fachlichen Eingaben, Ergebnisse und Fehler, intern bleiben veränderliche Welt und Serialisierungsstrukturen.

```rust
pub fn new_game(scenario: Scenario, rules: RuleSet, seed: u64) -> Result<Self, ValidationError>;
pub fn apply(&mut self, command: Command) -> Result<CommandReceipt, CommandError>;
pub fn advance(&mut self, ticks: u64) -> Result<AdvanceReport, AdvanceError>;
pub fn step(&mut self) -> Result<AdvanceReport, AdvanceError>;
pub fn inspect(&self, query: Query) -> Result<Observation, QueryError>;
pub fn preview(&self, edit: &EditCommand) -> Result<EditPreview, CommandError>;
pub fn save(&self) -> Result<Vec<u8>, SaveError>;
pub fn load(bytes: &[u8]) -> Result<Self, LoadError>;
pub fn fingerprint(&self) -> Result<[u8; 32], SaveError>;
```

Ein `CommandReceipt` enthält Tick, Eingabesequenz, betroffene IDs und fachliche Folgen. `AdvanceReport` enthält Start- und Endtick sowie die Zahl ausgeführter Ticks. Der Runner kann pro Tick einen Ereignissuffix ausgeben. Der Kern hält Detailereignisse und Messfenster begrenzt. Für einen fehlgeschlagenen Tick bleibt der letzte abgeschlossene Zustand gültig. Beim Laden ersetzt der Aufrufer seine Simulation erst nach erfolgreichem `load`.

## Fünf besonders fehleranfällige Eingaben

| Eingabe | Erwartung und Testaufgabe |
| --- | --- |
| Eine Bauvorschau wird später auf inzwischen belegtem Netz bestätigt | Bestätigung prüft die aktuelle Belegung vollständig erneut und weist eine veraltete Netzrevision ab. Paket 8. |
| Eine funktionierende Datei hat eine gültige Prüfsumme, aber im Undo eine ungültige Referenz | Laden meldet den Pfad in der Undo-Struktur. Der bisherige Spielstand läuft weiter. Paket 9. |
| Ein verfügbares Taxi und zwei gleichzeitige Abholwünsche konkurrieren | Eine Reservierung pro Fahrzeug und Wunsch. Paket 7. |
| Ein Pendler erreicht den Arbeitsplatz erst nach dem geplanten Rückfahrtfenster | Der Heimweg entsteht erst nach der Ankunft und genau einmal. Paket 5. |
| Austausch der Datei erfolgreich, anschließende Synchronisierung des Verzeichnisses fehlerhaft | Neue Datei ist vollständig lesbar, Ergebnis meldet unbestätigte Haltbarkeit. Paket 4. |

## Etappe A · Die Brücke von einem Spielstand aus betreiben

### 1. Startzustand und Tickgrenzen

**Dateien** `Cargo.toml`, `.gitignore`, `crates/sim-core/{Cargo.toml,src/{lib,ids,simulation,world,observe}.rs,tests/clock.rs}`.

- [x] Workspace und API-Grenze anlegen. Test `pause_and_single_step_preserve_boundary_order` mit Starttick 10, Gesamtpause, `advance(7)` → 10, `step()` → 11, Fortsetzen und `advance(3)` → 14. Weitere Tests erwarten eine ID samt Feld für doppelte IDs und einen Fehler vor Überlauf der Tickgrenze.
- [x] `cargo test -p sim-core --test clock` ausführen und die fachliche Abweichung prüfen.
- [x] `Simulation::new_game`, `apply`, `step`, `advance` und `inspect` für Takt, Validierung, Pausen und Eingabereihenfolge implementieren. Als interne Zustände getrennte Verkehrs- und Entwicklungsuhren führen.
- [x] Einzeltest und Workspace-Prüfungen wiederholen. Commit `feat: Simulationszeit und Weltvalidierung`.

### 2. Geordnete Spuren und Kreuzungen

**Dateien** `crates/sim-core/src/{network,traffic,junction,simulation,observe}.rs`, `crates/sim-core/tests/bridge.rs`.

- [x] `bridge` testet `[3 Pkw links, Bus 12 gerade, 5 Pkw gerade]` bei Geradeausgrün und roter Linksbewegung. Erwartet sind null Übertritte und Wartegrund am Kopf. Mit freiem legalem Ziel und Freigabe muss Bus 12 anschließend als einzelnes Fahrzeug kreuzen. Bei vollem Ziel bleibt er stehen. Für zwei gleichrangige unverändert bereite Zufahrten mit einem Slot lautet die erwartete Folge A, B, A, B. Fahrt über einen leeren Abschnitt benötigt weiterhin seine freie Fahrzeit.
- [x] `cargo test -p sim-core --test bridge` rot prüfen.
- [x] `RoadState::plan_moves(&self, tick: Tick) -> Vec<MoveRequest>` und `RoadState::commit_moves(&mut self, moves: &[GrantedMove])` intern hinter der Tickgrenze verdrahten. Bewegungen aus einem gemeinsamen Vorzustand planen und Platz sowie Konfliktfelder vor der Übernahme reservieren. Einen halben Pkw Durchsatz pro Grün-Tick als zwei Ticks pro Übertritt testen. Langes Rot darf daraus keinen Vorrat erzeugen. Getrennte Gleisfreigabe erhält einen eigenen Test.
- [x] Einzeltest und Workspace-Prüfungen grün. Commit `feat: Spurwarteschlangen und Kreuzungsfreigaben`.

### 3. Spielstand V1 und Befehlswiedergabe

**Dateien** `crates/sim-core/src/{save.rs,save/v1.rs}`, `crates/sim-core/tests/{save_format,continuation}.rs`, `crates/sim-runner/{Cargo.toml,src/{main,replay}.rs,tests/replay.rs}`, `scenarios/bridge-test.json`.

- [x] Test `restart_during_red_preserves_first_legal_crossing` mit Checkpoint bei Tick 10 und erster zulässiger Geradeausfahrt für Bus 12 bei Tick 13. Ein zweiter Checkpoint nach A prüft B, A, B als exakten Ereignissuffix. A verwendet Einzelticks, B startet einen frischen Prozess und verwendet Blöcke von 3, 7 und 17 Ticks, an den Befehlsgrenzen geschnitten. Erwartet sind gleiche Abfragen, Ereignisse und Fingerabdrücke an den Vergleichspunkten. Mehrfaches Save und zusätzliche Abfragen ändern den Fingerabdruck nicht.
- [x] `cargo test -p sim-core --test continuation` und `cargo test -p sim-runner --test replay` rot prüfen. Zuerst auch `cargo test -p sim-core --test save_format` mit den unten genannten Fehlerfällen rot prüfen.
- [x] `save`, `load` und `fingerprint` implementieren. JSON-Hülle mit `format_version`, `rules_version`, `checksum`, `payload`. V1 hasht die kanonische UTF-8-Nutzlast mit BLAKE3. Die Nutzlast verwendet feste Feldreihenfolge, nach stabiler ID sortierte Tabellen, geordnete Warteschlangen und dezimale Ganzzahlen. Whitespace und die Reihenfolge von JSON-Objektschlüsseln beeinflussen die Prüfsumme nicht. Doppelte Schlüssel zurückweisen. Eingabe auf 128 MiB begrenzen. Format- und Regelversion getrennt prüfen, danach ausschließlich validierte DTOs in die Welt umwandeln. Regeln, Startdaten, laufende Uhren, IDs, Sequenz, Warteschlangen, Ratenreste, Freigaben und Vorrangposition persistieren. Extern geänderte Szenariodateien dürfen das Laden nicht beeinflussen.
- [x] Runner `sim-runner run --scenario PFAD | --load PFAD` als zwei alternative Eingabeformen vorsehen. Optionen `--commands PFAD`, `--until-tick N`, `--save PFAD` und `--trace PFAD`. Befehlsdatei ist eine geordnete Liste mit Tick und Befehl. Bei mehreren Befehlen am Rand gilt Eintragsreihenfolge. Absolute Tickgrenzen verwenden, überholte Befehle und eine pausierte, unerreichbare Zielzeit gezielt melden. Laufende Reihen und V1-Referenz am Ende in Paket 11 einfrieren.
- [x] `cargo test -p sim-core --test save_format` prüft abgeschnittenes JSON, falsche Prüfsumme, höheres Format, inkompatible Regeln und Grenze + 1 Byte. Referenzfehler und negative Mengen mit neu berechneter Prüfsumme einschleusen. Einzeltests und Workspace grün. Commit `feat: Spielstand und deterministische Wiedergabe`.

### 4. Haltbare Dateiablage

**Dateien** `crates/sim-runner/src/{file_store,main}.rs`, `crates/sim-runner/tests/file_store.rs`.

- [x] `atomic_save_keeps_previous_file_until_replace` mit echtem temporärem Verzeichnis und zwei unterschiedlichen gültigen Spielständen. Einen kurzen Schreibvorgang, einen Schreibfehler, Fehler beim Datei-Sync und Fehler beim Austausch gezielt auslösen. Vor Austausch bleibt O bytegleich und ladbar. Nach Austausch und fehlgeschlagenem Verzeichnis-Sync ist N vollständig ladbar, Status `DurabilityUnconfirmed`. Zwei konkurrierende Writer für dasselbe Pfadziel werden serialisiert und Leser sehen nur vollständige Dateien.
- [x] `cargo test -p sim-runner --test file_store` rot prüfen.
- [x] `FileStore::write(&self, path: &Path, bytes: &[u8]) -> Result<(), StoreError>` und `FileStore::read(&self, path: &Path) -> Result<Vec<u8>, StoreError>` implementieren. Dateiablage begrenzt das Lesen schon vor vollständiger Allokation. Unter einem exklusiven zielbezogenen Lock im selben Verzeichnis temporär schreiben, Datei synchronisieren, atomar ersetzen und erforderlichen Verzeichnis-Sync durchführen. Pfad-Aliasse über kanonisches Elternverzeichnis behandeln. Die Sperre bei jedem Rückgabepfad zuverlässig freigeben. Änderungen an einem Ziel und sein Fehlerstatus dürfen keine fremde Datei verändern.
- [x] Einzeltest inklusive erneutem Versuch nach Fehler und Workspace grün. Commit `feat: Spielstände atomar und haltbar ablegen`.

## Etappe B · Vollständige Reisen abwickeln

### 5. Nachfrage, Reiseplanung und Fußverkehr

**Dateien** `crates/sim-core/src/{demand,routing,network,observe}.rs`, `crates/sim-core/tests/demand.rs`.

- [x] `demand` prüft halbe Reise pro Fenster → 0, 1; zehn Arbeitsuchende bei sechs erreichbaren Plätzen → sechs zugeteilt und vier sichtbar unversorgt. Von sechs Hinwegen kommen vier an → vier Heimwege, ein später eintreffender Pendler erzeugt seinen Heimweg erst nach der Ankunft. Bei Modusgewichten 2:1 erhält eine Menge von sechs die Aufteilung 4:2. Bei roter Fußquerung oder vollem Folgeabschnitt wartet die Menge.
- [x] `cargo test -p sim-core --test demand` rot prüfen.
- [x] Intern `DemandState::open_day(&mut self, world: &World, rules: &RuleSet)` und `Router::alternatives(&self, origin: ZoneId, destination: ZoneId, at: Tick) -> Vec<JourneyOption>` einführen. Stabile Rasterzonen, täglich rotierende Zielzuteilung, Restmengen und Rückfahrtkonto. Kanten nach Netzberechtigungen durchsuchen. Reisekosten als Komponenten für Zugang, Warten, Fahrt und Umstieg erfassen. Kosten aus Fahrplänen und abgeschlossenen Messfenstern je Planungsfenster gewinnen. Verteilung ganzzahlig mit stabilem Rest. Gestartete gültige Routen beibehalten, außer am Umstieg oder bei notwendiger Umplanung. Messung beginnt bereits am Quellzugang. Fußetappen durchlaufen ihre eigene Laufzeit und Querschnittskapazität.
- [x] Save/Load bei halber Restmenge und offenem Rückfahrtkonto, Bilanz und Optionen über öffentliche Abfragen prüfen. Workspace grün. Commit `feat: Quell-Ziel-Nachfrage und Reiseetappen`.

### 6. Linien, Umstiege, Bus und Straßenbahn

**Dateien** `crates/sim-core/src/{service,routing,traffic,observe}.rs`, `crates/sim-core/tests/service.rs`.

- [x] `service` testet 12 Wartende, 8 freie Plätze → acht fahren, vier behalten ihren ursprünglichen Ankunftstick. Vier Umsteiger mit zwei Ticks Restgehzeit sind genau beim zweiten Tick verfügbar. Bei drei Ticks Mindesthalt darf der folgende Pkw auf der gleichen Spur vorher nicht passieren. Vergleichsläufe mit geändertem Takt weisen verpassten bzw. erreichten Anschluss und eine niedrigere Gesamtreisezeit aus. Fehlende Fahrzeuge erzeugen Verspätung oder Ausfall.
- [x] `cargo test -p sim-core --test service` rot prüfen.
- [x] Intern `ServiceState::tick_stops(&mut self, tick: Tick, …)` vor Bewegungsplanung aufrufen. Genau ein Fahrzeug pro Einsatz, Abfahrtsaufträge getrennt von der Flotte. Erst Ausstieg, danach Einstieg stabil nach Ankunft; endliche Halteplätze, Mindesthalt plus Fahrgastwechsel, Umstiegswege zum konkreten Halt. Gleisberechtigung und Halt in Bucht oder auf Spur beachten. Noch fällige Abfahrten bei Taktänderung erhalten, künftige Aufträge aus dem neuen Takt erzeugen. Eine Abfahrt verfällt spätestens beim nächsten geplanten Auftrag derselben Richtung, wenn sie bis dahin keinem Fahrzeug zugewiesen wurde.
- [x] Fortsetzung mitten im Halt und mitten im Fußumstieg in frischem Prozess testen. Workspace grün. Commit `feat: Linienfahrzeuge und Umstiege betreiben`.

### 7. Taxifahrzeuge disponieren

**Dateien** `crates/sim-core/src/{taxi,service,demand,traffic,observe}.rs`, `crates/sim-core/tests/taxi.rs`.

- [x] `taxi` testet gleich nahe Fahrzeuge 7 und 9 → nur 7 reserviert den Wunsch. Ein zweiter gleichzeitiger Wunsch wird nicht zusätzlich an 7 vergeben. Reservierte Reisemengen zählen bis zur Abholung einmal als wartend. Eine freie Strecke mit zwei Ticks Anfahrt und drei Ticks besetzter Fahrt wird gegen dieselbe Strecke mit Rückstau verglichen.
- [x] `cargo test -p sim-core --test taxi` rot prüfen.
- [x] `TaxiState::dispatch(&mut self, tick: Tick, …)` bei der Halteabwicklung vor Bewegungsplanung verdrahten. Ältester bedienbarer Wunsch, kürzeste erreichbare Leerfahrt, dann ID. Gleiche Ziele und Abholorte bis zur Fahrzeugkapazität bündeln. Die Zustände und Buchung von Abholung, Reise und Ankunft atomar wechseln. Leerfahrt belegt denselben Spurraum wie besetzte Fahrt.
- [x] Neustarts während Leerfahrt und mit Fahrgast vergleichen. Einzeltest und Workspace grün. Commit `feat: Taxiaufträge einzeln disponieren`.

## Etappe C · Umbauen und experimentieren

### 8. Bauen, belegen und neu planen

**Dateien** `crates/sim-core/src/{edit,world,network,routing,traffic,service,taxi}.rs`, `crates/sim-core/tests/edit.rs`.

- [x] `edit` testet bei der Brücke eine Tasche für drei Pkw und eine für zwei. Erstere erlaubt Bus 12 die Fahrt beim nächsten Geradeausgrün, letztere lässt einen Linksabbieger vor ihm. Prüfe eine veraltete Revision, inzwischen volle Zielspur, widersprüchliche Ampelphase, versetztes Gebäude mit gleicher ID und Einwohnerzahl und unerreichbaren ÖPNV-Halt. Prüfe unterbrochene Privatfahrt mit erreichbarer Ausstiegszufahrt und ohne jeden Ausstieg.
- [x] `cargo test -p sim-core --test edit` rot prüfen.
- [x] `Simulation::preview` und `Command::CommitEdit` mit gleicher Prüfung auf Revision und gegenwärtiger Belegung verdrahten. Die Vorschau ist rein lesend, die Bestätigung übernimmt den vollständigen vorbereiteten Zustand. Straßenverbindungen, Fahrtrichtungen, Spuren, Signale, Gehwege, Begrünung, Gleise, Halte, Linien und Takte als typisierte Varianten modellieren. Betroffene Einträge nur physisch erreichbar umlegen. Lage der Abbiegetasche und Reihenfolge erhalten. Bei unterbrochener Linie oder Taxifahrt bleiben Auftrag und Belegung nachvollziehbar. Abgeschnittene Privatmengen verlassen die Straße nur an erreichbaren Ausstiegen und bleiben andernfalls mit Wartegrund auf der Spur. Geänderte Verbindungen lösen gezielte Neuplanung aus.
- [x] Beide Privatfahrtfälle nach Neustart prüfen. Einzeltest und Workspace grün. Commit `feat: Straßen und Linien transaktional umbauen`.

### 9. Undo, Ortsmarken und vollständige Ladevalidierung

**Dateien** `crates/sim-core/src/{simulation,edit,marker,observe,save.rs,save/v1.rs}`, `crates/sim-core/tests/{edit,save_format,undo}.rs`.

- [x] Vor Umbau bei Tick 10 fachlichen Zustand abfragen; danach pausieren, fehlerhaften zweiten Umbau versuchen, weiterschalten, speichern, Prozess neu starten, Undo. Erwartet sind Tick 10, vorherige Welt und leerer Undo-Punkt. Bei entferntem Objekt bleibt die Ortsmarke an ihrer Koordinate. Erneuerter gültiger Umbau ersetzt den vorherigen Wiederherstellungspunkt; keine verschachtelte Historie.
- [x] `cargo test -p sim-core --test undo` und `cargo test -p sim-core --test save_format` rot prüfen.
- [x] Wiederherstellungspunkt ohne eigene Historie unmittelbar vor erfolgreichem Spielerumbau aufnehmen. Undo stellt Uhren, Regeln, Zufalls- und Verkehrszustände wieder her. Abfragen liefern Zielzeit vor Bestätigung. Vollständigen Validator auch auf den Undo-Punkt anwenden. Tests mit neu berechneter Prüfsumme für doppelte IDs, fehlende Referenzen, überfüllte Spur, unmögliche Zeit, doppelte Fahrzeugposition, ungültige Reservierung, Mengenbilanz und kaputten Undo-Punkt. Nach Ladefehler den laufenden Stand gegen einen Kontrolltick vergleichen.
- [x] Einzeltests und Workspace grün. Commit `feat: Umbauten nach Neustart rückgängig machen`.

## Etappe D · Stadtentwicklung und Abnahme

### 10. Wachsen, pausieren und weiterbauen

**Dateien** `crates/sim-core/src/{growth,world,demand,routing,observe,simulation,save/v1}.rs`, `crates/sim-core/tests/growth.rs`.

- [x] Zwei identische Startstände mit erschließbarer Wiese und Arbeitsplätzen vergleichen. Nur einer erhält eine Linie. Seine Erreichbarkeit steigt, ein Bau mit drei Perioden Laufzeit aktiviert erst bei Fertigstellung Bevölkerung und am nächsten Tagesprofil entstehen Arbeitswege. Speichern nach erster Bauperiode lässt genau zwei weitere offen. Wachstumspause über zwei Periodengrenzen friert Fortschritt und Messfenster ein, Verkehr läuft und es gibt keine Nachholung. Zusätzlicher Grenzfall 9.999 Einwohner und zwei mögliche Zuzüge.
- [x] `cargo test -p sim-core --test growth` rot prüfen.
- [x] `GrowthState::close_period(&mut self, world: &mut World, …)` am Ende der Tickphasen einbinden. Erreichbarkeit, gemessene Bedienung, Bedarf und Fläche für deterministisch geordnete Kandidaten verwenden. Regelpaket legt Gewichte und Schwellen fest. Lokale Erschließung protokolliert ihr Motiv und respektiert Gelände, Brücken und Spielerinfrastruktur. Bauvorhaben reservieren Flächen und besitzen Restdauer. Weiterbau und Verdichtung berücksichtigen die Grenze von 10.000 Einwohnern. Baufortschritt und wirksame Messfenster zusammen pausieren.
- [x] Vergleich, Pause und neuer Quell-Ziel-Strom nach Neustart prüfen. Einzeltest und Workspace grün. Commit `feat: Stadtwachstum aus Erreichbarkeit berechnen`.

### 11. Brückenstadt ausliefern und Fortsetzung abnehmen

**Dateien** `scenarios/bridge-city.json`, `scenarios/load-10000.json`, `crates/sim-runner/tests/{acceptance,load}.rs`, `crates/sim-runner/tests/fixtures/save-v1.json`, `docs/abnahme/simulationskern.md`.

- [ ] Vor Aufbau der Szenarien einen Runner-Test für Start, Verhalten und Endzustand schreiben. Das spielbare Szenario enthält Ortskern, Wohnviertel, Arbeitsschwerpunkt, Fluss und Brücke, Bus- und Tramanschluss sowie Taxi. Die Lastkarte startet mit 10.000 Einwohnern und enthält Berufsverkehr und die Störung am Brückenkopf. Zuerst `cargo test -p sim-runner --test acceptance` gegen die noch fehlenden Szenarien rot prüfen.
- [ ] Im kurzen öffentlichen Testlauf die Abnahmefälle des Konzepts ausführen. Die drei Zeithorizonte anhand nachvollziehbarer Beobachtungen prüfen. Feste Seeds für gültige und gezielt ungültige Befehlsketten verwenden. Invarianten pro Kontrollpunkt prüfen: erzeugt = wartend + unterwegs + angekommen, eine Fahrzeugposition, gültige Fahrzeug- und Spurbelegung. Abfrage- und Advance-Taktung variieren. V1-Referenz mit manuell erwarteten Fachwerten und nachfolgenden Ereignissen einfrieren. Künftige Formatmigration benötigt je Altversion eine solche Referenz.
- [ ] `cargo test --release -p sim-runner --test load one_day -- --ignored --nocapture` führt 86.400 Ticks aus. `cargo test --release -p sim-runner --test load five_years -- --ignored --nocapture` führt 5.184.000 Ticks aus. Monatliche fachliche Prüfpunkte und Jahresfingerabdrücke festhalten. Mehrfach in frischem Prozess nach Laden fortsetzen und Kontrollzustände gegen den durchgehenden Lauf vergleichen. Last und Langzeitlauf können unabhängig gestartet werden. Für den Fünfjahreslauf sind bei 60 Ticks/s rund 24 reale Stunden pro vollständigem Durchlauf einzuplanen.
- [ ] Referenzgerät, Betriebssystem, Build, Seed, Szenario, gemessene Ticks/s, Spitzen-RSS, maximale Zahl aktiver Mengen, Dateigröße sowie Lade- und Speicherzeit erfassen. Das vorläufige Ziel sind im eintägigen Lastfall durchschnittlich 60 Ticks/s oder mehr. Bei verfehltem Ziel Hotspots messen, Engpass korrigieren und die Fortsetzungstests erneut ausführen. Messwerte und tatsächlichen Abnahmestand im Abnahmebericht dokumentieren.
- [ ] Workspace vollständig prüfen. Commit `test: Brückenstadt und Fünfjahreslauf abnehmen`, zu `origin/main` pushen und Remote-Commit prüfen. Vollständig ist der Kern nach den nachweislich ausgeführten fachlichen Abnahmen, dem Lastfall und dem Fünfjahreslauf.

## Reihenfolge und Delegation

Der Hauptagent hält die Schnittstellen, integriert und verantwortet die Commits. Nach Festlegung der Spielstandhülle können Dateiadapter und Kernkonvertierung getrennt entstehen. Nach Stabilisierung von Fahrzeugidentität, Reiseetappen und Tickreihenfolge sind Linien- und Taxilogik getrennte Schreibbereiche, die Änderungen an gemeinsamen Modulen integriert der Hauptagent. Lastkarten und Abnahmedaten können anhand des öffentlichen Szenarioformats vorbereitet werden, während das Wachstum implementiert wird. Bei Änderungen gemeinsamer Verträge Pakete serialisieren. Bei Implementierung die Rust-Skills zu API, typisierten Zuständen, Fehlern, Serde, Tests und idiomatischem Rust am jeweiligen Paket einsetzen. Für Leistungsarbeit erst reproduzierbar messen und dann den Rust-Performance-Skill anwenden.
