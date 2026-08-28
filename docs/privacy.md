# Privacy

Callback's runtime:

- Opens no TCP or UDP listener
- Transmits no captured message content
- Stores data in a local SQLite file
- Uses `chrome.storage.local` only (never `storage.sync`)
- Does not fetch selector packs at runtime
- Logs capture ids, kinds, and scores — never raw message bodies

Chrome Web Store and package-manager traffic is outside this runtime guarantee.

Native messaging uses stdin/stdout and a current-user ACL named pipe. Windows Focus Assist / DND is left to the OS; Callback never bypasses it with an always-on-top card.

Purge from Health or `callback-app --purge` deletes the local database and unregisters the native-host registry key. Autostart, when enabled, writes only a current-user Run key and never bypasses Windows Focus Assist.

The global shortcut only opens the local `?window=quick` window. Callback does not scrape selected text from other applications. CI Windows installers are unsigned; SmartScreen will warn until a certificate is added.
