# Third-Party Notices & Attribution

This project incorporates logic, algorithms, or concepts derived from the following open-source projects under their respective licenses:

---

## 1. mEDIFIER
* **Project:** mEDIFIER
* **Author / Copyright:** Copyright (c) 2022-2024 wh201906
* **Repository:** https://github.com/wh201906/mEDIFIER
* **License:** MIT License
* **Usage:** Edifier V1 packet structure, CRC16 calculation (`sum + 0x2019`), and command definitions (`0xD0`, `0xC1`, `0xCC`, `0x08`, `0x09`).

---

## 2. WearEars
* **Project:** WearEars
* **Author / Copyright:** Copyright (c) 2023 hdzw1155
* **Repository:** https://github.com/hdzw1155/WearEars
* **License:** MIT License
* **Usage:** Edifier V2 frame formatting (`0xAA 0xEC`), XOR obfuscation (`0xA5`), CRC8 calculation, and TWS battery command (`0xF2`).

---

## 3. win-bt-stereo-vs-handsfree
* **Project:** win-bt-stereo-vs-handsfree
* **Author / Copyright:** Copyright (c) 2024 Mark.Huang (Z-M-Huang)
* **Repository:** https://github.com/Z-M-Huang/win-bt-stereo-vs-handsfree
* **License:** MIT License with Attribution Clause
* **Attribution Requirement:** Derived works must state the original author name (Mark.Huang) and repository URL.
* **Usage:** WinRT Bluetooth device enumeration and RFCOMM transport pattern (`src/drivers/edifier/transport.rs`).

---

## 4. BudsLink
* **Project:** BudsLink
* **Author / Copyright:** Copyright (c) 2024 maniacx
* **Repository:** https://github.com/maniacx/BudsLink
* **Commit:** `3d894c0cf0071b0f47600c1c72ec92a9704d8823`
* **License:** GNU General Public License v3.0 (GPL-3.0)
* **Files:** `assets/icons/bbm-earbuds-stem-left-symbolic.svg`, `assets/icons/bbm-earbuds-stem-right-symbolic.svg`, `assets/icons/bbm-case-normal-symbolic.svg` (copied from `data/icons/hicolor/scalable/actions/`)
* **Usage:** Symbolic SVG glyphs for earbuds (left/right stems) and charging case. The circular battery ring design and visualization are modeled on `src/lib/widgets/circleBatteryIconWidget.js`.

---

## 5. GalaxyBudsClient
* **Project:** GalaxyBudsClient
* **Author / Copyright:** Copyright (c) timschneeb and contributors
* **Repository:** https://github.com/timschneeb/GalaxyBudsClient
* **License:** GNU General Public License v3.0 (GPL-3.0)
* **Files:** `src/drivers/galaxy/fixtures/Buds2Pro_rev13.bin`, `BudsPlus_rev13.bin`, `Buds_rev3.bin` (copied from `GalaxyBudsClient.Tests/TestData/ExtendedStatusUpdate/`)
* **Usage:** The three captured `ExtendedStatusUpdate` frames are used as known-answer tests for the Galaxy Buds frame parser. The frame layout, CRC16/XMODEM and the status message layouts (`0x60`, `0x61`) follow the project's source and its "Galaxy Buds Plus RFComm Protocol Notes". Expected values come from its `ExtendedStatusUpdateTests.cs`.
