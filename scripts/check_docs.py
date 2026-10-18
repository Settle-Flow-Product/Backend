#!/usr/bin/env python3
"""Check that the documentation and the code still agree.

Every claim below was checked by hand once. This file is what stops it drifting: an error code
renumbered in Rust, an endpoint documented but never routed, a test cited in a table and never
written. All of those look fine in review and are wrong the moment somebody relies on them.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
failures: list[str] = []


def fail(msg: str) -> None:
    failures.append(msg)


# --- every relative link resolves -----------------------------------------------------------
checked = 0
for md in sorted(ROOT.rglob("*.md")):
    if any(p in md.parts for p in ("node_modules", "target", ".next", ".git")):
        continue
    for m in re.finditer(r"\[([^\]]*)\]\(([^)]+)\)", md.read_text(errors="replace")):
        link = m.group(2).strip()
        if link.startswith(("http://", "https://", "#", "mailto:")):
            continue
        target = link.split("#")[0]
        if not target:
            continue
        checked += 1
        if not (md.parent / target).resolve().exists():
            fail(f"broken link in {md.relative_to(ROOT)}: {link}")

# --- the error table is the same in Rust and in the docs -------------------------------------
src = (ROOT / "smart-contract/src/errors.rs").read_text()
in_code = {m.group(1): int(m.group(2)) for m in re.finditer(r"^\s{4}(\w+) = (\d+),", src, re.M)}
doc = (ROOT / "docs/04-contract-interface.md").read_text()
in_doc = {
    m.group(2): int(m.group(1))
    for m in re.finditer(r"^\|[^|]*\|\s*(\d+)\s*\|\s*`(\w+)`\s*\|", doc, re.M)
}
if in_code != in_doc:
    fail(f"error table drift. code={in_code} doc={in_doc}")
if len(in_code) != 12:
    fail(f"module A1 specifies twelve error codes; errors.rs has {len(in_code)}")

# --- every documented endpoint is actually routed ---------------------------------------------
http = (ROOT / "backend/crates/settle-backend/src/http.rs").read_text()
routes = set(re.findall(r'\.route\("([^"]+)"', http))
api = (ROOT / "docs/09-api-contract.md").read_text()
documented = {
    m.group(2).replace(":id", "{id}")
    for m in re.finditer(r"\|\s*`(GET|POST)`\s*\|\s*`([^`]+)`", api)
}
if documented - routes:
    fail(f"documented but not routed: {sorted(documented - routes)}")

# --- every identifier a test table cites exists somewhere -------------------------------------
sources = []
for pat in (
    "smart-contract/src/**/*.rs",
    "backend/crates/**/*.rs",
    "backend/scripts/*.sh",
    "smart-contract/scripts/*.sh",
    "scripts/*.sh",
    "frontend/website/lib/*.ts",
    "backend/migrations/*.sql",
):
    sources.extend(ROOT.glob(pat))
blob = "\n".join(p.read_text(errors="replace") for p in sources)

# The doc names one BAD example on purpose, to say what not to call a test.
COUNTER_EXAMPLES = {"test_mark_paid_2"}
cited = sorted(set(re.findall(r"`([a-z][a-z0-9_]{10,})`", (ROOT / "docs/12-testing.md").read_text())))
absent = [c for c in cited if c not in blob and c not in COUNTER_EXAMPLES]
if absent:
    fail(f"cited in docs/12-testing.md but nowhere in the tree: {absent}")

# --- migrations pair up ------------------------------------------------------------------------
ups = sorted(p.name[: -len(".up.sql")] for p in (ROOT / "backend/migrations").glob("*.up.sql"))
for u in ups:
    if not (ROOT / f"backend/migrations/{u}.down.sql").exists():
        fail(f"{u} has no down migration")

# --- the translation mirrors the English, banner and all ---------------------------------------
en = {p.relative_to(ROOT / "docs") for p in (ROOT / "docs").rglob("*.md") if "translate" not in p.parts}
vi = {p.relative_to(ROOT / "docs/translate/vi") for p in (ROOT / "docs/translate/vi").rglob("*.md")}
if en != vi:
    fail(f"translation gap. missing={sorted(en - vi)} extra={sorted(vi - en)}")
for p in (ROOT / "docs/translate/vi").rglob("*.md"):
    if "bản có hiệu lực" not in p.read_text():
        fail(f"{p.relative_to(ROOT)} has no precedence banner")

# --- counts the docs assert about themselves ---------------------------------------------------
mods = (ROOT / "docs/02-modules.md").read_text()
rows = len(re.findall(r"\| \*\*[A-G]\d\*\* \|", mods))
claimed = re.search(r"The official work list: (\d+) modules", mods)
if not claimed or int(claimed.group(1)) != rows:
    fail(f"docs/02-modules.md claims {claimed and claimed.group(1)} modules but the tables have {rows}")

feats = (ROOT / "docs/01-features.md").read_text()
frows = len(re.findall(r"\| \*\*F-\d\d\*\* \|", feats))
if frows != 13:
    fail(f"docs/01-features.md should list 13 features, has {frows}")

# --- the sixteen mandatory contract cases all exist ---------------------------------------------
contract_tests = "\n".join(p.read_text() for p in (ROOT / "smart-contract/src/test").glob("*.rs"))
mandatory = re.findall(r"^\| \d+ \| `(\w+)` \|", (ROOT / "docs/12-testing.md").read_text(), re.M)
if len(mandatory) != 16:
    fail(f"docs/12-testing.md should name 16 mandatory cases, names {len(mandatory)}")
for t in mandatory:
    if f"fn {t}(" not in contract_tests:
        fail(f"mandatory case `{t}` is documented but not implemented")

# --- a chapter's title matches its filename ------------------------------------------------------
#
# Renumbering rewrites links, because links contain the filename. It does not touch the `# 11 ·`
# heading inside the file, so the two drift apart silently and every chapter after the insertion
# point is titled one behind. Both language trees, and both indexes, are checked here.
for chapter in sorted(ROOT.glob("docs/*.md")) + sorted(ROOT.glob("docs/translate/vi/*.md")):
    number = re.match(r"(\d\d)-", chapter.name)
    if not number:
        continue
    heading = chapter.read_text().split("\n", 1)[0]
    titled = re.match(r"# (\d+) · ", heading)
    if not titled:
        fail(f"{chapter.relative_to(ROOT)} does not open with a `# NN · Title` heading")
    elif titled.group(1) != number.group(1):
        fail(
            f"{chapter.relative_to(ROOT)} is titled '{titled.group(1)} ·' — the filename says "
            f"{number.group(1)}"
        )

# --- the reading order lists every chapter, in order, under its own number ----------------------
for index in ("docs/README.md", "docs/translate/vi/README.md"):
    text = (ROOT / index).read_text()
    listed = re.findall(r"^\| (\d\d) \| \[(\d\d)-[^\]]+\]", text, re.M)
    if not listed:
        fail(f"{index} has no reading-order table this check can read")
        continue
    for shown, filename in listed:
        if shown != filename:
            fail(f"{index} lists {filename}-* in row {shown}")
    tree = "docs" if index == "docs/README.md" else "docs/translate/vi"
    on_disk = {m.group(1) for p in (ROOT / tree).glob("*.md")
               if (m := re.match(r"(\d\d)-", p.name))}
    missing = sorted(on_disk - {n for n, _ in listed})
    if missing:
        fail(f"{index} omits chapter(s) {', '.join(missing)}")

# ------------------------------------------------------------------------------------------------
if failures:
    print(f"{len(failures)} inconsistency(ies):")
    for f in failures:
        print(f"  - {f}")
    sys.exit(1)

print(f"docs and code agree ({checked} links, {len(in_code)} error codes, {len(mandatory)} mandatory cases)")
