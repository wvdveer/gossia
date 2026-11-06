# Gossia

Gossia workbooks in Visual Studio Code. A workbook's sheets are trees, each defined by a schema (JSON Schema, XSD or a COBOL copybook), and its formulas are written in Python.

- Open a `.gossia` file, or make an empty one, and it opens in Gossia's editor. VS Code's Save, Revert and Undo history work as they do for any file.
- Modules open in VS Code's own editor as `gossia:` files, with its highlighting, search and Python extensions. Saving one puts the module in the workbook.
- Gossia's commands are in the Command Palette, under **Gossia**.
- Data sources read over HTTP go through the extension, so the sites they read need no CORS headers. SQLite, PostgreSQL, MariaDB, ODBC and Beancount sources need the desktop application.

- Gossia speaks English, French, Spanish, Portuguese, Japanese and Simplified Chinese, following VS Code's display language unless its Options choose another. For Japanese and Chinese it loads Noto Sans JP or SC (SIL Open Font License, `media/fonts/OFL.txt`) the first time it needs one.

The Python runs in the editor's own worker (RustPython compiled to WebAssembly), so VS Code's Python debugger cannot step through formulas or modules.

More in the [design document](https://github.com/wvdveer/gossia/blob/main/DESIGN.md).
