#!/usr/bin/env python3
"""Regenerate the checked-in example projects under examples/ from their
templates, so they cannot silently fall behind what the templates actually
produce.

Usage:
    scripts/regenerate-examples.py            # overwrite examples/ in place
    scripts/regenerate-examples.py --check     # diff against examples/, don't write

`--check` is what CI runs: it scaffolds each example into a temp directory
and diffs it against the checked-in tree, ignoring Cargo.lock (which is
resolved against live crates.io state at generation time, independent of any
template change) and target/ (build output, already gitignored). On a
mismatch it prints the example, the diff, and the exact command that
regenerates it.

hello-forge is intentionally excluded: it's the one example whose
regeneration steps (`soroban-forge test-init`, `soroban-forge ci-init`) are
documented and manually verified in examples/README.md, and it is not part of
this generate-and-diff loop.
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXAMPLES_DIR = ROOT / "examples"
BINARY = ROOT / "target" / "release" / "soroban-forge"
AUTHOR = "Joseph <josepholadele001@gmail.com>"

# (directory name under examples/, template name). Keep in sync with the
# table in examples/README.md.
EXAMPLES = [
    ("amm", "amm"),
    ("crowdfund", "crowdfund"),
    ("escrow", "escrow"),
    ("merkle-airdrop", "merkle-airdrop"),
    ("multisig", "multisig"),
    ("nft", "nft"),
    ("oracle-consumer", "oracle-consumer"),
    ("staking", "staking"),
    ("subscription", "subscription"),
    ("timelock", "timelock"),
    ("token", "token"),
    ("vesting", "vesting"),
]

IGNORED_NAMES = {"Cargo.lock", "target"}


def regen_command(name: str, template: str, output: str) -> list[str]:
    return [
        str(BINARY),
        "new",
        name,
        "--template",
        template,
        "--no-git",
        "--author",
        AUTHOR,
        "--output-dir",
        output,
        "--yes",
    ]


def regen_command_str(name: str, template: str) -> str:
    return (
        f"soroban-forge new {name} --template {template} --no-git "
        f'--author "{AUTHOR}" --output-dir examples --yes'
    )


def scaffold(name: str, template: str, output_dir: Path) -> None:
    subprocess.run(
        regen_command(name, template, str(output_dir)),
        check=True,
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    # Match the one round of tooling every example gets after scaffolding
    # (see examples/README.md): a resolved lockfile, checked in like
    # hello-forge's.
    subprocess.run(
        ["cargo", "generate-lockfile"],
        check=True,
        capture_output=True,
        text=True,
        cwd=output_dir / name,
    )


def tree_files(base: Path) -> set[str]:
    out = set()
    for p in base.rglob("*"):
        if not p.is_file():
            continue
        rel = p.relative_to(base)
        if any(part in IGNORED_NAMES for part in rel.parts):
            continue
        out.add(str(rel))
    return out


def diff_trees(expected: Path, actual: Path) -> list[str]:
    problems = []
    expected_files = tree_files(expected)
    actual_files = tree_files(actual)
    for missing in sorted(expected_files - actual_files):
        problems.append(f"  removed: {missing}")
    for extra in sorted(actual_files - expected_files):
        problems.append(f"  added:   {extra}")
    for common in sorted(expected_files & actual_files):
        if (expected / common).read_bytes() != (actual / common).read_bytes():
            problems.append(f"  changed: {common}")
    return problems


def main() -> int:
    check = "--check" in sys.argv
    if not BINARY.exists():
        sys.exit(
            f"error: {BINARY} not found — run `cargo build --release --bin soroban-forge` first"
        )

    if not check:
        for name, template in EXAMPLES:
            target = EXAMPLES_DIR / name
            shutil.rmtree(target, ignore_errors=True)
            scaffold(name, template, EXAMPLES_DIR)
            print(f"regenerated examples/{name}")
        return 0

    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        for name, template in EXAMPLES:
            try:
                scaffold(name, template, tmp_path)
            except subprocess.CalledProcessError as e:
                failures.append(
                    f"{name}: regeneration itself failed:\n{e.stdout}\n{e.stderr}\n"
                    f"  command: {regen_command_str(name, template)}"
                )
                continue
            diff = diff_trees(EXAMPLES_DIR / name, tmp_path / name)
            if diff:
                failures.append(
                    f"examples/{name} is out of date with the `{template}` template:\n"
                    + "\n".join(diff)
                    + f"\n  regenerate with: {regen_command_str(name, template)}"
                )

    if failures:
        print("\n\n".join(failures), file=sys.stderr)
        print(
            f"\n{len(failures)} example(s) out of date. Regenerate locally with:\n"
            "  scripts/regenerate-examples.py",
            file=sys.stderr,
        )
        return 1

    print(f"all {len(EXAMPLES)} examples match their templates")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
