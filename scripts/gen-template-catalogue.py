#!/usr/bin/env python3
"""Generate docs/template-catalogue.md from templates/ and the CLI's own
template list, so the page can never list a template that does not exist (or
omit one that does).

Usage:
    scripts/gen-template-catalogue.py [--check]

Without --check, (re)writes docs/template-catalogue.md. With --check, writes
to a temp file and diffs it against the checked-in page, exiting non-zero (and
naming this script as the fix) if they differ.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TEMPLATES_DIR = ROOT / "templates"
OUT_PATH = ROOT / "docs" / "template-catalogue.md"
BINARY = ROOT / "target" / "release" / "soroban-forge"

# Curated one-line guidance on when to reach for each template. Keyed by
# template name; every name in templates/ must appear here (enforced below).
WHEN_TO_PICK = {
    "access-control": "you need reusable grant/revoke/has-role primitives with an admin role, to gate entrypoints you add yourself",
    "allowlist-token": "regulatory or KYC'd transfers where only admin-approved addresses may hold or move the token",
    "amm": "a self-contained two-token constant-product pool, when you don't need a separate LP-share token contract",
    "atomic-swap": "trustless two-party token-for-token trades that must settle both legs or neither",
    "crowdfund": "deadline-based fundraising that must refund automatically if a goal is missed",
    "cross-contract": "learning or bootstrapping the caller/receiver pattern and how authorization propagates across a contract call",
    "dutch-auction": "a single-item sale that should clear quickly via a falling price instead of a bidding war",
    "english-auction": "**do not use yet** — this template is an unfinished stub (see note above); pick a different auction pattern for now",
    "escrow": "a single buyer/seller trade that needs an approval-or-timeout release path, without building a marketplace",
    "faucet": "testnet/devnet token distribution with a per-address cooldown, e.g. for demos or QA environments",
    "flash-loan": "uncollateralized same-transaction borrowing, or as a reference for writing a callback-repaid pool",
    "governance": "on-chain DAO decisions that need weighted voting, quorum, and automatic execution of the winning proposal",
    "hello-world": "your very first soroban-forge project, or a clean slate before writing real contract logic",
    "lottery": "on-chain randomness where no single party (including the drawer) should be able to bias the outcome",
    "merkle-airdrop": "distributing tokens to a large, fixed allowlist without storing every address on-chain",
    "multi-token": "one contract instance needs to manage many distinct token ids (ERC1155-style), not a single fungible/NFT asset",
    "multisig": "an account that should require M-of-N co-signers for every outgoing action",
    "nft": "a standalone non-fungible token with per-token metadata and admin-gated minting",
    "nft-marketplace": "listing and buying/selling NFTs (from an `nft`-shaped contract) with a protocol fee, without writing escrow logic yourself",
    "oracle-consumer": "pricing logic that needs a live external price feed, in a shape compatible with Reflector",
    "pausable": "a circuit breaker you can drop into any contract to freeze guarded entrypoints during an incident",
    "payment-splitter": "dividing incoming payments among fixed payees by share, with pull-based withdrawals",
    "prediction-market": "a binary-outcome market settled by a single trusted oracle, paid out parimutuel-style",
    "soulbound": "non-transferable tokens — reputation, credentials, or membership marks that must stay with the original holder",
    "staking": "reward-per-share style staking where depositors accrue a proportional share of periodically distributed rewards",
    "storage-migration": "learning or bootstrapping the version-marker pattern for safely migrating storage layouts across upgrades",
    "streaming": "continuous, claimable-anytime payment release over a fixed duration (vesting without a cliff)",
    "subscription": "recurring merchant billing against a payer's standing allowance, charged at most once per interval",
    "timelock": "any privileged action (upgrades, parameter changes) that must be queued and delayed before it can execute",
    "token": "a standard SEP-41 fungible token with no extra restrictions",
    "upgradeable": "learning or bootstrapping the minimal admin-gated `upgrade(new_wasm_hash)` pattern",
    "vesting": "token grants that unlock linearly after a cliff, e.g. team or investor allocations",
    "wrapped-asset": "a 1:1 wrapper around an existing SEP-41 token, e.g. to give it a different contract identity",
    "yield-vault": "ERC4626-style pooled yield with proportional shares, when share-inflation attacks must be rounded against depositors",
}

DOC_LINE = re.compile(r"^//!\s?(.*)$")


def read_module_doc(lib_rs: Path) -> str:
    if not lib_rs.exists():
        return ""
    lines = []
    for line in lib_rs.read_text().splitlines():
        m = DOC_LINE.match(line)
        if m is None:
            if lines:
                break
            continue
        text = m.group(1)
        if text.startswith("Generated by [soroban-forge]"):
            continue
        lines.append(text)
    return "\n".join(lines).strip()


def entrypoints(lib_rs: Path) -> list[str]:
    if not lib_rs.exists():
        return []
    names = []
    for line in lib_rs.read_text().splitlines():
        m = re.search(r"pub fn (\w+)\s*\(", line)
        if m:
            names.append(m.group(1))
    return names


NO_DESCRIPTION = "no description available"


def template_data(name: str, description: str) -> dict:
    tdir = TEMPLATES_DIR / name
    has_manifest = (tdir / "template.toml").exists()

    if name == "cross-contract":
        doc = (
            read_module_doc(tdir / "caller" / "src" / "lib.rs")
            + "\n\n"
            + read_module_doc(tdir / "receiver" / "src" / "lib.rs")
        ).strip()
        caller_eps = entrypoints(tdir / "caller" / "src" / "lib.rs")
        receiver_eps = entrypoints(tdir / "receiver" / "src" / "lib.rs")
        eps = [f"caller::{e}" for e in caller_eps if e != "__constructor"] + [
            f"receiver::{e}" for e in receiver_eps if e != "__constructor"
        ]
        has_source = True
    else:
        lib_rs = tdir / "src" / "lib.rs"
        doc = read_module_doc(lib_rs)
        eps = [e for e in entrypoints(lib_rs) if e != "__constructor"]
        has_source = lib_rs.exists()

    if description == NO_DESCRIPTION and doc:
        # template.toml is missing, but there's real source with its own doc
        # comment — use that instead of the CLI's placeholder for the summary
        # row, while the per-section callout still flags the missing manifest.
        first_para = doc.split("\n\n")[0].replace("\n", " ")
        description = first_para.split(". ")[0].rstrip(".").lower()

    return {
        "name": name,
        "description": description,
        "doc": doc,
        "entrypoints": eps,
        "has_manifest": has_manifest,
        "has_source": has_source,
    }


def load_cli_templates() -> list[dict]:
    if not BINARY.exists():
        sys.exit(
            f"error: {BINARY} not found — run `cargo build --release --bin soroban-forge` first "
            "(this script reads the template list from the built CLI, not a hardcoded copy)"
        )
    out = subprocess.run(
        [str(BINARY), "new", "--list-templates", "--json"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def render(templates: list[dict]) -> str:
    lines = [
        "# Template catalogue",
        "",
        "Every template bundled under [`templates/`](../templates), generated from"
        " the CLI's own template list (`soroban-forge new --list-templates --json`)"
        " so this page cannot drift from what `soroban-forge new` actually offers.",
        "Regenerate it after adding or changing a template:",
        "",
        "```sh",
        "cargo build --release --bin soroban-forge",
        "scripts/gen-template-catalogue.py",
        "```",
        "",
        "| template | what it demonstrates |",
        "|----------|----------------------|",
    ]
    for t in templates:
        summary = t["description"]
        lines.append(f"| [`{t['name']}`](#{t['name']}) | {summary} |")
    lines.append("")

    for t in templates:
        lines.append(f"## {t['name']}")
        lines.append("")
        if not t["has_source"]:
            lines.append(
                f"> **Incomplete template.** `templates/{t['name']}/` has no `src/` and no "
                "`template.toml` — `soroban-forge new` will scaffold it and report success, "
                "but the result has no contract to build or test. Tracked as a known gap; "
                "do not use this template until it has real source."
            )
            lines.append("")
        elif not t["has_manifest"]:
            lines.append(
                f"> `templates/{t['name']}/` has no `template.toml` manifest — variables and "
                "post-generate hints for it, if any, are not declared."
            )
            lines.append("")
        if t["doc"]:
            lines.append(t["doc"])
            lines.append("")
        elif t["has_source"]:
            lines.append(t["description"])
            lines.append("")
        if t["entrypoints"]:
            lines.append("**Entrypoints:** " + ", ".join(f"`{e}`" for e in t["entrypoints"]))
            lines.append("")
        when = WHEN_TO_PICK.get(t["name"])
        if when:
            lines.append(f"**When to pick it:** {when}")
            lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    check = "--check" in sys.argv
    cli_templates = load_cli_templates()

    missing_guidance = [t["name"] for t in cli_templates if t["name"] not in WHEN_TO_PICK]
    if missing_guidance:
        sys.exit(
            "error: no WHEN_TO_PICK entry for: "
            + ", ".join(missing_guidance)
            + " — add one in scripts/gen-template-catalogue.py"
        )

    templates = [template_data(t["name"], t["description"]) for t in cli_templates]
    templates.sort(key=lambda t: t["name"])
    rendered = render(templates)

    if check:
        current = OUT_PATH.read_text() if OUT_PATH.exists() else ""
        if current != rendered:
            print(
                "docs/template-catalogue.md is out of date.\n"
                "Regenerate it with: scripts/gen-template-catalogue.py",
                file=sys.stderr,
            )
            return 1
        return 0

    OUT_PATH.write_text(rendered)
    print(f"wrote {OUT_PATH.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
