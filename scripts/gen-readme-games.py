#!/usr/bin/env python3
"""Rewrite the README's Supported Games tables from plugins/app/*/sandbox.json.

    scripts/gen-readme-games.py          # rewrite README.md
    scripts/gen-readme-games.py --check  # exit 1 if README.md is stale
"""
import glob, json, os, sys
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BEGIN, END = "<!-- games:begin -->", "<!-- games:end -->"
groups = {"BepInEx": [], "Loader and other plugin folders": [], "Mod folders": []}
for f in sorted(glob.glob(os.path.join(root, "plugins/app/*/sandbox.json"))):
    s = json.load(open(f))
    name = s["label"].removesuffix(" sandboxes")
    t = s["sandbox"]["deploy"]["modTargets"]
    paths = ", ".join("`%s/`" % x["relPath"].rstrip("/") if x["relPath"] not in ("", ".") else "game root" for x in t[:3])
    first = t[0]
    if first["type"] == "loader_plugin" and first["relPath"].startswith("BepInEx"):
        k = "BepInEx"
    elif first["type"].startswith(("loader", "plugin", "native", "modloader")):
        k = "Loader and other plugin folders"
    else:
        k = "Mod folders"
    groups[k].append((name, paths))
out = [BEGIN]
for k, rows in groups.items():
    out += ["", "<details><summary><b>%s</b> (%d games)</summary>" % (k, len(rows)), "",
            "| Game | Mods land in | Tested |", "| --- | --- | :-: |"]
    out += ["| %s | %s | **No** |" % r for r in sorted(rows, key=lambda r: r[0].lower())]
    out += ["", "</details>"]
out += ["", END]
text = "\n".join(out)
p = os.path.join(root, "README.md")
r = open(p).read()
a, b = r.index(BEGIN), r.index(END) + len(END)
new = r[:a] + text + r[b:]
if "--check" in sys.argv:
    sys.exit(0 if new == r else 1)
open(p, "w").write(new)
