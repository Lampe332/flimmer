# Usability-Durchgang (20.11.2026)

Vorher → nachher. Design unverändert, nur Bedienung, Texte und Tempo.

## Start und Navigation
- Abspielen ging nur über die Titelseite (2 Klicks) → runder Play-Knopf auf jeder Karte beim Drüberfahren (Maus/Tastatur), 1 Klick. Auf Touch öffnet die Karte weiter die Titelseite.
- „Abspielen“ bei Serien (Hero, Karten) startete immer S1F1 → `/play/tv/:id` ohne Folge springt dorthin, wo du aufgehört hast.
- Titelseite lud erst nach dem Klick → Daten werden beim Drüberfahren/Fokus vorgeladen.
- „Zurück“ landete immer oben → Scroll-Position je Verlaufseintrag, Zurück/Vor stellt sie wieder her. Staffel-Tabs und Tippen in der Suche springen nicht mehr nach oben.
- Zurück-Pfeil im Player ging immer zur Titelseite → geht zur Seite, von der du kamst (Fallback Titelseite).

## Warten auf die Quelle
- Text „startet, sobald alle durch sind“ war falsch (Start nach 8 s) und zeigte „12 von 41 Anbietern“ → „Suche die beste Quelle …“, „Quelle gefunden, es geht in 5 s los“, ruhiger Fortschrittsbalken, Knopf „Sofort starten“. Nach 8 s ohne Treffer: „Suche läuft noch, kann bis zu 2 Minuten dauern“.
- Quellenliste zeigte 41 Zeilen samt Fehlern → nur spielbare Quellen, darunter „N Anbieter werden noch gefragt“ und eingeklappt „N Anbieter ohne Stream“.
- Filter ohne Erklärung, Fachwörter (Bitrate, Tempo, UT, ms) → eingeklappt als „Automatische Auswahl: jede Qualität · Deutsch · Schärfe“, Felder „Mindestqualität“, „Tonspur“, „Bei Gleichstand zählt: Schärfe / Datenrate / Schneller Start“ mit Satz-Erklärung. Datenrate und Antwortzeit stehen nur noch im Tooltip.

## Player
- Einfachklick pausierte auch beim Doppelklick (Vollbild) → Klick wartet 240 ms, Doppelklick = nur Vollbild.
- Keine Übersicht der Tasten → `?` oder ?-Knopf öffnet „Tastenkürzel“ (auch im Vollbild). Neu: `C` Untertitel an/aus, `N` nächste Folge, Tooltips mit Taste.
- Nach Klick auf den Lautstärkeregler gingen keine Tasten mehr (F, M, Leertaste) → nur Textfelder schlucken Kürzel.
- Menüs nur per Maus → öffnen mit Fokus auf der aktuellen Wahl, Pfeiltasten/Home/End, Esc gibt den Fokus zurück.
- Menü zeigte optimistisch, nicht was hls.js spielt → Ton-/Untertitelmenü spiegelt `hls.audioTrack`/`subtitleTrack` (auch als `data-audio-track`/`data-subtitle-track` am Player).
- Spurnamen englisch („Italian [Forced]“) → „Italienisch (nur Fremdsprache)“, „Englisch (mit Geräuschen)“.
- Fehler im Video: nur „wähle rechts eine andere Quelle“ → Knopf „Andere Quelle“ (nimmt die nächstbeste) bzw. „Quellen neu suchen“.
- Auto-Next-Karte bei 390 px: Bild + Textspalte quetschten sich → kompakte Leiste über den Steuerelementen, Abbrechen als ×.

## Fortschritt beim Schließen des Fensters
- Vorher: WebView2 feuert beim Schließen kein `pagehide`; gemessen ging die letzte Position bis zu 10 s verloren (letzter Intervall-Save).
- Nachher: Die Desktop-App fängt `CloseRequested` ab, schickt `flimmer:beforeclose` an die Seite (Player speichert sofort) und schließt 400 ms später. Zweiter Klick auf X schließt sofort. Im Browser gilt weiter: Speichern alle 10 s, bei Pause, Spulen, Ende, `pagehide`.

## Offen
- Fehler-Knopf „Andere Quelle“ nur im Code geprüft (ein echter Abbruch ließ sich nicht erzwingen).
- Vollbild verweigert WebView2, solange das Fenster minimiert ist (Windows-Verhalten).
