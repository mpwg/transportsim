# Technisches Konzept für den Simulationskern der wachsenden Modellstadt

**Entwurf zur Freigabe · 19. November 2026**

Der erste Meilenstein ist erreicht, wenn die handgebaute Brückenstadt allein durch Kernbefehle über mehrere Spieljahre betrieben, umgebaut, gespeichert und exakt fortgesetzt werden kann. Die Stadt unterstützt bis zu 10.000 Einwohner. Ob eine Änderung hilft, lässt sich an der nächsten Ampelphase, am nächsten Anschluss und später an der Entwicklung eines Viertels ablesen. Die fachliche Grundlage ist das [Spielkonzept der wachsenden Modellstadt](2026-11-19-wachsende-modellstadt-design.md).

## 1. Was „vollständig“ für diesen Meilenstein heißt

| Im Kern ausführbar | Woran man es erkennt |
| --- | --- |
| Stadt und Netz anlegen | Ein Szenario beschreibt Gelände, Fluss, Brücke, Straßen, Gehwege, Gleise, Gebäude, Bevölkerung, Linien und Startfahrzeuge. Dieselben Regeln prüfen die Brückenstadt und kleine Testkarten. |
| Verkehr erzeugen und abwickeln | Wege entstehen nach Quelle, Ziel, Tageszeit und Menge. Fußverkehr, private Autos, Bus, Taxi und Straßenbahn teilen sich Reisende, Verkehrsraum und Umsteigepunkte. |
| Werkzeuge des Sandkastens anwenden | Befehle verbinden und trennen Straßen, ändern Einbahnen, Spurzuordnung, Abbiegespuren, Ampeln, Gehsteige und Begrünung. Weitere Befehle verwalten Gleise, Haltepunkte, Linien, Takte und Fahrzeugeinsätze. Die Stadt kann während einer Gesamtpause umgebaut werden. |
| Folgen über längere Zeit berechnen | Die Stadt baut lokale Straßen, Wohnungen, Geschäfte und Arbeitsstätten und verdichtet geeignete Standorte. Eine eigene Wachstumspause lässt den Tagesverkehr weiterlaufen. |
| Beobachten und experimentieren | Abfragen erklären Warteursachen, Verkehrsmittelwahl, Anschlüsse und Standortentscheidungen. Der Spielstand enthält Markierungen und den letzten rücknehmbaren Umbau. Zwei unabhängige Läufe aus demselben Stand machen Eingriffe vergleichbar. |
| Spielstand fortsetzen | Speichern und Laden funktioniert auch mit vollem Bus, halbfertigem Umstieg, unterwegs befindlichem Taxi, blockierter Kreuzung und pausiertem Wachstum. |

Der Meilenstein folgt dem Umfang der ersten Brückenstadt, ergänzt um die ausdrücklich gewünschten Taxis. Für den anschließenden Autobahnausbau sind gerichtete Fahrbahnen, zulässige Fahrzeugarten, Einfädelbewegungen und endlicher Stauraum bereits darstellbar. Autobahnwerkzeuge, Anschlussbauformen und deren eigene Abnahme gehören zur nächsten Ausbaustufe. Grafik, Ton und Bedienung erhalten eine Schnittstelle zum Kern. Geld, Freischaltungen und Spielverlust sieht der freie Sandkasten nicht vor.

## 2. Architekturentscheidung

| Ansatz | Vorteil für dieses Spiel | Aufwand oder Grenze |
| --- | --- | --- |
| **Spurabschnitte mit geordneten Warteschlangen** | Ein konkreter Bus kann hinter drei Linksabbiegern stehen. Begrenzter Stauraum überträgt den Rückstau bis zur vorherigen Kreuzung. Die Reihenfolge lässt sich inspizieren und speichern. | Die Position privater Autos innerhalb einer Menge ist angenähert. Die Einteilung in Abschnitte bestimmt, wo eine Spurwahl möglich ist. |
| **Kurze Zellen mit Dichte und Fluss** | Die Ausbreitung von Stauwellen innerhalb langer Straßen lässt sich räumlich feiner auflösen. | Mehr Zustände pro Straße. Für einen bestimmten Linienbus und seine Position relativ zu den Linksabbiegern wäre zusätzliche Logik nötig. |

**Gemeinsam gewählt** sind Warteschlangen pro Spurabschnitt, aggregierte private Wege und einzeln geführte Einsatzfahrzeuge. Ein privater Weg ist über seine Quelle, sein Ziel, seine Menge und seine Reiseetappen erklärbar. Ein Bus hat über seine gesamte Einsatzdauer eine eigene ID.

**Ich schlage eine eigenständige Rust-Bibliothek vor**, ergänzt um ein kleines Kommandozeilenprogramm für Szenarien, Diagnose und Abnahme. Der Projektstand enthält das Spielkonzept und Konzeptbilder. Eine Laufzeit oder Engine ist bislang nicht festgelegt. Rust bietet hier typisierte Einheiten, Tests ohne Engine und einen eigenständig baubaren Kern. Dateisystem, Echtzeituhr, Eingabegeräte, Audio und Rendering liegen in Adaptern. Die spätere Oberfläche kann denselben Kern einbetten.

| Baustein | Besitzt und entscheidet |
| --- | --- |
| Welt und Netz | Gelände, Parzellen, Gebäude, stabile IDs, gerichtete Spuren, Fahrbeziehungen, Fuß- und Gleisnetz, Geometrie und Bauregeln |
| Nachfrage und Reiseplanung | Quell-Ziel-Mengen, Tagesprofile, Wegezweck, Verkehrsmittelwahl, Etappen, Umstiege und unerfüllte Nachfrage |
| Verkehr und Betrieb | Spurbelegung, Ampeln, Freigaben, Rückstau, Fußwege, Halte, Fahrzeugzustände, Linien und Taxiaufträge |
| Stadtentwicklung | Erreichbarkeit, freie Bauflächen, lokale Erschließung, Baufortschritt, Bevölkerung und Arbeitsplätze |
| Simulationssteuerung | Einziger schreibender Zugriff auf die Welt, Befehlsreihenfolge, Zeit, Pausen, Undo und veröffentlichte Beobachtungen |
| Szenarien und Spielstände | Stabile externe Datenverträge, Validierung, Migration und Wiederherstellung. Ein Speicheradapter übernimmt die haltbare Dateiablage. |

Ein Szenario liefert Anfangsdaten, das Regelpaket liefert das Verhalten, der Spielstand liefert die Fortsetzung. Dadurch kann derselbe Brückentest eine kleine vorbereitete Ausgangslage oder eine gespeicherte Partie verwenden.

Alle Bausteine leben zunächst in einem Prozess. Querverweise verwenden dauerhafte IDs. Indizes für das Routing und die Ansicht lassen sich aus der Welt aufbauen. Eine Ansicht kann ausfallen, neu verbinden oder mit anderer Bildrate laufen, ohne dass sich ein Simulationsergebnis ändert.

## 3. Welt, Zeit und öffentlicher Vertrag

Eine Straße hat einen Verlauf im Stadtraster und einen Querschnitt. Ein gerichteter **Spurabschnitt** reicht bis zur nächsten betrieblich relevanten Stelle, etwa einer Verzweigung, Haltelinie, Haltestelle oder Einfädelung. Er hat Länge, freie Fahrzeit, nutzbare Länge für Fahrzeuge, zulässige Fahrzeugarten und Verbindungen zu Folgeabschnitten. Zusammenhängende Abschnitte können unterschiedliche Fahrstreifen haben. Gehwege verbinden Gebäudeeingänge, Querungen und Haltepositionen. Gleise können eigenen Raum belegen oder dieselbe Warteschlange wie der Straßenverkehr nutzen. Grundstücke und Gebäude behalten ihre ID, auch wenn ein Umbau sie versetzt.

Die öffentliche API lässt sich ohne Dateien und Fenster verwenden. Die folgenden Namen bezeichnen Verträge, die konkrete Rust-Signatur bleibt der Implementierung überlassen.

| Operation | Vertrag |
| --- | --- |
| `new_game(scenario, rules, seed)` | Prüft Definition und Topologie. Liefert eine spielbare Welt oder lokalisierbare Fehler mit Objekt-ID und Feld. |
| `apply(command)` | Prüft einen Befehl am abgeschlossenen Zeitschritt. Erfolg liefert Sequenznummer, betroffene IDs und Folgen. Bei Ablehnung bleiben Welt, Uhr und letztes Undo erhalten. |
| `advance(ticks)` | Berechnet ganze Zeitschritte bis zur gewünschten Grenze. Gesamtpause hält die Uhr an. Ein expliziter Einzelschritt führt im pausierten Zustand genau einen Tick aus. |
| `inspect(query)` | Liest den zuletzt abgeschlossenen Zustand, etwa Reisekosten, Spurbelegung, Haltestellenrückstand, Linienpünktlichkeit oder die Entscheidung für ein Baugrundstück. |
| `save() / load(bytes)` | Liefert einen konsistenten Stand oder eine vollständig geprüfte neue Welt. Ein Ladefehler enthält einen Fehlergrund und lässt eine laufende Partie benutzbar. |

**Vorgeschlagener Zeittakt** ist eine simulierte Verkehrssekunde. Die Oberfläche bestimmt, wie häufig sie ganze Ticks anfordert. Für Tests und Regelpakete sind Startzeit, Tagesprofile und Kalendertempo definiert. Als erster Spielwert gilt ein vollständiger Verkehrstag pro Entwicklungsmonat, zwölf Verkehrstage pro Spieljahr. Die Uhr für Fahrplan und Verkehr durchläuft dabei alle 24 Stunden. Das Spieljahr taktet Bau und Verdichtung. Diese Verdichtung der Kalenderzeit lässt sich im Balancing anhand der ersten Stadt ändern.

Alle Befehle einer Tickgrenze werden in Eingangsreihenfolge angewandt. Jeder Tick durchläuft dieselben vier Phasen.

1. Uhr, Signalphasen, Fahrplan und fällige Nachfrage aktualisieren. Fertige Fußetappen und Ankünfte an Haltestellen verfügbar machen.
2. An Haltepositionen aussteigen, umsteigen, einsteigen und Haltezeit aktualisieren. Abholungen disponieren. Bewegungswünsche aus dem so entstandenen Zustand ermitteln.
3. An Knoten und Halten um Platz konkurrierende Wünsche auflösen, Plätze reservieren und genehmigte Bewegungen gemeinsam übernehmen. Ein Fahrzeug oder eine Teilmenge übertritt höchstens eine Abschnittsgrenze pro Tick.
4. Zähler, gemessene Reisezeiten und Gründe für Wartezeiten aktualisieren. Bei fälliger Entwicklungsperiode das Wachstum auswerten, anschließend den neuen Zustand veröffentlichen.

ID und Eingabesequenz brechen Gleichstände reproduzierbar. Die Vergabe knapper Durchfahrt verwendet bei gleichrangigen Zufahrten einen rotierenden Vorrang, dessen Position Teil des Zustands ist. Ganze Ticks, ganzzahlige Mengen, feste Einheiten für Länge und Kosten, explizite Rundung sowie ein versionierter Zufallsgenerator mit getrennten Zuständen für Nachfrage und Wachstum sichern reproduzierbare Läufe. Weder Iterationsreihenfolge einer Hashtabelle noch Zahl oder Häufigkeit der Abfragen dürfen eine Entscheidung verändern.

## 4. Wie ein Verkehrsfluss entsteht

Bewohner und Arbeitsplätze werden als Anzahlen an Gebäuden geführt. Gebäude werden für die Reiseplanung zu kleinen Verkehrszonen mit definierten Netzzugängen zusammengefasst. Ein Nachfrageprofil erzeugt pro Zeitfenster Wege zwischen diesen Zonen, beispielsweise morgens vom Wohnviertel zum Arbeitsplatz und abends zurück. Wege zu Geschäften erhalten eigene Ziele und Zeitprofile. Bruchteile aus Raten bleiben als Rest bis zum nächsten Zeitfenster erhalten. Die Bilanz zählt erzeugte Reisen getrennt vom Einwohnerbestand. Eine Person kann über den Tag mehrere Wege verursachen.

Eine **Reisemenge** enthält Anzahl, Herkunft, Ziel, zeitliches Abfahrtsfenster, Wegezweck, geplante Etappen und bisherigen Reisefortschritt. Mengen mit unterschiedlicher Wartedauer oder weiterem Reiseweg bleiben unterscheidbar. Teilung erhält die ganzzahlige Summe. Der Kern führt ausschließlich benachbarte, für Reihenfolge, künftige Entscheidungen und Messwerte gleichwertige Mengen zusammen. Damit entsteht bei jedem Zeitschritt weder pro Einwohner noch pro dargestelltem Auto ein dauerhaftes Objekt.

**Neue Stadtteile erzeugen selbst neue Beziehungen.** Das Szenario liefert Nutzungen, Kapazitäten und Tagesprofile. Am Beginn jedes Verkehrstages bildet der Kern aus den bezogenen Wohnungen und aktiven Zielkapazitäten eine Quell-Ziel-Matrix. Ein festes Raster verleiht auch neu erschlossenen Zonen stabile IDs. Pro Wegezweck erzeugt eine belegte Wohnzone eine Menge aus Einwohnerzahl und Tagesprofil. Für Arbeitswege erhält jede erreichbare Arbeitszone einen Anteil nach freien Arbeitsplätzen und Reiseaufwand. Die Zuteilung berücksichtigt die restlichen Plätze, arbeitet Wohnzonen in einer täglich rotierenden, deterministischen Reihenfolge ab und führt unversorgte Nachfrage ausdrücklich. Die zugeteilte Tagesmatrix liefert auch die umgekehrte Beziehung für Heimwege. Rückfahrten werden nur für tatsächlich angekommene Mengen freigegeben. Für Einkaufswege gewichtet der Kern erreichbare Geschäftsflächen und ihre Attraktivität. Neu bezogene Gebäude wirken ab dem nächsten Tagesprofil. Matrix, bereits erzeugte Wege, Restmengen und ausstehende Rückfahrten werden gespeichert. Tests können für einen Vorbereitungstag eine kleine feste Matrix vorgeben.

Der Planer bewertet für dieselbe Beziehung Fußweg, Auto, Taxi und erreichbare Bus- oder Tramverbindungen einschließlich Zu- und Abgang, Wartezeit, Fahrt und Umstiegen. Als Vorschlag für den Start verteilt er Mengen auf erreichbare Alternativen nach festen, mit steigenden Reisekosten fallenden Gewichten aus dem Regelpaket. Ganze Wege werden im Verhältnis dieser Gewichte zugeteilt, Rundungsreste in stabiler Reihenfolge verteilt. Die Auskunft zeigt Alternativen, Kostenbestandteile, Mengen und gegebenenfalls den Grund der Unerreichbarkeit. Gesperrte oder unverbundene Ziele behalten einen sichtbaren Rückstand. Nach einer Netzänderung oder zum nächsten Planungsfenster wird die Erreichbarkeit erneut geprüft.

Für neue Reisen werden die Kosten je Planungsfenster aus den Fahrplänen und den abgeschlossenen Messfenstern berechnet. Unbefahrene Abschnitte starten mit ihrer freien Fahrzeit. Eine unterwegs befindliche Menge hält ihre gültige Route bis zu einem geplanten Umstieg oder einer notwendigen Umplanung. Neu erzeugte Nachfrage auf einer vollen Zufahrt wartet am Quellzugang. Diese Wartezeit zählt zur Gesamtreise.

Fußetappen besitzen Laufzeit und Durchlass entsprechend der Gehwegbreite. Querungen können eine Signalfreigabe benötigen. Ein Fußgängerstrom verlässt eine Querung erst, wenn Platz auf dem nächsten Abschnitt verfügbar ist. Für den ersten Regelstand entspricht jede privat motorisierte Reise einer Person und einem Fahrzeug. Der Zähler für transportierte Personen bleibt fachlich von benötigter Spurlänge und Fahrzeuganzahl getrennt. Taxis belegen mit und ohne Fahrgast Platz auf der Straße.

## 5. Wie eine Spur entscheidet

Jeder Eintrag enthält entweder eine private Fahrzeugmenge mit gleicher nächster Bewegung oder die ID eines einzelnen Einsatzfahrzeugs. Dazu kommen frühester Austrittstick und belegte Länge. Freie Fahrzeit muss auch bei leerer Straße verstreichen. Am Ende des Abschnitts entscheidet der vorderste bereite Eintrag. Fehlt Grün, Vorfahrt, Abflusskapazität oder Platz im Zielabschnitt, bleiben die dahinterliegenden Einträge in Reihenfolge stehen. Nur der vorderste passende Anteil einer Menge rückt nach. Eine Buseinheit ist unteilbar.

Eine Bewegung braucht gleichzeitig Freigabe der Kreuzung, ein freies Konfliktfeld und ausreichend zusammenhängenden Zielplatz im vereinfachten Längenmodell. Der Regelstand beschreibt konfliktfreie Ampelgruppen, Räumzeiten, Vorfahrt und Abfluss pro Zeit. Mindestabstände und Restguthaben rechnen unterschiedliche Fahrzeuglängen und Flussraten in ganzzahlige Übertritte um. Eine längere Rotphase kann keinen beliebig großen Durchsatz für das nächste Grün ansparen. Neue Kreuzungen erhalten einen gültigen Grundplan. Ein widersprüchlicher Signalplan wird schon bei der Befehlsprüfung abgewiesen.

Die Spurwahl geschieht am Eintritt in einen Abschnitt. Eine kurze Linksabbiegespur beginnt erst an ihrer tatsächlichen Verzweigung. Ist sie voll, kann der nächste Linksabbieger auf der gemeinsamen Zufahrt auch einen geradeaus fahrenden Bus aufhalten. Beim erstmaligen Anbau einer Abbiegespur ordnet ein definierter Umbau die bereits im betroffenen Abschnitt stehenden Einträge anhand ihrer nächsten Bewegung den neuen Spuren zu. Nur Einträge innerhalb der neuen Abbiegetasche sind dafür berechtigt. Ihre Position ergibt sich aus Abschnittslänge, Eintrittstick, freier Fahrzeit und den davor belegten Längen. Innerhalb jedes Zielstreifens bleibt die Reihenfolge erhalten. So hat die Länge der Tasche eine direkt beobachtbare Spielwirkung.

## 6. Linienbetrieb, Haltestellen und Taxis

Ein eingesetztes Fahrzeug hat Typ, stabile ID, Länge, Kapazität, aktuellen Abschnitt, Zeitfortschritt, Betriebszustand und Belegung. Busse und Straßenbahnen besitzen außerdem Linie, Richtung, nächsten Halt, Umlauf und geplante Abfahrt. Ein Einsatz erzeugt genau ein Fahrzeug an einem gültigen Startpunkt. Bei belegtem Startplatz wartet es auf die Einfahrt. Taktvorgaben erzeugen Abfahrtsaufträge. Verfügbare eingesetzte Fahrzeuge übernehmen sie der Reihe nach. Bei zu kleiner Flotte weist der Kern verspätete oder ausgefallene Fahrten aus. Ein Taktwechsel erzeugt keine zusätzlichen Fahrzeuge.

Haltepositionen haben endliche Kapazität und sind als Halt auf der Spur oder in einer Bucht beschrieben. Die Straßenbahn bedient nur über passende Gleisverbindungen erreichbare Halte. Ausstieg gibt Plätze frei, danach steigen passende Reisemengen nach Ankunftsreihenfolge bis zur Restkapazität ein. Ein Rest wartet mit seinem ursprünglichen Ankunftszeitpunkt. Die Haltezeit ergibt sich aus Mindesthalt und Fahrgastwechsel. Ein Umstieg braucht die tatsächliche Gehzeit zwischen den Haltepositionen. Bei einem Halt auf der Fahrspur steht der nachfolgende Verkehr während des Halts mit an.

Ein Taxi durchläuft **frei → zur Abholung → mit Fahrgast zum Ziel → frei**. Taxinachfrage entsteht aus der Verkehrsmittelwahl. Der erste Regelstand bündelt am Abholort wartende Wünsche mit gleichem Ziel bis zur Fahrzeugkapazität und vergibt den ältesten bedienbaren Wunsch an das freie Taxi mit der kürzesten erreichbaren Anfahrt. Bei Gleichstand entscheidet die ID. Eine Reservierung schließt doppelte Bedienung aus. Auch die Leerfahrt verbraucht Fahrzeit und Stauraum. Auskünfte unterscheiden Warten auf Zuteilung, Anfahrt und besetzte Fahrt. Weitere eingesetzte Fahrzeugtypen können dieselben Verträge für Identität, Netzberechtigung und individuellen Auftrag verwenden.

## 7. Umbauen, wachsen und wiedererkennen

Ein Baubefehl führt zuerst eine vollständige Vorschau auf Geometrie, Netz, Gebäudezugänge, Fahrzeuge und betroffene Reisen aus. Sie prüft unter anderem verfügbaren Raum, zulässige Fahrtrichtungen, erreichbare Gleise und konfliktfreie Signale. Für vorhandene Belegung erstellt sie eine Zuordnung auf die neue Topologie. Einträge dürfen nur auf physisch anschließende, für ihren Typ nutzbare Abschnitte wechseln. Bei fehlendem Platz oder fehlender sicherer Position wird der gesamte Befehl mit Ursache und betroffener ID abgewiesen. Freie Abschnitte können auch dann getrennt werden, wenn dadurch ein Ziel unerreichbar wird. Vorschau und Bestätigung benutzen denselben Prüfer und dieselbe Netzrevision.

Nach einem gültigen Umbau werden betroffene Wege ab dem aktuellen Ort neu geplant. Eine unterbrochene Linie oder ein abgeschnittenes Taxi geht mit belegten Plätzen und Auftrag in einen sichtbaren Wartezustand. Fahrgäste können an einem erreichbaren, geeigneten Halt aussteigen und neu planen. Verliert eine private Automenge den Weg zum Ziel, sucht der Kern vom aktuellen Spurabschnitt aus den nächstgelegenen erreichbaren Ausstiegspunkt mit Fußnetzzugang (nach Fahrzeit, bei Gleichstand nach ID). Gebäudezufahrten und ausdrücklich definierte Parkzugänge können als Ausstieg dienen. Die Autos fahren regulär bis dorthin und verlassen erst dort die Spur. Die Reisemenge plant den Restweg zu Fuß oder mit öffentlichem Verkehr neu. Ist der Restweg unmöglich, wartet sie abseits der Straße an diesem Zugang. Gibt es keinen erreichbaren Ausstieg, bleiben Autos mit Wartegrund auf ihrer Spur und können einen Rückstau auslösen. Die Vorschau weist auf betroffene Mengen hin. Wiederhergestellte Verbindungen lösen die erneute Prüfung aus. Gebäude können innerhalb der Bauregeln versetzt werden und behalten Nutzung, Bevölkerung und Zugang. Passt ein Grundstück weder an Ort und Stelle noch auf eine zulässige Nachbarfläche, erklärt die Vorschau die Ablehnung. Begrünung und breitere Gehsteige belegen Querschnittsfläche. Ein breiterer Gehweg verändert dessen Durchlass.

**Rückgängig mit Zeitrücksprung.** Vor jedem erfolgreichen Spielerumbau merkt sich der Kern einen vollständigen Wiederherstellungspunkt. „Letzten Umbau zurücknehmen“ stellt auch die damalige Spielzeit wieder her. So lässt sich selbst nach mehreren Umläufen der tatsächliche Ausgangszustand für einen Vergleich erreichen. Der Befehl liefert die Zielzeit vorab für die Anzeige. Der Wiederherstellungspunkt überlebt Speichern und Laden und enthält selbst keine Undo-Historie. Ein fehlgeschlagener Bau, eine Abfrage oder eine Pause überschreibt ihn nicht. Markierungen sind separat editierbare Weltkoordinaten mit optionaler Objekt-ID, bei entfallenem Objekt bleibt die Ortsmarke sichtbar.

Das Wachstum wertet in festen Entwicklungsperioden tatsächlich erreichbare Arbeitsplätze, Wohnungen und Geschäfte aus. Dabei zählen Wegezeit und gemessene Bedienqualität einschließlich Wartezeiten. Kandidaten sind freie geeignete Parzellen, verdichtbare Gebäude und durch eine kurze zulässige lokale Straße erschließbare Flächen. Bedarf je Nutzung, Bauland, Zugänglichkeit und Regeln aus dem Szenario bestimmen Rang und Bauvolumen. Die Entwicklung respektiert Fluss, Brücken und Spielerinfrastruktur. Bauvorhaben belegen ihre Fläche, durchlaufen eine Bauzeit und aktivieren Bevölkerung oder Zielkapazität erst bei Fertigstellung. Bis 10.000 Einwohner darf die Stadt wachsen. Dann ist die Wohnkapazitätsgrenze des ersten Regelstands erreicht. Die Wachstumspause friert Entwicklungsuhr, Baufortschritt und wachstumswirksame Messfenster ein. Beim Fortsetzen werden pausierte Perioden nicht nachgeholt.

## 8. Was die Oberfläche und die Tests erfahren

Die Ansicht erhält zeitgestempelte Ausschnitte des Netzes mit Dichte, Flussrichtung, Durchsatz, geschätzter Staulänge und Wartegrund. Damit kann sie repräsentative Autos und Personen zeichnen und zwischen Ticks animieren. Die Zahl sichtbarer Figuren darf von Zoom und Grafikleistung abhängen. Einzelne Einsatzfahrzeuge liefert der Kern mit ID, Abschnitt, Fortschritt, Haltezustand und Belegung. Ein privates Reisesymbol kann auf seine Quell-Ziel-Menge verweisen. Eine individuelle Biografie wäre aus diesen Daten nicht ableitbar.

| Spielentscheidung | Rückmeldung aus dem Kern | Längerfristig sichtbare Folge |
| --- | --- | --- |
| Abbiegespur vor der Brücke ergänzen | Wer den Bus aufhält, wie viele Fahrzeuge Platz finden, bei welchem Grün der Bus passiert | Reisezeit und Zuverlässigkeit der Linie über mehrere Umläufe |
| Takt oder Umstieg ändern | Ankunft, Fußweg, tatsächliche Abfahrt, wartende und zurückbleibende Menge | Gesamtreisezeit und Verkehrsmittelanteile auf derselben Quell-Ziel-Beziehung |
| Eine Linie bis zur Wiese führen | Von dort erreichbare Ziele, Fahrzeit und Auslastung | Bauentscheidung samt Gründen, Baufortschritt und spätere Belegung |
| Fahrspur in Gehsteig umwandeln | Platzverteilung, Fußdurchsatz, Autostau und unerfüllte Wege | Veränderung beider Verkehrsmengen und der Erreichbarkeit |

Die kurzen Rückmeldungen unterstützen den im Game-Design vorgesehenen Wechsel aus Beobachten, Eingreifen und erneutem Beobachten. Ein Stau darf bestehen bleiben. Marker und vergleichbare Ausgangsstände unterstützen eigene Experimente. Diagnoseereignisse wie „Bus passiert Haltelinie“, „vier warten auf nächste Fahrt“ oder „Bauplatz wegen Erreichbarkeit gewählt“ tragen Tick, beteiligte IDs, Menge und einen strukturierten Grund. Zeitreihen haben begrenzte Fenster. Entscheidungsrelevante Messwerte gehören zum Spielstand. Rein für die Anzeige erzeugte Detailmeldungen dürfen verfallen.

## 9. Laden und Speichern

**Für Version 1 schlage ich ein eigenständiges JSON-Dokument vor.** Es enthält stabile IDs, explizite Einheiten, eine Formatversion und eine Prüfsumme über die kanonische Nutzlast. Es lässt sich mit Testwerkzeugen und im Fehlerfall direkt untersuchen. Interne Speicherlayouts und Engine-Objekte sind kein Dateivertrag. Das Szenario samt effektivem Regelpaket ist im Stand enthalten. Eine später geänderte Szenariodatei darf einen gespeicherten Lauf nicht verändern.

| Zustand im Spielstand | Beispiele |
| --- | --- |
| Herkunft und Regeln | Formatversion, Simulationsregelversion, Szenario-ID, effektive Parameter, Seed und laufende Zustände der Zufallsgeneratoren |
| Welt und Steuerung | Gelände, Gebäude, Bauvorhaben, Netze, nächste IDs, Revisionen, Verkehrs- und Entwicklungsuhr, Pausen, Befehlssequenz, Markierungen und ein optionaler Undo-Punkt |
| Laufender Verkehr | Spurreihenfolgen und Teilmengen, belegte Längen, Austrittszeiten, Kapazitätsreste, Signal- und Räumzeiten, Vorrangpositionen, Fußetappen und Haltewarteschlangen |
| Laufender Betrieb | Fahrzeug-IDs und Standorte, Umläufe, Haltezeiten, Belegung mit Folgeetappen, Aufträge, Reservierungen und verpasste Abfahrten |
| Nachfrage und Entwicklung | Mengenreste, unerfüllte Wege, aktuelle Routen und Planungsfenster, Reisezeitmittel, Wachstumsbewertungen und angebrochene Entwicklungsperioden |

Speichern erfasst einen abgeschlossenen Tick oder den Zustand unmittelbar nach einem vollständig angewandten Befehl. Der Dateiadapter schreibt in eine temporäre Datei im selben Verzeichnis, stellt deren vollständige Speicherung sicher und ersetzt die Zieldatei atomar. Wo die Plattform es verlangt, synchronisiert er auch das Verzeichnis. Ein Fehler vor dem Austausch erhält den alten Stand. Ein Fehler nach dem Austausch weist die Haltbarkeit als unbestätigt aus. Pro Zielpfad wird höchstens ein Schreibvorgang gleichzeitig ausgeführt.

Laden liest zuerst Größenobergrenzen, Version und Prüfsumme, migriert bekannte ältere Formate in einer separaten Datenstruktur und prüft IDs, Verweise, Reihenfolgen, Zeitgrenzen, Kapazitäten und Mengenbilanzen vor der Übernahme. Auch der Undo-Punkt wird geprüft. Unbekannte neuere Formate und inkompatible Simulationsregeln ergeben unterscheidbare Fehler. Jede künftig unterstützte Migration braucht eine alte Beispieldatei mit erwartetem Fortsetzungsverhalten. Version 1 begründet die erste solche Referenz. Routensuchindizes und Ansichtsgeometrie werden neu aufgebaut. Ein Cache, dessen Inhalt eine spätere Entscheidung ändern könnte, muss entweder gespeichert oder schon im laufenden Spiel rein aus dem gespeicherten Zustand berechnet werden.

## 10. Abnahme ohne Oberfläche

Der Headless-Runner lädt Szenario oder Stand, wendet eine Befehlsfolge an definierten Tickgrenzen an und gibt Abfrageergebnisse, Messreihen und einen kanonischen Zustandsfingerabdruck aus. Szenarien, Befehle und öffentliche Abfragen bilden dieselbe Grenze, die später die Oberfläche nutzt. Kleine Modultests ergänzen diese fachlichen Prüfungen.

| Abnahmefall | Erwartung |
| --- | --- |
| **Brücke bei Rot** | Drei private Linksabbieger stehen vor Bus 12, fünf Geradeausfahrer dahinter. Bei gesperrtem Linksabbiegen wartet der Bus trotz Geradeausgrün. Mit passender, erreichbarer Abbiegetasche passiert er beim nächsten Geradeausgrün. Eine zu kurze Tasche lässt einen Rest auf der gemeinsamen Spur. |
| **Rückstau und Konkurrenz** | Bei vollem Zielabschnitt bleibt Verkehr vor der Kreuzung. Nach Freigabe fahren bereite Einträge in Reihenfolge. Zwei dauerhaft berechtigte gleichrangige Zufahrten kommen beide zum Zug. Eine Straßenbahn auf eigenem Gleis folgt ihren eigenen Freigaben. |
| **Anschluss und Kapazität** | Zwölf warten mit gleichem Ziel, acht Plätze sind frei. Acht fahren mit, vier behalten ihre Wartezeit. Ein geänderter Takt ermöglicht im vorbereiteten Szenario den Anschluss und verkürzt die gemessene Gesamtreise. |
| **Taxi** | Zwei mögliche Taxis und ein Auftrag ergeben genau eine Reservierung. Save/Load während der Leerfahrt führt zur gleichen Abholung und Ankunft. Stau auf der Anfahrt verlängert die Wartezeit. |
| **Umbau und Undo** | Belegter, sicher umlegbarer Abschnitt wird mitsamt Belegung umgebaut. Ein Umbau ohne Platz wird vollständig abgewiesen. Undo nach mehreren Ticks und einem Ladevorgang stellt den Zustand vor dem Bau einschließlich seiner Uhr wieder her, der verbrauchte Undo-Punkt ist leer. |
| **Abgetrenntes Reiseziel** | Ein privater Strom verliert seine Route. Mit erreichbarem Ausstieg bleibt die Menge bis zur Zufahrt im Straßenverkehr und setzt danach ihren Weg fort oder wartet am Zugang. Ohne erreichbaren Ausstieg bleibt die Spur belegt. Speichern und Fortsetzen erhält beide Fälle. |
| **Wachstum** | Zwei Läufe beginnen auf derselben Karte. Das Szenario gibt einer freien Wiese erschließbare Baufläche und Nachfrage. Eine vorteilhafte Linie verbessert die gemessene Erreichbarkeit und führt dort im vorbereiteten Regelstand früher zu Bebauung. Nach Bezug entsteht am neuen Standort beim nächsten Tagesprofil ein messbarer ausgehender Arbeitsweg. Im pausierten Vergleich bleibt die Bebauung konstant, Busse fahren weiter. |
| **Fortsetzung** | Lauf A läuft durch. Lauf B speichert bei Rot, mitten im Halt, während eines Umstiegs oder während eines Bauvorhabens und lädt jeweils in einem neuen Prozess. Identische weitere Befehle ergeben an denselben Tickgrenzen identische fachliche Zustände, Zähler und künftige Ereignisse. |
| **Fehlerfälle** | Abgeschnittene Datei, falsche Prüfsumme, ungültige Referenz, negative Menge und unbekannte Version liefern gezielte Fehler. Simulierter Schreibfehler vor Austausch erhält den letzten ladbaren Stand. |
| **Last und Langzeitlauf** | Eine feste Stadt mit 10.000 Einwohnern, Berufsverkehr, Linien, Taxis und blockierter Brücke läuft einen vollständigen Verkehrstag. Ein zweiter Lauf über fünf Spieljahre erreicht seine Prüfpunkte auch nach Neustarts. Berichtet werden Ticks/s, Spitzenbedarf an Speicher, Anzahl aktiver Mengen, Dateigröße sowie Lade- und Speicherzeit. |

Über alle Fälle gelten Erhaltungsregeln. Erzeugte Reisen sind zu jedem Zeitpunkt in genau einem Zustand (wartend, unterwegs oder angekommen). Reservierte Taxiwünsche zählen weiterhin als wartend. Umstiege erhöhen die Zahl erzeugter Reisen nicht. Fahrzeuge haben genau einen Betriebsort, Belegung und Spurlänge bleiben innerhalb ihrer Grenzen. Zufällig erzeugte gültige Befehlsfolgen mit festem Seed prüfen diese Regeln auch bei Umbauten und nach dem Laden. Mehrfaches Speichern, andere Abfragehäufigkeit und unterschiedlich große Aufrufe von `advance` müssen dieselbe fachliche Entwicklung ergeben.

Als vorläufiges Leistungsziel soll der Lastfall auf einem dokumentierten Desktop-Referenzgerät mindestens 60 Verkehrsticks pro realer Sekunde im Mittel erreichen. Dann passt eine Minute Rushhour in eine reale Sekunde. Das ist ein Zielwert, kein Messergebnis. Lange Läufe laufen ohne Grafik und ohne unbegrenztes Ereignisprotokoll. Der erste Benchmark legt Gerät, Buildprofil, Szenariodatei und Seed fest. Bei Engpässen werden zunächst aktive Mengen, Routenwiederverwendung und unveränderte Netzteile untersucht. Optimierungen müssen die Fortsetzungs- und Bilanztests bestehen. Die kurzen fachlichen Fälle laufen bei jeder Änderung. Der ausgewachsene Lastfall und der mehrjährige Lauf sind eigene Abnahmen im optimierten Build. Bei dem vorläufigen Mindestdurchsatz benötigt ein Lauf über fünf Spieljahre rund 24 reale Stunden.

## 11. Reihenfolge bis zum ersten vollständigen Kern

1. **Die Brücke reproduzierbar machen.** Szenariolader, IDs, Spur- und Kreuzungsmodell, Ticks, Befehle, Diagnose und Save/Load gemeinsam durch den ersten Brückentest führen. Schon dieser Stand muss einen belegten Abschnitt exakt fortsetzen.
2. **Die ganze Reise schließen.** Quellen und Ziele, Fußwege, Verkehrsmittelwahl, Busse, Straßenbahnen, Taxis und Umstiege bis zur Ankunft und Mengenbilanz verbinden.
3. **Eingriffe im laufenden Betrieb absichern.** Alle Sandkastenwerkzeuge, Vorschau, Umplanung, beide Pausen, Markierungen und Undo am selben öffentlichen Vertrag prüfen.
4. **Mehrere Spieljahre tragen.** Erschließung, Bebauung, Verdichtung, Wachstumsdiagnose, Lastszenario und Fünfjahreslauf ergänzen. Der gesamte Katalog aus Abschnitt 10 ist die Freigabebedingung für den Kern.

Der Implementierung überlassen bleiben interne Container, Suchalgorithmus, räumlicher Index und Aufteilung der Rust-Module. Kalibrierbare Werte wie Sättigungsdurchsatz, Umsteigewiderstand, Haltezeit und Wachstumsschwellen gehören in ein versioniertes Regelpaket. Sie lassen sich anhand derselben Szenarien und späterer Spieltests anpassen. Architektur, Kalendertempo, Undo mit Zeitrücksprung und die Leistungsannahme sind Vorschläge dieses technischen Entwurfs.
