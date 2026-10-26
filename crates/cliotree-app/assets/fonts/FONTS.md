# Bundled fonts

All are variable fonts with a weight axis (`wght`), under the SIL Open Font
License 1.1; each licence, with its copyright line, is in the `*-OFL.txt` file
next to the font. They are built into the program (see `src/style.rs`).

Source: the Google Fonts repository, https://github.com/google/fonts, commit
`e7db6ca603fd4862010383ff17ffd6cc4ffac1c0`, folder `ofl/`. The files were
renamed (the axis names in brackets dropped).

| File | Original | Use | SHA-256 |
|---|---|---|---|
| `Inter.ttf` | `inter/Inter[opsz,wght].ttf` | interface (sans) | `29160a80ff49ddcab2c97711247e08b1fab27a484a329ce8b813d820dc559031` |
| `GeistMono.ttf` | `geistmono/GeistMono[wght].ttf` | formulas and paths (monospace) | `d00e590b8eb3a59acc329b2d044fd143ae935090b7da33199ebee27cc7de8196` |
| `SourceSerif4.ttf` | `sourceserif4/SourceSerif4[opsz,wght].ttf` | serif, for forms | `97b2d4da6e3cb494b5a1e66ae176914d852ccabef49e0c02c0df25f3e39aca0b` |
| `Oswald.ttf` | `oswald/Oswald[wght].ttf` | condensed, for forms | `5b38c246e255a12f5712d640d56bcced0472466fc68983d2d0410ec0457c2817` |
| `DancingScript.ttf` | `dancingscript/DancingScript[wght].ttf` | script, for forms | `21808625578fe8d8cd10cb684be546dca077b27cd03a53a2f1ec11dc743c924c` |
| `Caveat.ttf` | `caveat/Caveat[wght].ttf` | handwriting, for forms | `0bdb6b660482d31531b3945849fba5916b3ef8695da7024a9e6b9ee3c4157988` |

To check a file: `sha256sum Inter.ttf`.
