"""Write the packaged text notices and the offline Settings catalog from the same inputs."""
import json
from pathlib import Path
import subprocess
import sys
from urllib.parse import urlsplit
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo


def output(*args):
    return subprocess.check_output(args, text=True)


def generate(destination):
    about = json.loads(output("cargo", "about", "generate", "--fail", "--locked", "--workspace",
                              "-c", "packaging/licenses/about.toml", "--format", "json"))
    packages = json.loads(output("cargo", "metadata", "--format-version", "1", "--locked"))["packages"]
    crate_dirs = {p["name"]: Path(p["manifest_path"]).parent for p in packages}
    texts, components = {}, {}

    def record(name, version, license_id, bundle, source, text):
        key = (name, version, bundle)
        entry = components.setdefault(key, dict(name=name, version=version, bundle=bundle,
                                                 source=None, notices=[]))
        if source and urlsplit(source).scheme in {"http", "https"}:
            entry["source"] = source
        index = texts.setdefault(text, len(texts))
        notice = dict(license=license_id, text=index)
        if notice not in entry["notices"]:
            entry["notices"].append(notice)

    sections = ["Third-party software in Bruhs\n=============================\n\n"
                "Bruhs is MIT licensed. It includes the open source software below; each\n"
                "entry lists the components under one license, followed by that license.\n\nRust crates\n-----------\n"]
    divider = "\n" + "-" * 74 + "\n"
    for group in about["licenses"]:
        used_by = []
        for use in group["used_by"]:
            crate = use["crate"]
            name, version, source = crate["name"], crate["version"], crate["repository"]
            used_by.append(f"  - {name} {version}" + (f" <{source}>" if source else ""))
            record(name, version, group["id"], "Rust", source, group["text"])
        sections.append(f"{divider}{group['name']} ({group['id']})\nUsed by:\n" +
                        "\n".join(used_by) + "\n\n" + group["text"] + "\n")

    def section(name, license_id, bundle, path, source=None, version=None):
        text = Path(path).read_text()
        record(name, version, license_id, bundle, source, text)
        sections.append(f"{divider}{name}" + (f" {version}" if version else "") +
                        f" ({license_id}), {bundle}\n\n{text}\n")

    sections.append("\nBundled libraries, scripts and fonts\n------------------------------------\n")
    section("GPUI and its Zed crates", "Apache-2.0", "Desktop", crate_dirs["gpui"] / "LICENSE-APACHE",
            "https://github.com/zed-industries/zed")
    section("whisper.cpp", "MIT", "Capture", crate_dirs["whisper-rs-sys"] / "whisper.cpp/LICENSE",
            "https://github.com/ggml-org/whisper.cpp")
    section("sherpa-onnx", "Apache-2.0", "Capture", crate_dirs["sherpa-rs-sys"] / "sherpa-onnx/LICENSE",
            "https://github.com/k2-fsa/sherpa-onnx")
    section("ONNX Runtime", "MIT", "Capture", "packaging/licenses/onnxruntime-LICENSE",
            "https://github.com/microsoft/onnxruntime")
    web = {
        "addon-fit": ("MIT", "https://github.com/xtermjs/xterm.js"),
        "xterm": ("MIT", "https://github.com/xtermjs/xterm.js"),
    }
    for file in sorted(Path("bruhs-web/vendor/lib").glob("*/LICENSE")):
        name, _, version = file.parent.name.rpartition("-")
        license_id, source = web.get(name, ("License notice", None))
        section(name, license_id, "Web", file, source, version)

    sections.append("\nBundled composer syntax definitions\n------------------------------------\n")
    syntaxes = json.loads(output("cargo", "run", "--quiet", "--locked", "-p", "bruhs-ui",
                                 "--example", "syntax_notices", "--", "--json"))
    syntax_ids = {"Mit": "MIT", "Bsd2Clause": "BSD-2-Clause", "Bsd2ClauseFreeBsd": "BSD-2-Clause-FreeBSD",
                  "Unlicense": "Unlicense", "Bsd3Clause": "BSD-3-Clause", "Apache2": "Apache-2.0",
                  "Wtfpl": "WTFPL", "Sublime": "Sublime"}
    for syntax in syntaxes:
        name, text = syntax["rel_path"], syntax["text"]
        record(name, None, syntax_ids.get(syntax["ty"], syntax["ty"]), "Composer syntaxes", None, text)
        sections.append(f"<details>\n<summary>{name}</summary>\n\n````text\n{text}\n````\n</details>\n\n")

    section("Bruhs design tokens", "MIT", "Web", "bruhs-web/vendor/BRUHS-LICENSE")
    section("Lucide icons", "ISC / MIT", "Assets", "bruhs-ui/assets/icons/LICENSE-LUCIDE")
    section("Monaspace fonts", "OFL-1.1", "Assets", "bruhs-desktop/assets/fonts/LICENSE",
            "https://github.com/githubnext/monaspace")

    entries = sorted(components.values(), key=lambda e: (e["name"].casefold(), e["version"] or "", e["bundle"]))
    catalog = dict(entries=entries, texts=list(texts))
    destination.write_text("".join(sections))
    with ZipFile(str(destination) + ".zip", "w") as archive:
        archive.writestr(ZipInfo("notices.json"), json.dumps(catalog, ensure_ascii=False,
                         separators=(",", ":")).encode(), compress_type=ZIP_DEFLATED, compresslevel=9)
    print(f"Wrote {len(entries)} entries to {destination} and its Settings catalog")


if __name__ == "__main__":
    generate(Path(sys.argv[1]))
