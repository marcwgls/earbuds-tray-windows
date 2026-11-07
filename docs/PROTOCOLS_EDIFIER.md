# Edifier Protokoll-Dokumentation (NeoBuds Pro 3 / BES Protokoll)

> **Status:** An Hardware verifiziert (EDIFIER NeoBuds Pro 3, 2026-10-31, Capture 1 & 2).  
> **Verifizierte Bestandteile:** RFCOMM UUID, V2 + XOR (0xA5), F2 TWS Battery Query (Left, Right, Case, Bud-in-Case Flag), CRC8-Berechnung über übertragene Bytes, sowie unaufgefordertes F2-Push-Verhalten.  
> **Nicht beantwortet:** D0 (Classic Battery Query liefert keine Antwort auf NeoBuds Pro 3).  
> **Ausstehend für Phase 2:** ANC-Steuerung (CC/C1).

---

## 1. Schnittstelle & Transport

* **Steuerkanal:** Bluetooth RFCOMM Socket
* **Service-UUID:** `EDF00000-EDFE-DFED-FEDF-EDFEDFEDFEDF`
* **Quelle:** mEDIFIER (`Qt/comms/comm.cpp`), WearEars (`EdifierClient.java`)
* **Status:** *verifiziert am Gerät: ja (NeoBuds Pro 3, 2026-10-31)*

---

## 2. Frame-Formate

### V1-Frame (Klassisches Format)
Verwendet für ältere Edifier-Kopfhörer (z. B. W820NB, W200BT Plus):

```text
[AA] [LEN] [CMD] [DATA...] [CRC_HI] [CRC_LO]
```

* **Header:** `0xAA` (Host -> Gerät). Antworten vom Gerät beginnen mit `0xBB` oder `0xCC`.
* **LEN:** 1 Byte. Länge von `CMD` + `DATA` (ohne Header, LEN und CRC).
* **CMD:** 1 Byte Befehls-ID.
* **DATA:** Variable Payload-Bytes.
* **CRC16 (Big Endian):**
  $$\text{CRC} = \left(\sum \text{aller Bytes inkl. AA und LEN}\right) + \text{0x2019} \pmod{2^{16}}$$
* **Quelle:** mEDIFIER (`Qt/comms/comm.cpp`), WearEars (`EdifierClient.java`)
* **Status:** *verifiziert am Gerät: nein* (wird von NeoBuds Pro 3 nicht genutzt)

### V2-Frame (TWS / Neueres Format)
Verwendet von NeoBuds Pro 3 (und anderen BES-basierten TWS-Modellen):

```text
[AA/BB] [EC] [CMD] [LEN_HI] [LEN_LO] [PAYLOAD...] [CRC8]
```

* **Header:** `0xAA 0xEC` (Host -> Gerät); Antworten vom Gerät beginnen mit `0xBB 0xEC`.
* **CMD:** 1 Byte Befehls-ID.
* **LEN:** 2 Bytes (Big Endian) Länge der Payload.
* **PAYLOAD:** Mit `0xA5` XOR-verschleiert (`byte ^ 0xA5`).
* **CRC8:** 1 Byte: Summe aller vorhergehenden Bytes wie übertragen (d. h. über die verschleierte Payload) $\pmod{256}$.
* **Quelle:** WearEars (`EdifierClient.java`) & Hardware-Probe
* **Status:** *verifiziert am Gerät: ja (NeoBuds Pro 3, 2026-10-31)*

---

## 3. Befehls-Übersicht (Sicher vs. Gefährlich)

### Sichere Lese- & Steuerbefehle

| CMD (Hex) | Name | Format / Payload | Quelle | Status |
| :--- | :--- | :--- | :--- | :--- |
| `0xD0` | **Battery Query (Single / Classic)** | Anfrage: keine Payload. Antwort: 1 Byte Akkustand (`value & 0x7F`). | mEDIFIER (`basedevice.cpp`) | *verifiziert am Gerät: keine Antwort auf NeoBuds Pro 3* |
| `0xF2` | **Battery Query (TWS)** | Anfrage: keine Payload. Antwort: 6 Bytes Payload (siehe Details unten). | WearEars & Hardware-Probe | *verifiziert am Gerät: ja (NeoBuds Pro 3, 2026-10-31)* |
| `0xCC` | **ANC State Query** | Fragt den aktuellen ANC-Modus ab. | mEDIFIER (`basedevice.cpp`) | *verifiziert am Gerät: nein* |
| `0xC1` | **ANC Mode Set** | Setzt ANC-Modus (z. B. High ANC, Ambient Sound, Off). *(In Phase 1 nicht verwendet!)* | mEDIFIER (`basedevice.cpp`) | *verifiziert am Gerät: nein* |
| `0x08` | **Game Mode Query** | Fragt Low-Latency / Game-Mode ab. | mEDIFIER (`basedevice.cpp`) | *verifiziert am Gerät: nein* |
| `0x09` | **Game Mode Set** | `0x01` = aktiv, `0x00` = inaktiv. | mEDIFIER (`basedevice.cpp`) | *verifiziert am Gerät: nein* |

### F2 TWS Battery Payload Details (NeoBuds Pro 3)
Empfangene V2+XOR-Frames (Muster aus Capture 1 & 2):
- Beide Buds im Ohr / außerhalb des Cases: `03 4E 51 00 03 11` (Left: 78%, Right: 81%, Case: None/0%, BudInCase: false)
- Mindestens ein Bud im geöffneten Case: `03 4E 51 34 02 11` (Left: 78%, Right: 81%, Case: 52%, BudInCase: true)

Nutzlast nach De-XOR (`^ 0xA5`): `03 [L] [R] [C] [FLAG] 11`:
- **Byte 0:** unbekannt (`0x03`, laut ConneX/BudsLink `peerValue`, konstant)
- **Byte 1:** Left % (`0..=100`, z. B. `0x4E` = 78): *verifiziert*
- **Byte 2:** Right % (`0..=100`, z. B. `0x51` = 81): *verifiziert*
- **Byte 3:** Case % (`0x00` = unbekannt bzw. kein Bud im Case; `1..=100` = Akkustand des Lade-Cases, z. B. `0x34` = 52 %): *verifiziert (Gegenprobe mit Edifier App: 52 %)*
- **Byte 4:** Case-Präsenz (`0x03` = kein Bud im Case, `0x02` = mindestens ein Bud im geöffneten Case): *verifiziert*. Hinweis: Differenziert nicht zwischen linkem und rechtem Bud und ändert sich beim Einlegen des zweiten Buds nicht weiter.
- **Byte 5:** unbekannt (`0x11`, laut ConneX/BudsLink `edValue`, konstant)

**Besonderheiten & Gerätezustände:**
- **Ladezustand (Charging):** Wird im Protokoll der NeoBuds Pro 3 nicht übermittelt (auch die offizielle Edifier ConneX App zeigt kein Lade-Icon oder Charging-Status).
- **Case geschlossen (Lid Closed):** Das Schließen des Deckels sendet ggf. noch einen finalen F2-Push, woraufhin die Bluetooth-Verbindung sofort abreißt (Socket Disconnect). Die Tray-App behandelt dies direkt als "Getrennt".
- **Unaufgeforderte Benachrichtigungen (Push):** Das Gerät sendet F2 eigenständig bei Zustandsänderungen (z. B. Bud ins Case gelegt, Akkuänderung) als unaufgefordertes Push-Paket. Der Tray-Hintergrunddienst kann somit passiv lauschen.  
*Status:* *verifiziert am Gerät: ja (NeoBuds Pro 3, 2026-10-31)*

### Weitere beobachtete Push-Frames (nur lesend, unaufgefordert)

| CMD (Hex) | Payload (Beispiel) | Beschreibung / Beobachtung | Status |
| :--- | :--- | :--- | :--- |
| `0xCC` | `20 06` | Pushed, wenn ein Bud aus dem Ohr genommen wird. BudsLink interpretiert CC als ANC `[group, mode]`, Modus 06 ist jedoch unklar (App zeigte ANC Aus). | *beobachtet, Bedeutung teilweise unbestätigt* |
| `0x50` | `00 10 "..." 14 "..."` | Now-Playing-Metadaten vom Smartphone (`[state][title len][title][artist len][artist]`). Für Tray-App irrelevant. | *beobachtet, für Tray irrelevant* |

---

## 4. ⚠️ Gefährliche Befehle (STRENGSTENS VERBOTEN)

Diese Befehle dürfen **niemals** an das Gerät gesendet werden (auch nicht zu Testzwecken):

| CMD (Hex) | Auswirkung laut Referenzcode |
| :--- | :--- |
| `0x07` | **Factory Reset!** (Nicht ANC Set!) |
| `0xCD` | Disconnect (Verbindung trennen) |
| `0xCE` | Power Off (Ausschalten) |
| `0xCF` | Re-Pairing auslösen |
| `0xD1` bis `0xD3` | Abschalt-Timer konfigurieren |
| `0xCA` | Gerät umbenennen |

---

## 5. Reverse-Engineering Strategie für Phase 1

Da WearEars selbst vermerkt, dass das V2-Format nicht an echten NeoBuds Pro 3 getestet wurde:
1. `earbuds-tray probe` probiert in folgender Reihenfolge:
   - V2 mit XOR: `0xD0`, `0xF2`
   - V2 ohne XOR: `0xD0`, `0xF2`
   - V1: `0xD0`, `0xF2`
2. Wartezeit: 1,5 Sekunden pro Anfrage.
3. Nur die sicheren Abfrage-Befehle werden implementiert.
4. Sobald ein echtes Antwortpaket eintrifft, werden die Hex-Dumps hier dokumentiert und als Unit-Test-Fixtures übernommen.
