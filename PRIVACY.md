# Privacy policy

*Gossia, all versions. Last updated 7 November 2026.*

Gossia does not collect, store or send any personal information. It has no telemetry, no analytics, no crash reporting, no advertising and no accounts, and it does not check for updates.

## What Gossia connects to

Gossia connects to the internet, or to a database on your network, only when a workbook reads from or writes to a **data source**: a web address, a database or an ODBC source that you defined, or that the workbook's author defined and you then trusted. Those requests go only to the addresses in those definitions, and carry only what the workbook's formulas or procedures put in them.

A workbook from someone else runs no formulas and makes no requests until you trust it.

## What Gossia keeps on your computer

- **Your workbooks** stay where you save them. Gossia does not copy them anywhere else.
- **Settings**, in `%APPDATA%\Gossia` on Windows (`~/.config/gossia` on Linux): your preferences, the list of recently opened workbooks, the record of which workbooks you trust, the history of the code editor's Immediate window, and your data source definitions with their passwords and keys. Data source definitions are never saved into workbooks.
- In the **Visual Studio Code extension**, the same settings are kept in the storage Visual Studio Code gives the extension, and data source passwords and keys in Visual Studio Code's secret storage, which keeps them in your operating system's keychain, on your computer.

Removing Gossia's settings folder (or uninstalling the app from the Microsoft Store, which removes its storage) removes all of it.

## Contact

Questions about this policy: open an issue at https://github.com/wvdveer/gossia/issues.
