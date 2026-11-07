# Device support research (2026-11-01)

Question: do earbuds tell us what they are, and do brands share one protocol, so we can sort devices by how they communicate instead of unlocking each model?

## What a device tells us

1. **Name** (for example "EDIFIER NeoBuds Pro 3"). Free text, set by the vendor, renamable by the user. Weak.
2. **Device ID record** (SDP service 0x1200: vendor id source, vendor id, product id). Windows puts it into the hardware ids (`BTHENUM\{uuid}_VID&0001xxxx_PID&yyyy`). Optional, and our NeoBuds Pro 3 do not publish it: Windows only has `_LOCALMFG&08e7` (that is the PC's own radio, not the buds).
3. **Service UUIDs** (SDP). Always there, and the strongest signal. NeoBuds Pro 3 (`pnputil /enum-devices /ids`):
   - `1101` SPP, `110b` A2DP sink, `110c`/`110e` AVRCP, `111e` Hands-Free
   - `edf00000-edfe-dfed-fedf-edfedfedfedf`: Edifier vendor channel (what we use today)
   - `df21fe2c-2515-4fdb-8886-f12c4d67927c`: **Google Fast Pair Message Stream**
   - `66666666-6666-6666-6666-666666666666`: unknown, probably a chip vendor (Airoha or similar) service. Not investigated.
4. **Fast Pair model id** (3 bytes), sent over the Message Stream when it connects. Identifies the exact model; Google keeps the public registry.

## Google Fast Pair Message Stream: a cross-brand battery protocol

- RFCOMM, fixed UUID `df21fe2c-2515-4fdb-8886-f12c4d67927c`.
- Frame: group (1 byte), code (1 byte), length (2 bytes, big endian), data.
- Group `0x03` "Device information event": `0x01` model id, `0x02` BLE address, `0x03` battery updated, `0x04` remaining battery time.
- Battery updated: one byte each for left, right, case; bit 7 = charging, bits 0-6 = percent, `0x7F` = unknown. Example `57 41 7F`: L 87 %, R 65 %, case unknown.
- The provider sends model id and battery on its own when the stream connects, and battery again on every change. No request from us needed, so a pure listener is enough.
- Supported by most Android-oriented earbuds (Sony, Soundcore, JBL, Nothing, Pixel Buds, Edifier, ...). Not by AirPods, not by Samsung Galaxy Buds (they use their own Samsung pairing).
- Open questions: does a Windows PC get the stream at all, or do some buds expect an Android seeker first? Is the channel exclusive like the Edifier one (phone with Google Play Services connected)? Needs a spike on the NeoBuds.

## Vendor protocols (rich features: ANC, EQ, exact L/R/case)

From known open source clients, not verified here in detail:

| Family | Transport | One protocol for the brand? | Reference |
|---|---|---|---|
| Edifier | RFCOMM, `EDF00000-...` | shared framing, per model feature sets | ours (`docs/PROTOCOLS_EDIFIER.md`) |
| Soundcore (Anker) | RFCOMM | shared framing, per model differences (model ids A3xxx, D1xxx), about 40 models | OpenSCQ30 (Rust, runs on Windows) |
| Sony | RFCOMM | one protocol family with versions (V1/V2) | Gadgetbridge, SonyHeadphonesClient |
| Samsung Galaxy Buds | RFCOMM (SPP) | shared framing, message ids differ per model | GalaxyBudsClient (Windows), Gadgetbridge |
| Huawei FreeBuds | RFCOMM | shared framing | FreeBuddy, Gadgetbridge |
| Nothing | RFCOMM | shared | Gadgetbridge |
| Apple AirPods | L2CAP (AAP), BLE adverts | own | hard on Windows (no plain L2CAP API) |

So brands do share a protocol, but usually with per model differences (feature sets, sometimes message ids). Sorting by communication works at the transport and framing level; features still need a per model table.

## Tiers

1. **Vendor driver** (rich: exact L/R/case, later ANC and so on). Chosen by vendor RFCOMM UUID plus name or model id. Today only Edifier.
2. **Fast Pair listener** (generic L/R/case for any Fast Pair device). Chosen by the `df21fe2c-...` UUID. Read only, sends nothing. Gives the model id, so we can also show the real model name and suggest "raise an issue" for a vendor driver.
3. **Windows HFP battery** (one value, any hands-free headset). Device property `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2` on the Hands-Free node. Tooltip: "Your earbuds are not yet supported! Please raise an issue on GitHub".

## Spike results

Spike on 2026-11-01 with the NeoBuds Pro 3:
- Windows successfully opens the Fast Pair Message Stream, even alongside a connected Android phone (multipoint) and an active Edifier RFCOMM channel.
- Implemented as a strictly read-only listener (`FastPairTransport`), never writing to the socket.
- Tiers 1 (Vendor Edifier), 2 (Fast Pair listener), and 3 (Windows HFP battery) are implemented with automatic candidate picking and audio tracking.

## Research for other brands (2026-11-07)

Rule for devices we cannot test: **listen only**. The app sends nothing and reads only what the device sends by itself.

| Brand | Listen only possible? | Built? | Why |
|---|---|---|---|
| Samsung Galaxy Buds | yes: the buds push an extended status (`0x61`) with L/R/case on connect and a status (`0x60`) on every change | **yes** (beta) | Biggest gap: Galaxy Buds have no Fast Pair, so before they only got the single Windows value. |
| Soundcore | partly: the full state needs a request (`RequestState`), only battery changes are pushed, and only as levels 0 to 5 | no | Fast Pair already gives Soundcore exact L/R/case percent. |
| Sony | no: newer models need an init handshake and host ACKs; opcode `0x22` is battery on v2 but **power off** on v1 | no | Not safe without a test device. Newer Sony models already work through Fast Pair. |
| More Edifier models | already covered: every device that publishes the Edifier service `EDF00000-...` gets the Edifier tier and the `F2` battery query | label only | mEDIFIER only covers three old headphones with other UUIDs, nothing to take over. |

Sources: GalaxyBudsClient (GPL-3.0, github.com/timschneeb/GalaxyBudsClient), its "Galaxy Buds Plus RFComm Protocol Notes", OpenSCQ30 (GPL-3.0), sonyd and Sony Device Center notes, mEDIFIER (MIT).

Galaxy tier order: Edifier, then Fast Pair, then Galaxy, then Windows. A Galaxy model that also publishes the Fast Pair stream keeps the tested Fast Pair tier.

## Galaxy Buds protocol (from GalaxyBudsClient)

- RFCOMM service UUIDs: `2e73a4ad-332d-41fc-90e2-16bef06523f2` (Buds+ and newer, "SppNew") and `00001102-0000-1000-8000-00805f9b34fd` (original Galaxy Buds, "SppLegacy"). Both are Samsung specific; `00001101-...` (standard SPP) is **not** used for matching, because many devices have it.
- Frame (new): `FD`, header u16 little endian (bits 0 to 9 size, bit 12 request, bit 13 fragment), message id, payload, CRC16 XMODEM (poly 0x1021, init 0, no reflection) over id + payload, written **little endian** on the wire, `DD`. Size counts id + payload + CRC. Check: compute the CRC over id + payload and compare it with the u16 LE read from the wire. (Feeding the wire bytes in order through the CRC does **not** give 0; verified on the fixtures: Buds2 Pro CRC 0x37B6, wire `B6 37`.)
- Frame (legacy Buds): `FE`, type byte, size byte, id, payload, CRC, `EE`.
- `0x61` EXTENDED_STATUS_UPDATED: payload `[0]` revision, `[1]` ear type, `[2]` battery L, `[3]` battery R, `[4]` coupled, `[5]` main connection, Buds+ and newer: `[6]` placement (high nibble L, low nibble R), `[7]` case battery. Legacy Buds: no case value.
- `0x60` STATUS_UPDATED (Buds+ and newer): `[0]` revision, `[1]` L, `[2]` R, `[3]` coupled, `[4]` main connection, `[5]` placement, `[6]` case. Legacy Buds: `[0]` ear type, `[1]` L, `[2]` R, no case.
- Real captures exist as test data in GalaxyBudsClient (`GalaxyBudsClient.Tests/TestData/ExtendedStatusUpdate/*.bin`, GPL-3.0). They are **full frames** from SOM to EOM (checked: `FD ... DD` and `FE ... EE`, CRC matches), for example Buds2 Pro rev 13: L 100, R 100, case 88; Buds rev 3 (legacy): L 70, R 95. Three of them are copied to `src/drivers/galaxy/fixtures/` (credited in `THIRD_PARTY_NOTICES.md`) and serve as CRC known-answer tests.
- Battery 0: a bud that is not coupled or not reachable may report 0. Treat 0 as unknown (`None`) for buds and case. Real buds switch off before 0 %, so nothing useful is lost, and no false low-battery toast fires for a missing bud.

Open risks, accepted as beta (see the README):

1. The notes say a client "should" answer with an ACK and `MANAGER_INFO`. We do not. It is unknown whether the buds keep sending `0x60` updates without that. If not, the first value arrives on connect and later values only after a reconnect.
2. The phone's Galaxy Wearable app may hold the channel, like the Edifier app does.
3. Never send anything: one known command (`0xD4`) even dumps link keys. The transport for this driver has no write path at all.

## Sources

- Fast Pair Message Stream: https://developers.google.com/nearby/fast-pair/specifications/extensions/messagestream
- Fast Pair Device information: https://developers.google.com/nearby/fast-pair/specifications/extensions/deviceinformation
- Fast Pair battery notification: https://developers.google.com/nearby/fast-pair/specifications/extensions/batterynotification
- OpenSCQ30: https://github.com/Oppzippy/OpenSCQ30
- Gadgetbridge Galaxy Buds wiki: https://github.com/Freeyourgadget/Gadgetbridge/wiki/Galaxy-Buds
- Gadgetbridge changelog: https://github.com/Freeyourgadget/Gadgetbridge/blob/master/CHANGELOG.md
- BTHENUM hardware id examples: https://learn.microsoft.com/en-us/answers/questions/3833225/device-manager-2-items-under-other-devices-without
