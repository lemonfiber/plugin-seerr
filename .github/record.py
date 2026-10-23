#!/usr/bin/env python3
"""Record every conformance case of the contract the adapter speaks.

**This is CI harness, not plugin content.** Nothing under `.github/` is
installed, and lemonfiber never runs any of it (`F3-R6`).

Each recording under `conformance/<capability>@<major>/<case>.json` is the
adapter's answer to one case of that contract's `conformance.json` (ARCH-R217),
asked with the case's own `asked`, in the order the contract lists them. It is
taken against a throwaway Seerr at the digest `plugin.toml` pins for the
plugin's own service, on a network with no route off the host, so nothing it
answers depends on the metadata provider. Seerr answers its key only once it
holds an owner, and its owner is made by the first sign-in through an identity
source, so the throwaway Seerr is given the owner that sign-in makes, and the
member the contract's cases ask about, before the adapter starts. The adapter runs from this repository's Dockerfile, as
lemonfiber runs it: the caller's uid, read-only, no capabilities, the key and
the upstream credential owner-only in its configuration directory. This script
stands where lemonfiber stands: it reads the key Seerr wrote to `main.apiKey`
in `settings.json` on its first start, writes it to the adapter's
`upstream.json` beside `lemonfiber.key`, and asks.

    python3 .github/record.py            write conformance/
    python3 .github/record.py --check    record again and compare

Needs Docker. Exit 0 = recorded (or matched), 1 = it did not.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import pathlib
import re
import secrets
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
RECORDINGS = ROOT / "conformance"
CONTRACTS = "https://raw.githubusercontent.com/lemonfiber/lemonfiber/{rev}/contract/capabilities/{capability}/v{major}/conformance.json"
JSON = "application/json"
STARTUP_S = 180
ANSWER_S = 30
ADAPTER_PORT = 8080
SEGMENT = re.compile(r"[a-z][a-z0-9_.]*")
MAJOR = re.compile(r"[1-9]\d*")
API_KEY = "api_key"
SETTINGS = "settings.json"
DATABASE = pathlib.Path("db") / "db.sqlite3"
CONFIG = "/app/config"
STATUS = "/api/v1/status"
OWNER = ("owner@lemonfiber.local", "owner", 2, "/avatarproxy/owner", 3, "owner", "0" * 32)
MEMBER = ("member@lemonfiber.local", "member", 32, "/avatarproxy/member", 3, "member")
INTAKE = "request.intake@1"
OWNED = ('INSERT INTO "user" ("email", "username", "permissions", "avatar", "userType", '
         '"jellyfinUsername", "jellyfinUserId") VALUES (?, ?, ?, ?, ?, ?, ?)')
REDACTED = "<redacted>"
SCRUBBED_WITHIN = 8
PLACEHOLDER = re.compile(r"\{[^{}]+\}")
NOTES = {
    "refuses-without-the-key": "Asked without the plugin's key: the kit refuses it before the adapter is reached.",
    "establish-answers": "Seerr already answers its key as its owner, so there is nothing to establish.",
    "initialized-answers": "A Seerr holding its owner and never set up: not initialised.",
    "configure_identity-answers": "An identity source Seerr cannot sign in through: refused before Seerr is asked.",
    "answers-answers": "Seerr answers its own key.",
    "requests-answers": "A fresh Seerr holds no request.",
    "link_members-answers": "Seerr reaches no identity source to import members from: refused.",
    "member_for-answers": "The account Seerr holds for the member the case names.",
    "requesting-answers": "What that member may ask for: not their own approval.",
    "approval_first-answers": "That member's asks made to wait for approval.",
    "remove_member-answers": "Seerr holds no account numbered 0: refused.",
    "telling-answers": "What a fresh Seerr tells the household through the browser: nothing.",
    "tell-answers": "Browser notices switched on for what arrives.",
    "add_fulfilment_target-answers": "The television curator added to Seerr's Sonarr list.",
    "fulfilment_targets-answers": "The television curator just added, as Seerr lists it.",
    "move_fulfilment_target-answers": "The television curator moved in place under the id Seerr listed it as.",
    "test_fulfilment_target-answers": "Seerr reaches no curator on a network with no route to one: refused.",
    "media_server_link-answers": "Where a fresh Seerr reaches the identity source: nowhere, with no key.",
    "link_media_server-answers": "Seerr refuses an identity source it cannot reach.",
    "asking-answers": "What a fresh Seerr lets the household ask: approval first and no quota.",
    "set_asking-answers": "Approval first and five requests a week written as the household's asking.",
    "left-answers": "What that member has left to ask for under the household's quota.",
    "set_quota-answers": "Five requests a week written as that member's quota, the settings Seerr never held left unsent.",
    "approves_own-answers": "That member made to wait for approval of their own asks.",
    "decide-answers": "Seerr holds no request numbered 0: refused.",
    "hold_requests-answers": "That member's asking taken away, and what was taken said.",
    "release_requests-answers": "Nothing given back, as the case holds nothing taken.",
    "reachable-answers": "Seerr holds no request numbered 0: refused.",
    "set_notices-answers": "The household's notice written to Seerr's discover page.",
    "search-answers": "The metadata provider is out of reach: no titles.",
    "detail-answers": "The metadata provider is out of reach: refused.",
    "ask-answers": "An ask by that member, whose asking was taken away: Seerr forbids it, and the adapter refuses it.",
}


def docker(*args: str, check: bool = True) -> str:
    done = subprocess.run(["docker", *args], capture_output=True, text=True, check=False)
    if check and done.returncode != 0:
        raise RuntimeError(f"docker {' '.join(args)}: {done.stderr.strip()}")
    return done.stdout.strip()


def manifest() -> tuple[str, str, int]:
    """The plugin's own service: its id, its pinned image and the port it listens on."""
    read = tomllib.loads((ROOT / "plugin.toml").read_text(encoding="utf-8"))
    plugin = read["plugin"]["id"]
    upstream = next(one for one in read["service"] if one["id"] == plugin)
    return plugin, f"{upstream['image']}@{upstream['digest']}", upstream["listens"]


def core_rev() -> str:
    read = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    return read["dependencies"]["lemonfiber-adapter-kit"]["rev"]


def published_cases(spoken: str) -> list[dict]:
    capability, major = spoken.split("@")
    url = CONTRACTS.format(rev=core_rev(), capability=capability, major=major)
    with urllib.request.urlopen(url, timeout=ANSWER_S) as answer:
        return json.load(answer)["cases"]


def asked(base: str, path: str, key: str | None, body: dict | None = None) -> tuple[int, str, str]:
    request = urllib.request.Request(
        f"{base}{path}",
        data=None if body is None else json.dumps(body).encode(),
        method="GET" if body is None else "POST",
        headers={"Content-Type": JSON, "Accept": JSON} | ({"Authorization": f"Bearer {key}"} if key else {}),
    )
    try:
        with urllib.request.urlopen(request, timeout=ANSWER_S) as answer:
            return answer.status, answer.headers.get("content-type", ""), answer.read().decode()
    except urllib.error.HTTPError as refused:
        return refused.code, refused.headers.get("content-type", ""), refused.read().decode()


def recording(pinned: str, path: str, status: int, kind: str, body: str, note: str) -> dict:
    response: dict = {"status": status}
    if kind:
        response["headers"] = {"content-type": kind}
    if body.strip() == "null":
        response["body_starts_with"] = "null"
    elif body.strip():
        response["json"] = json.loads(body)
    return {
        "recorded_from": pinned,
        "note": note,
        "request": {"method": "POST", "path": path, "accept": JSON},
        "response": response,
    }


def generated_key(settings: pathlib.Path) -> str | None:
    """The key Seerr wrote to `main.apiKey` on its first start, as lemonfiber reads it."""
    try:
        held = json.loads(settings.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    key = held.get("main", {}).get("apiKey") if isinstance(held, dict) else None
    return key if isinstance(key, str) and key else None


def member_id() -> str:
    """The identity-source member the contract's cases link and ask about."""
    return next(case["asked"]["media_server_id"] for case in published_cases(INTAKE)
                if case["operation"] == "member_for")


def owned(database: pathlib.Path) -> None:
    """The owner the first sign-in through an identity source makes, and the member the
    contract's cases ask about, each held as Seerr holds an account it imported."""
    with contextlib.closing(sqlite3.connect(database)) as held, held:
        held.execute(OWNED, OWNER)
        held.execute(OWNED, (*MEMBER, member_id()))


def answers_status(container: str, listens: int) -> bool:
    asked_inside = ["docker", "exec", container, "wget", "-q", "-O", "/dev/null",
                    f"http://localhost:{listens}{STATUS}"]
    return subprocess.run(asked_inside, capture_output=True, check=False).returncode == 0


def owner_only(directory: pathlib.Path) -> None:
    for file in directory.iterdir():
        file.chmod(0o600)
    directory.chmod(0o700)


def waited(what: str, ready) -> object:
    deadline = time.monotonic() + STARTUP_S
    while time.monotonic() < deadline:
        found = ready()
        if found:
            return found
        time.sleep(2)
    raise RuntimeError(f"{what} within {STARTUP_S}s")


def recorded_live(plugin: str, pinned: str, listens: int) -> tuple[list[str], dict[tuple[str, str], dict | str]]:
    """What the adapter speaks, and every case asked of it in front of a throwaway Seerr."""
    name = f"{plugin}-record-{uuid.uuid4().hex[:8]}"
    inside, outside = f"{name}-inside", f"{name}-outside"
    key = secrets.token_hex(24)
    with tempfile.TemporaryDirectory() as held:
        held = pathlib.Path(held)
        upstream, adapter = held / "upstream", held / "adapter"
        for directory in (upstream, adapter):
            directory.mkdir()
        try:
            docker("network", "create", "--internal", inside)
            docker("network", "create", outside)
            docker("run", "-d", "--name", f"{name}-upstream", "--network", inside,
                   "--network-alias", plugin, "--user", f"{os.getuid()}:{os.getgid()}",
                   "-e", "TZ=Etc/UTC", "-v", f"{upstream}:{CONFIG}", pinned)
            api_key = waited("Seerr wrote no key", lambda: generated_key(upstream / SETTINGS))
            waited("Seerr did not answer", lambda: answers_status(f"{name}-upstream", listens))
            docker("stop", f"{name}-upstream")
            owned(upstream / DATABASE)
            docker("start", f"{name}-upstream")
            waited("Seerr did not answer again", lambda: answers_status(f"{name}-upstream", listens))
            (adapter / "lemonfiber.key").write_text(key, encoding="utf-8")
            (adapter / "upstream.json").write_text(json.dumps({API_KEY: api_key}), encoding="utf-8")
            owner_only(adapter)
            docker("build", "-q", "-t", f"{name}:adapter", str(ROOT))
            docker("create", "--name", f"{name}-adapter", "--network", outside,
                   "--user", f"{os.getuid()}:{os.getgid()}", "--read-only", "--cap-drop", "ALL",
                   "--security-opt", "no-new-privileges", "-v", f"{adapter}:/config:ro",
                   "-p", f"127.0.0.1::{ADAPTER_PORT}", f"{name}:adapter")
            docker("network", "connect", inside, f"{name}-adapter")
            docker("start", f"{name}-adapter")
            port = docker("port", f"{name}-adapter", str(ADAPTER_PORT)).splitlines()[0].rsplit(":", 1)[1]
            base = f"http://127.0.0.1:{port}"
            waited("the adapter was not ready", lambda: answering(base, key))
            speaks = json.loads(asked(base, "/lemonfiber/adapter/v1/about", key)[2])["speaks"]
            said, made = recorded(base, key, pinned, speaks, adapter / "upstream.json")
            leaked = [case for case, one in said.items() if isinstance(one, dict)
                      if any(secret in json.dumps(one) for secret in (api_key, key, *made)
                             if len(secret) >= SCRUBBED_WITHIN)]
            if leaked:
                raise RuntimeError(f"an answer holds a credential, so nothing is written: {leaked}")
            return speaks, said
        finally:
            for container in (f"{name}-adapter", f"{name}-upstream"):
                docker("rm", "-fv", container, check=False)
            docker("rmi", f"{name}:adapter", check=False)
            for network in (inside, outside):
                docker("network", "rm", network, check=False)


def answering(base: str, key: str) -> bool:
    try:
        status, _, body = asked(base, "/lemonfiber/adapter/v1/ready", key)
    except (urllib.error.URLError, ConnectionError):
        return False
    return status == 200 and body.strip() == "true"


class Filling:
    """The placeholders a case's `asked` may hold, filled as the contract says, and every
    value filled kept so no recording holds one."""

    def __init__(self, upstream: pathlib.Path) -> None:
        self.upstream = upstream
        self.answers: dict[str, object] = {}
        self.made: list[str] = []
        self.pending: dict[str, str] = {}

    def filled(self, value):
        if isinstance(value, dict):
            return {name: self.filled(inner) for name, inner in value.items()}
        if isinstance(value, list):
            return [self.filled(inner) for inner in value]
        if not isinstance(value, str) or not PLACEHOLDER.fullmatch(value):
            return value
        named = value[1:-1]
        if named == "minted":
            return self.minted()
        held = json.loads(self.upstream.read_text(encoding="utf-8"))
        if named.startswith("minted:upstream:"):
            field = named.removeprefix("minted:upstream:")
            self.pending[field] = self.minted()
            return self.pending[field]
        if named.startswith("upstream:"):
            field = named.removeprefix("upstream:")
            if not isinstance(held.get(field), str):
                raise RuntimeError(f"{value} names nothing upstream.json holds")
            self.made.append(held[field])
            return held[field]
        case, _, pointer = named.partition("#")
        found = pointed(self.answers.get(case), pointer)
        if not isinstance(found, str):
            raise RuntimeError(f"{value} names no string an earlier case answered")
        self.made.append(found)
        return found

    def minted(self) -> str:
        made = secrets.token_hex(16)
        self.made.append(made)
        return made

    def answered(self, case: str, status: int, body: str) -> None:
        try:
            self.answers[case] = json.loads(body) if body.strip() else None
        except json.JSONDecodeError:
            self.answers[case] = None
        if 200 <= status < 300 and self.pending:
            held = json.loads(self.upstream.read_text(encoding="utf-8"))
            held.update(self.pending)
            self.upstream.write_text(json.dumps(held), encoding="utf-8")
        self.pending = {}


def pointed(value, pointer: str):
    """What the JSON pointer `pointer` names in `value`, or nothing."""
    for part in pointer.split("/")[1:] if pointer else []:
        part = part.replace("~1", "/").replace("~0", "~")
        if isinstance(value, list) and part.isdigit() and int(part) < len(value):
            value = value[int(part)]
        elif isinstance(value, dict):
            value = value.get(part)
        else:
            return None
    return value


def scrubbed(value, made: list[str]):
    """`value` with every value a placeholder filled written as `<redacted>`."""
    if isinstance(value, dict):
        return {name: scrubbed(inner, made) for name, inner in value.items()}
    if isinstance(value, list):
        return [scrubbed(inner, made) for inner in value]
    if isinstance(value, str):
        if value in made:
            return REDACTED
        for one in made:
            if len(one) >= SCRUBBED_WITHIN:
                value = value.replace(one, REDACTED)
    return value


def recorded(base: str, key: str, pinned: str, speaks: list[str],
             upstream: pathlib.Path) -> tuple[dict[tuple[str, str], dict | str], list[str]]:
    """Every case of every contract spoken, asked in the order each contract lists them, or
    why it could not be asked, and every value a placeholder filled."""
    said: dict[tuple[str, str], dict | str] = {}
    filling = Filling(upstream)
    for spoken in speaks:
        capability, major = spoken.split("@")
        for case in published_cases(spoken):
            path = operation_path(capability, major, case["operation"])
            try:
                body_asked = filling.filled(case["asked"])
            except RuntimeError as unfilled:
                said[(spoken, case["case"])] = f"{spoken}/{case['case']} was not asked: {unfilled}"
                filling.answered(f"{spoken}/{case['case']}", 0, "")
                continue
            status, kind, body = asked(base, path, key if case["keyed"] else None, body_asked)
            filling.answered(f"{spoken}/{case['case']}", status, body)
            note = NOTES.get(case["case"], "")
            if not note:
                raise RuntimeError(f"{spoken}/{case['case']} has no note to record it with")
            said[(spoken, case["case"])] = scrubbed(recording(pinned, path, status, kind, body, note),
                                                    filling.made)
    return said, filling.made


def operation_path(capability: str, major: str, operation: str) -> str:
    """Where an operation is asked, each segment held to the shape the contract writes it in."""
    for segment, shape in ((capability, SEGMENT), (major, MAJOR), (operation, SEGMENT)):
        if not shape.fullmatch(segment):
            raise RuntimeError(f"{segment!r} is not a segment a contract names")
    quoted = [urllib.parse.quote(segment, safe="") for segment in (capability, major, operation)]
    return "/lemonfiber/{}/v{}/{}".format(*quoted)


def filed(said: dict[tuple[str, str], dict | str], spoken: str, case: str, checking: bool) -> str | None:
    """One case written to conformance/, or compared with what is there; the fault, if any."""
    got = said.get((spoken, case))
    path = RECORDINGS / spoken / f"{case}.json"
    if got is None:
        return f"{spoken}/{case} was not asked"
    if isinstance(got, str):
        return got
    if checking:
        held = json.loads(path.read_text(encoding="utf-8")) if path.is_file() else None
        return None if held == got else f"{path.relative_to(ROOT)} is not what Seerr answers now"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(got, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="record again and compare with conformance/")
    checking = parser.parse_args().check

    plugin, pinned, listens = manifest()
    speaks, said = recorded_live(plugin, pinned, listens)
    faults = [fault for spoken in speaks for case in published_cases(spoken)
              if (fault := filed(said, spoken, case["case"], checking)) is not None]
    for fault in faults:
        print(f"::error::{fault}")
    asked_count = sum(isinstance(one, dict) for one in said.values())
    print(f"{asked_count} cases asked against {pinned}; {len(faults)} fault(s).")
    return 1 if faults else 0


if __name__ == "__main__":
    sys.exit(main())
