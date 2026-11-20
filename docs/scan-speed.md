# Scan-Geschwindigkeit

Gemessen am 20.11.2026 mit `force=1`, 41 Anbieter, Film 550 und Serie 1396 S1E1, gleicher PC, eigene Testinstanz auf Port 43303.
Zeiten ab dem POST. "Beste" = Zeitpunkt, zu dem die am Ende beste Quelle (höchste Auflösung, Filter "Qualität") fertig war.
RAM = Spitze von Flimmer.exe samt aller WebView2-Prozesse.

## Vorher / Nachher

| Lauf | Titel | erste Quelle | beste Quelle | ganzer Scan | ok / kein Stream / Fehler | RAM-Spitze |
|---|---|---|---|---|---|---|
| Vorher | Film | 1,5 s | 1,9 s | 142,9 s | 16 / 19 / 6 | 2,7 GB |
| Vorher | Serie | 1,9 s | 14,8 s | 127,4 s | 18 / 16 / 7 | 2,1 GB |
| Nachher 1 (kalt, leere Statistik) | Film | 1,7 s | 2,7 s | 45,1 s | 17 / 19 / 5 | 3,4 GB |
| Nachher 1 (kalt) | Serie | 1,8 s | 1,8 s | 43,9 s | 17 / 19 / 5 | 3,0 GB |
| Nachher 2 (mit Statistik) | Film | 1,1 s | 5,1 s | 42,3 s | 18 / 18 / 5 | 2,8 GB |
| Nachher 2 | Serie | 0,6 s | 2,8 s | 48,5 s | 17 / 18 / 6 | 2,6 GB |
| Nachher 3 | Film | 1,8 s | 7,3 s | 44,7 s | 17 / 18 / 6 | 2,4 GB |
| Nachher 3 | Serie | 0,7 s | 2,9 s | 44,1 s | 18 / 17 / 6 | 2,4 GB |
| Nachher 4 (tote Anbieter übersprungen) | Film | 0,8 s | 7,7 s | 20,8 s | 18 / 18 / 5 | 2,3 GB |
| Nachher 4 | Serie | 0,6 s | 2,9 s | 22,6 s | 18 / 17 / 6 | 2,7 GB |

Ziele: erste Quelle ≤ 5 s erreicht, beste Quelle ≤ 15 s erreicht. Ganzer Scan ≤ 45 s: in den ersten drei Scans knapp (42 bis 48 s), ab dem vierten Scan rund 21 bis 23 s. Der Rest dort ist primesrc, das erst nach 12 s liefert.

## Was geändert wurde

- 10 Seiten gleichzeitig statt 4, Start im Abstand von 0,3 s.
- Aufgeben nach 15 s, wenn die Seite gar keine Medienanfrage gezeigt hat (vorher immer 25 s). Mit Medienanfrage 6 s Nachlauf, höchstens 25 s. Klicks bei 7, 10 und 13 s statt 7, 12 und 18 s, Play-Versuch ab 2,5 s statt 6 s.
- Statistik je Anbieter in SQLite (`provider_health`, getrennt nach Film und Serie). Reihenfolge: zuletzt erfolgreiche zuerst (schnellste vorn), dann unbekannte, dann zuletzt erfolglose.
- Drei Scans in Folge ohne Stream: Anbieter wird übersprungen und frühestens 24 h nach dem letzten Versuch wieder getestet, am Ende der Warteschlange. Gezielter Einzel-Scan eines Anbieters geht immer.
- Zuletzt erfolgreicher Anbieter ohne Stream bekommt im selben Scan einen zweiten Versuch.
- Der Sniffer lässt nur noch mögliche Medien-URLs über den UI-Thread laufen statt jede Anfrage jeder Seite. Ein leeres verstecktes Fenster hält den Scan-Browser samt Werbeblocker geladen.

## Geprüft und verworfen

- HTTP-Vorabcheck: hat braflix nach 5 s als "timed out" aussortiert, obwohl die Seite im Fenster funktioniert. Gewinn wäre nur etwa 1 s bei den fünf Anbietern ohne DNS-Eintrag gewesen.
- 12 Seiten gleichzeitig: im kalten Serien-Scan 14 bis 17 ok gegenüber 15 bis 18 bei 10 Seiten, bei nur etwa 5 s Zeitgewinn.
- Erster Klick schon nach 3 s: viduki und tgvid starteten danach mehrfach nicht mehr.
- Zweiter Versuch erst ganz am Ende des Scans: verlängerte den Scan auf 59 s.
- Fenster-Pool: das Öffnen eines Fensters kostet rund 0,1 s, kein messbarer Gewinn.

## Verlorene Quellen

Verglichen mit "Vorher" fehlen in "Nachher 4":
- tgvid (Film und Serie) und multiembed (Serie). Beide liefern inzwischen auch allein gescannt nichts, mit dem alten und dem neuen Build (je 2 bis 3 Einzel-Scans um 06:15 und 06:45). Das liegt an den Seiten, nicht am Scan. tgvid lief um 05:50 allein noch.

Dazugekommen sind cinesrc, primesrc und braflix (Film) sowie vidcore und primesrc (Serie).
Peestream und 1embed melden jetzt "kein Stream" statt "Fehler", weil das Fenster vor dem Lade-Timeout des Browsers aufgibt.

## Offen

- Anbieter schwanken von Scan zu Scan: auch im alten Stand fehlten je Scan ein bis zwei, die sonst gehen. Ein einzelner Fehlversuch schadet nicht, erst drei in Folge führen zum Überspringen.
- Je mehr Seiten gleichzeitig laden, desto öfter startet eine sonst funktionierende Seite nicht. Die Ursache ist nicht abschließend geklärt. Der zweite Versuch fängt das für bekannte Anbieter ab, im allerersten Scan (leere Statistik) nicht.
- Der Nutzen des dauerhaft offenen leeren Fensters wurde nicht einzeln gemessen.
