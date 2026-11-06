"""Packs the VS Code extension as a .vsix without Node.js (cargo xtask vsix
runs it, DESIGN.md, Web port, 0.3.0, In VS Code).

A .vsix is a zip: the extension's files under extension/, a manifest
(extension.vsixmanifest) and the content types of the files
([Content_Types].xml), as vsce writes them. The files are those of the
extension folder less what .vscodeignore names (a pattern per line: a path
with * wildcards, or a folder followed by /**). As vsce does, the licence is
put in as LICENSE.txt and the description is taken from package.nls.json when
package.json refers to it.

    python packaging/vscode/pack-vsix.py EXTENSION_DIR OUT.vsix
"""

import fnmatch
import json
import mimetypes
import sys
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape, quoteattr

# What is never packed, whatever .vscodeignore says (as vsce leaves out).
ALWAYS_LEFT_OUT = [".vscodeignore", "**/.git/**", "**/*.vsix", "**/.DS_Store", "**/.gitignore"]

CONTENT_TYPES = {
    ".js": "application/javascript",
    ".json": "application/json",
    ".md": "text/markdown",
    ".png": "image/png",
    ".svg": "image/svg+xml",
    ".ttf": "font/ttf",
    ".txt": "text/plain",
    ".vsixmanifest": "text/xml",
    ".wasm": "application/wasm",
}


def left_out(rel, patterns):
    for pattern in patterns:
        candidates = [pattern]
        if pattern.startswith("**/"):
            candidates.append(pattern[3:])
        for p in candidates:
            if p.endswith("/**"):
                folder = p[:-3]
                if rel == folder or rel.startswith(folder + "/") or fnmatch.fnmatch(rel, p):
                    return True
            elif fnmatch.fnmatch(rel, p) or ("/" not in p and fnmatch.fnmatch(rel.rsplit("/", 1)[-1], p)):
                return True
    return False


def files(ext):
    ignore = ext / ".vscodeignore"
    patterns = list(ALWAYS_LEFT_OUT)
    if ignore.is_file():
        for line in ignore.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                patterns.append(line.lstrip("/"))
    found = []
    for path in sorted(ext.rglob("*")):
        rel = path.relative_to(ext).as_posix()
        if path.is_file() and not left_out(rel, patterns):
            found.append(rel)
    return found


def localised(text, nls):
    if isinstance(text, str) and text.startswith("%") and text.endswith("%"):
        return nls.get(text[1:-1], text)
    return text


def manifest(package, nls, packed):
    """The extension.vsixmanifest, from package.json."""
    name = package["name"]
    repo = package.get("repository", {})
    repo = repo.get("url", "") if isinstance(repo, dict) else repo
    if repo and not repo.endswith(".git"):
        repo += ".git"
    base = repo[: -len(".git")] if repo.endswith(".git") else repo
    props = [
        ("Microsoft.VisualStudio.Code.Engine", package.get("engines", {}).get("vscode", "*")),
        ("Microsoft.VisualStudio.Code.ExtensionDependencies", ",".join(package.get("extensionDependencies", []))),
        ("Microsoft.VisualStudio.Code.ExtensionPack", ",".join(package.get("extensionPack", []))),
        ("Microsoft.VisualStudio.Code.ExtensionKind", ",".join(package.get("extensionKind", ["workspace"]))),
        ("Microsoft.VisualStudio.Code.LocalizedLanguages", ""),
        ("Microsoft.VisualStudio.Code.EnabledApiProposals", ""),
    ]
    if package.get("main") or package.get("browser"):
        props.append(("Microsoft.VisualStudio.Code.ExecutesCode", "true"))
    if base:
        props += [
            ("Microsoft.VisualStudio.Services.Links.Source", repo),
            ("Microsoft.VisualStudio.Services.Links.Getstarted", repo),
            ("Microsoft.VisualStudio.Services.Links.GitHub", repo),
            ("Microsoft.VisualStudio.Services.Links.Support", base + "/issues"),
            ("Microsoft.VisualStudio.Services.Links.Learn", base + "#readme"),
        ]
    props += [
        ("Microsoft.VisualStudio.Services.GitHubFlavoredMarkdown", "true"),
        ("Microsoft.VisualStudio.Services.Content.Pricing", "Free"),
    ]
    assets = [("Microsoft.VisualStudio.Code.Manifest", "extension/package.json")]
    readme = next((p for p in packed if p.lower() == "extension/readme.md"), None)
    if readme:
        assets.append(("Microsoft.VisualStudio.Services.Content.Details", readme))
    license_path = "extension/LICENSE.txt" if "extension/LICENSE.txt" in packed else None
    if license_path:
        assets.append(("Microsoft.VisualStudio.Services.Content.License", license_path))
    icon = "extension/" + package["icon"] if package.get("icon") else None
    if icon:
        assets.append(("Microsoft.VisualStudio.Services.Icons.Default", icon))

    lines = [
        '<?xml version="1.0" encoding="utf-8"?>',
        '<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011"'
        ' xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">',
        "  <Metadata>",
        f"    <Identity Language=\"en-US\" Id={quoteattr(name)} Version={quoteattr(package['version'])}"
        f" Publisher={quoteattr(package['publisher'])} />",
        f"    <DisplayName>{escape(localised(package.get('displayName', name), nls))}</DisplayName>",
        '    <Description xml:space="preserve">'
        f"{escape(localised(package.get('description', ''), nls))}</Description>",
        f"    <Tags>{escape(','.join(package.get('keywords', [])))}</Tags>",
        f"    <Categories>{escape(','.join(package.get('categories', [])))}</Categories>",
        "    <GalleryFlags>Public</GalleryFlags>",
        "    <Properties>",
    ]
    lines += [f"      <Property Id={quoteattr(k)} Value={quoteattr(v)} />" for k, v in props]
    lines.append("    </Properties>")
    if license_path:
        lines.append(f"    <License>{license_path}</License>")
    if icon:
        lines.append(f"    <Icon>{escape(icon)}</Icon>")
    lines += [
        "  </Metadata>",
        "  <Installation>",
        '    <InstallationTarget Id="Microsoft.VisualStudio.Code"/>',
        "  </Installation>",
        "  <Dependencies/>",
        "  <Assets>",
    ]
    lines += [f"    <Asset Type={quoteattr(t)} Path={quoteattr(p)} Addressable=\"true\" />" for t, p in assets]
    lines += ["  </Assets>", "</PackageManifest>", ""]
    return "\n".join(lines)


def content_types(names):
    extensions = {".vsixmanifest"}
    for name in names:
        suffix = Path(name).suffix.lower()
        if suffix:
            extensions.add(suffix)
    defaults = "".join(
        f"<Default Extension={quoteattr(e)} ContentType="
        f"{quoteattr(CONTENT_TYPES.get(e) or mimetypes.types_map.get(e) or 'application/octet-stream')}/>"
        for e in sorted(extensions)
    )
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        f'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">{defaults}</Types>'
    )


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: pack-vsix.py EXTENSION_DIR OUT.vsix")
    ext, out = Path(sys.argv[1]), Path(sys.argv[2])
    package = json.loads((ext / "package.json").read_text(encoding="utf-8"))
    nls_file = ext / "package.nls.json"
    nls = json.loads(nls_file.read_text(encoding="utf-8")) if nls_file.is_file() else {}
    # Where each file goes in the package: the licence as LICENSE.txt.
    entries = {}
    for rel in files(ext):
        name = "LICENSE.txt" if rel in ("LICENSE", "LICENSE.md") else rel
        entries["extension/" + name] = ext / rel
    if package.get("icon") and "extension/" + package["icon"] not in entries:
        sys.exit(f"the icon {package['icon']} is not in the extension")
    tmp = out.with_suffix(".tmp")
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.writestr("extension.vsixmanifest", manifest(package, nls, entries))
        z.writestr("[Content_Types].xml", content_types(entries))
        for name, path in entries.items():
            z.write(path, name)
    tmp.replace(out)
    print(f"packed {len(entries)} files into {out} ({out.stat().st_size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
