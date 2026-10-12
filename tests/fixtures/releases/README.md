Public source fixtures verified on Korax on 2026-10-12. Each JSON fixture retains only fields needed to select a stable publication. The full responses are not shipped. Endpoint and website mappings live in `catalog/release-sources.json`.

- Chrome: Linux/stable Version History, first version.
- Thunderbird: `LATEST_THUNDERBIRD_VERSION`, not the development/beta field.
- VS Code: stable Linux x64 endpoint `name`; `version` is a Git commit, not the product version.
- Audacity, Protégé, Telegram, Voxtype: official repository latest non-draft, non-prerelease tag, with their documented prefix removed. Voxtype's installed package points to voxtype.io; its download page links to peteonrails/voxtype.
- Ghostty: official repository semantic version tags, skipping `tip` and prereleases, checked against ghostty.org/download. `/releases/latest` returns 404 because the repository uses tagged stable releases and its rolling tip release.
- Remmina: official GitLab releases, excluding upcoming releases. The GitHub mirror is not a current release source.
- Steam: same-channel Valve manifest over HTTPS at client-update.akamai.steamstatic.com; package launcher and client manifest are independent.
- TickTick: no reliable publication source is asserted. Its package channel can still be checked.
- GeForce NOW: its exact installation/ref/origin is checked through Flatpak, without fabricating an application version.
