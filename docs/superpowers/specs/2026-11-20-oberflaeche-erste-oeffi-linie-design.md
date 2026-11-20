# Die erste eigene Öffi-Linie

**Oberflächenentwurf zur Freigabe · 20. November 2026**

Du erkundest die Brückenstadt am PC. Von oben fällt dir auf, wie viele Autos über dieselbe Brücke fahren. Du gehst ihren Wegen nach und probierst eine eigene Öffi-Verbindung aus. Nach den ersten Umläufen kannst du am selben Ort beobachten, ob die Schlange vor der Ampel kürzer wird. Du darfst genauso gut zum Marktplatz schwenken und anderen Dingen zuschauen. Ein liegen gebliebenes Problem löst weder Frist noch Strafe aus.

Im spielbaren Einstieg fährt die rote Straßenbahn bereits durch die Stadt. Für viele Arbeitswege aus einem Wohnviertel ans andere Ufer fehlt eine passende Verbindung. Die Herkunftsansicht macht diese Lücke auffindbar, wenn du ihr nachgehst. Das bestehende Prüfszenario des Kerns betreibt bereits Busse und Straßenbahnen. Für den Einstieg ist deshalb eine eigene Angebotskonfiguration vorgesehen.

## Die Stadt lesen

Die isometrischen Konzeptzeichnungen geben Maßstab, Material und Licht vor. Fluss, Brücke und Wirtshaus bleiben beim Wechsel zwischen Viertelansicht und Nahansicht wiedererkennbar. Aus der Nähe siehst du die Menschen zur Haltestelle kommen, einsteigen und gegebenenfalls am Gehsteig zurückbleiben. Der Bus reiht sich hinter den Autos ein. Bei Grün setzt sich die Kolonne vom vorderen Ende her in Bewegung. Auch ein fast leerer Gehsteig nach der Abfahrt erzählt etwas über die Linie.

Eine große wartende Gruppe und eine lange Wartezeit sind zwei verschiedene Beobachtungen. Langes Warten wird im Verlauf sichtbar, wenn dieselben Personen nach der Abfahrt noch dort stehen und neue dazukommen. Beim Herauszoomen kannst du die Ebene **Wartende** einschalten. Sie hebt die Gruppen an ihrem jeweiligen Haltepunkt hervor. Die Größe der Gruppe bleibt lesbar, auch wenn einzelne Personen zu klein werden.

Wenn du zum ersten Mal selbst eine Straße oder eine Haltestelle untersuchst, erscheint dort einmal ein kurzer Hinweis auf die passende Kartenebene. Beim Weiterziehen verschwindet er. Die Ebene bleibt später am selben Werkzeug erreichbar. Hohe Nachfrage allein öffnet keine Hinweise. Die Kamera folgt deiner Eingabe.

## Eine Idee für die Linie finden

Wählst du an der Brücke **Wege anzeigen**, legt sich die Herkunft der dortigen Autofahrten über die Stadt. Häuserblöcke mit vielen Fahrtantritten heben sich stärker ab. Wählst du einen Block, kannst du die dazugehörigen Wege über die Straßen bis zu ihren Zielen verfolgen. Die Ebene bezieht sich auf den gewählten Straßenabschnitt und macht zusammengehörige Start- und Zielorte erkennbar. Als erster Entwurfswert dient der aktuelle Morgenverkehr. Die normalen Stadtfarben kehren beim Ausschalten zurück.

Beim Verschieben einer probeweisen Haltestelle zeigen begehbare Wege und hervorgehobene Häuser, wen der Halt erschließt. Eine fehlende Querung oder der Fluss darf das Einzugsgebiet sichtbar begrenzen. Sobald ein zweiter Halt liegt, werden die bisherigen Autofahrten erkennbar, für die sowohl Einstieg als auch Ausstieg zu Fuß erreichbar wären. Ob die Menschen tatsächlich umsteigen, erfährst du im Betrieb. Balken, Ranglisten und eine automatisch empfohlene Siegerroute sind für diesen Ablauf entbehrlich.

## Mit dem Werkzeug bauen

Für den Baubeginn standen zwei Ansätze zur Wahl. Eine Aktion an der untersuchten Straßenecke hätte den ersten Halt gleich dort angelegt. **Gewählt ist ein festes Werkzeug am Kartenrand.** Du kannst es unabhängig von der aktuellen Auswahl aufrufen und den ersten Halt dort ausprobieren, wo du ihn sinnvoll findest.

1. Du wählst **Linie bauen** und entscheidest dich im selben Werkzeug für Bus oder Straßenbahn. Eine Haltestellenvorschau folgt dem Mauszeiger an geeigneten Straßenabschnitten oder Gleisen. Ein Klick setzt den ersten Halt im Entwurf. Jeder weitere Halt folgt in der angeklickten Reihenfolge.
2. Die Stadt sucht einen für das gewählte Verkehrsmittel befahrbaren Weg. Der Verlauf ist direkt auf der Straße oder dem Gleis zu sehen. Mein Vorschlag für Korrekturen ist, die angezeigte Strecke auf eine andere geeignete Straße oder Gleisverbindung zu ziehen. So entsteht ein Wegpunkt für die Durchfahrt. Eine unbefahrbare Verbindung bleibt im Entwurf an der betroffenen Stelle erkennbar und lässt sich korrigieren.
3. Du stellst ein, **wie oft die Linie fährt**. Der Betrieb organisiert die dafür nötigen Fahrzeuge. Der Takt ist eine Vorgabe für den Fahrplan. Stau kann dazu führen, dass die Fahrzeuge tatsächlich in einem anderen Abstand ankommen.
4. Erst mit **Linie starten** übernimmt der Kern die Änderungen zusammen. Bis dahin kannst du Halte verschieben oder den Entwurf verwerfen. Auf der laufenden Linie kannst du Fahrweg und Takt später ändern.

Bus und Straßenbahn erfüllen dieselbe Aufgabe. Als Ausgestaltung der höherwertigen Straßenbahn schlage ich mehr Platz je Fahrzeug vor. Beim gleichen Takt kann so eine größere Gruppe auf einmal mitfahren. Der Bus erreicht auch Straßen ohne Gleis. Ein Umstieg auf die Straßenbahn bleibt eine freiwillige Entscheidung, wenn die Verbindung und die Nachfrage dafür sprechen.

## Was der Kern dafür liefern muss

Die Oberfläche liest den abgeschlossenen Simulationszustand und übersetzt ihn in sichtbare Bewegung. Spurbelegung, Fahrzeugfortschritt, wartende Mengen und Linienfahrzeuge sind bereits beobachtbar. Für die Herkunftsansicht braucht es eine räumliche Zuordnung der tatsächlich über den gewählten Abschnitt geführten privaten Wege. Wo der Kern nur eine Nachfragezone kennt, darf die Darstellung keine einzelnen Herkunftshäuser vortäuschen.

Für die Umsetzung sind außerdem Haltepositionen mit passendem Fußzugang, die Vorschau beider Reiseenden, verbindliche Wegpunkte und die automatische Fahrzeugzuteilung zu berücksichtigen. Der gezeigte Linienweg muss mit dem später gefahrenen Weg übereinstimmen. Die öffentliche Linienbeschreibung des Kerns führt aktuell geordnete Halte einer Richtung. Als Entwurfsvorgabe für eine gewöhnliche Linie gilt ein regelmäßiger Hin- und Rückbetrieb mit jeweils zulässigem Straßen- oder Gleisverlauf.

## Woran der erste Ausschnitt gemessen wird

- Wer nur zuschaut, kann wartende Menschen und stockenden Verkehr entdecken und die Kamera jederzeit weiterbewegen.
- Wer der Herkunft der Autos nachgeht, kann auf der Karte zwei passende Orte finden und einen Halteentwurf entlang der Straße vergleichen.
- Bus und Straßenbahn lassen sich mit demselben Werkzeug planen. Bei der Straßenbahn folgt die Vorschau dem verbundenen Gleisnetz.
- Wer die Linie startet, kann Fahrgästen beim Einsteigen zusehen und das Fahrzeug auf seinem gewählten Weg verfolgen. Die Stadt vermittelt die Wirkung auch bei geschlossenen Kartenebenen.

Die Browserbilder und Animationen aus dem Gespräch sind illustrative Skizzen mit Beispieldaten. Für die Ausarbeitung werden die räumlichen und betrieblichen Informationen des Rust-Kerns verwendet. Enginewahl, genaue Mausbelegung und grafische Ausgestaltung bleiben der technischen und gestalterischen Umsetzung überlassen.
