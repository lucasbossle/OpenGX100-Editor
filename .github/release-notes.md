## 🍎 macOS only

**OpenGX100 Editor ${VERSION}** is an open-source editor for the **BOSS GX-100**. It's built for **macOS 11 or newer** as a universal app that runs on Apple Silicon and Intel Macs.

${CHANGES}
### Install
1. Download **OpenGX100-Editor-${VERSION}-macos.zip** below and unzip it.
2. Move **OpenGX100 Editor.app** to Applications, replacing the old version.
3. The app isn't signed with an Apple Developer ID, so macOS blocks it the first time. To allow it, open it once, go to **System Settings → Privacy & Security**, and click **Open Anyway**. Or run this in Terminal:
   `xattr -dr com.apple.quarantine "/Applications/OpenGX100 Editor.app"`
4. Quit BOSS TONE STUDIO, connect the GX-100 over USB, and open OpenGX100 Editor.

You need **BOSS TONE STUDIO for GX-100** installed (free from [boss.info](https://www.boss.info/)): on first launch the app reads the pedal's parameter tables from it. This app contains no data from Roland. Back up your pedal (Backups tab) before writing anything.

Not affiliated with Roland or BOSS. "BOSS", "GX-100" and "BOSS TONE STUDIO" are trademarks of Roland Corporation.
