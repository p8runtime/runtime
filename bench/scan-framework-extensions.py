#!/usr/bin/env python3
import json
import urllib.request
from collections import Counter, OrderedDict
from datetime import datetime
from pathlib import Path

frameworks = OrderedDict([
    ("Laravel", "https://raw.githubusercontent.com/laravel/framework/11.x/composer.json"),
    ("Symfony", "https://raw.githubusercontent.com/symfony/symfony/7.2/composer.json"),
    ("Magento", "https://raw.githubusercontent.com/magento/magento2/2.4-develop/composer.json"),
    ("Drupal", "https://raw.githubusercontent.com/drupal/drupal/10.3.x/composer.json"),
    ("WordPress", "https://raw.githubusercontent.com/WordPress/wordpress-develop/trunk/composer.json"),
])

results = {}
errors = {}

for name, url in frameworks.items():
    try:
        with urllib.request.urlopen(url, timeout=20) as resp:
            data = json.loads(resp.read().decode("utf-8"))
        req = data.get("require", {})
        ext = sorted(k.replace("ext-", "") for k in req.keys() if k.startswith("ext-"))
        results[name] = {
            "url": url,
            "php": req.get("php", ""),
            "extensions": ext,
        }
    except Exception as exc:
        errors[name] = f"{type(exc).__name__}: {exc}"

counter = Counter()
for info in results.values():
    counter.update(info["extensions"])

all_ext = sorted(counter.keys())

now = datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%SZ")

lines = []
lines.append("# PHP Extension Requirements (Framework Scan)")
lines.append("")
lines.append(f"Generated: {now}")
lines.append("")
lines.append("Sources: composer.json require ext-* keys (framework repos).")
lines.append("")

if errors:
    lines.append("## Fetch errors")
    lines.append("")
    for name, err in errors.items():
        lines.append(f"- {name}: {err}")
    lines.append("")

lines.append("## Per-framework requirements")
lines.append("")
for name, info in results.items():
    lines.append(f"### {name}")
    lines.append("")
    lines.append(f"Source: {info['url']}")
    if info["php"]:
        lines.append(f"PHP: {info['php']}")
    lines.append("Extensions:")
    if info["extensions"]:
        lines.append(", ".join(info["extensions"]))
    else:
        lines.append("(none found in composer.json)")
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

out_path = Path(__file__).resolve().parents[1] / "EXTENSIONS_FRAMEWORKS.md"
out_path.write_text("\n".join(lines), encoding="utf-8")
print(f"Wrote {out_path}")
