#!/usr/bin/env python3
import json
import urllib.request
from collections import Counter
from datetime import datetime
from pathlib import Path

TOP_N = int((__import__("os").environ.get("TOP_N") or "100"))

POPULAR_URL = "https://packagist.org/explore/popular.json"
PACKAGE_URL = "https://packagist.org/packages/{name}.json"

errors = []
package_ext = {}

try:
    with urllib.request.urlopen(POPULAR_URL, timeout=20) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    names = [item["name"] for item in data.get("packages", [])][:TOP_N]
except Exception as exc:
    errors.append(f"popular list: {type(exc).__name__}: {exc}")
    names = []

for name in names:
    try:
        with urllib.request.urlopen(PACKAGE_URL.format(name=name), timeout=20) as resp:
            data = json.loads(resp.read().decode("utf-8"))
        pkg = data.get("package", {})
        versions = pkg.get("versions", {})
        if not versions:
            package_ext[name] = []
            continue
        # pick the first stable-ish version
        version = None
        for ver in versions.keys():
            if "dev" not in ver:
                version = ver
                break
        if version is None:
            version = next(iter(versions.keys()))
        req = versions.get(version, {}).get("require", {})
        ext = sorted(k.replace("ext-", "") for k in req.keys() if k.startswith("ext-"))
        package_ext[name] = ext
    except Exception as exc:
        errors.append(f"{name}: {type(exc).__name__}: {exc}")

counter = Counter()
for ext_list in package_ext.values():
    counter.update(ext_list)

all_ext = sorted(counter.keys())

now = datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%SZ")

lines = []
lines.append("# PHP Extension Requirements (Top Composer Packages)")
lines.append("")
lines.append(f"Generated: {now}")
lines.append(f"Top N: {TOP_N}")
lines.append("")
lines.append("Sources: packagist popular list + package.json require ext-* keys.")
lines.append("")

if errors:
    lines.append("## Fetch errors")
    lines.append("")
    for err in errors:
        lines.append(f"- {err}")
    lines.append("")

lines.append("## Aggregate (union)")
lines.append("")
lines.append(", ".join(all_ext) if all_ext else "(none)")
lines.append("")

lines.append("## Frequency")
lines.append("")
lines.append("| Extension | Count |")
lines.append("| --- | --- |")
for ext in sorted(counter.keys()):
    lines.append(f"| {ext} | {counter[ext]} |")
lines.append("")

lines.append("## Per-package (extensions)")
lines.append("")
for name in sorted(package_ext.keys()):
    ext = package_ext[name]
    lines.append(f"- {name}: {', '.join(ext) if ext else '(none)'}")

out_path = Path(__file__).resolve().parents[1] / "EXTENSIONS_PACKAGES.md"
out_path.write_text("\n".join(lines), encoding="utf-8")
print(f"Wrote {out_path}")
