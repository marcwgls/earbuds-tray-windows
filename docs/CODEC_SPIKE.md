# Codec spike (2026-11-03)

Question: which A2DP codec does Windows use with the NeoBuds Pro 3, can the tray show it, and can we get a better one.

## How to read the codec

Windows has no API or settings page for the active codec. The inbox driver `BthA2dp.sys` logs it through the TraceLogging provider `Microsoft.Windows.Bluetooth.BthA2dp`, GUID `8776ad1e-5022-4451-a566-f47e708b9075` (GUID read from the provider metadata in the driver binary). The manifest provider `ddb6da39-08a7-4579-8d0c-68011146e205` from Microsoft's `BluetoothStack.wprp` has no codec fields and is not decodable without its manifest.

Useful events (all keywords, level 5):

- `AvdtpSepInuseInfo` (on connect): one per stream endpoint the buds offer, with `A2dpStandardCodecId`, `A2dpVendorId`, `A2dpVendorCodecId`.
- `AvdtpSetConfigInfo` (on connect): the configuration Windows chose, with codec ids, `SampleRate`, `ChannelCount`.
- `A2dpStreaming` (when playback stops): codec ids and `DurationInMilliseconds`.

Capture (needs an elevated prompt): `logman start <name> -p {8776ad1e-5022-4451-a566-f47e708b9075} 0xFFFFFFFFFFFFFFFF 5 -o x.etl -ets`, reconnect the buds, play and stop music, `logman stop <name> -ets`, then `tracerpt x.etl -o out.xml -of XML` (no admin needed to decode).

## Result on the test PC (Windows 11 26H2, build 26300)

- Buds offer four endpoints: SBC (0), AAC (2), and two vendor codecs with vendor id 1338 (0x053A, Savitech): 19507 (0x4C33) and 19509 (0x4C35), most likely LHDC v3/v4 and LHDC v5. No LDAC endpoint was offered (maybe a setting in the Edifier app).
- Windows chose **AAC, 48 kHz, 2 channels**. That is the best codec the inbox Windows stack supports (SBC and AAC only).

## Better codec

Not possible from a normal app. Codec negotiation and encoding happen in the kernel driver, and Windows gives user mode no access to the L2CAP/AVDTP channels. LDAC or LHDC would need a replacement kernel driver: Microsoft signing (EV certificate plus partner account), months of work, bluescreen risk. LHDC also needs a Savitech license; the LDAC encoder from AOSP is Apache 2.0. Third party drivers that do this exist.

## Codec display in the tray

Starting an ETW session needs admin rights (or membership in "Performance Log Users"). The tray runs as a normal user. Options: a one time permission change from the installer, or an elevated helper. Both change system security for little gain: with these buds Windows always picks AAC. Decision: not built for now.
