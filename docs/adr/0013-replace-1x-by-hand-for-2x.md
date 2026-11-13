# Replace the maintainer's 1.x install by hand for 2.0

Decided 2026-11-13. Open TouchDeck will not ship an automatic 1.x to 2.0 move. The maintainer has the only 1.x app install and will uninstall it, keep its data, then install the 2.0 MSI. This repeats the manual replacement decision for v0.2.0 in ADR 0005. A bridge release would add work for an update path nobody else needs; PRs #391 and #392 were closed unmerged.

The 1.x uninstaller keeps the app's data folders by default, and 2.0 reads the same folders and Windows Credential Manager entries. The 2.0 MSI uses the same program folder as 1.x and refuses to install while the 1.x uninstaller is there. The maintainer must uninstall 1.x first and decline its offer to delete Layouts and settings. Windows 10 users can keep using 1.x; 2.0 requires Windows 11.

This amends the automatic move in ADR 0011 and the bridge release in ADR 0012. Both remain as records of the decisions made on 2026-11-12.
