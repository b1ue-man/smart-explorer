"""Part `verbindungen` of the suite crates-task (plan docs/plaene/2026-11-14-verbindungen-je-zweck,
package P8, measurements and decision rules C7, follow-up D4).

native/test-crates-task.py runs the commands; this module holds what is decided from the plan: which
ignored measuring tests run on which platform and link, which lines each must print, how the lines
`VERBINDUNGEN-MESSUNG` are checked for completeness and how the rules of C7 turn them into the lines
`VERBINDUNGEN-BEFUND`. It runs only on GitHub Actions (AGENTS.md); it reads text and starts nothing but
the one download of the pinned OpenH264 file.

A measurement line has the form
`VERBINDUNGEN-MESSUNG fall=<M> plattform=<os-arch> variante=<name> profil=<link> <size>=<value> ...`
(native/crates/engine/src/share/core/verbindungen_messung_support.rs). The suite appends
`quelle=<openh264|system>` to the lines of the picture measurements M6 and M7, because Windows measures
the H.264 cases twice (Cisco's OpenH264 and the Media Foundation software coder).

A measurement that could not measure (a line missing, twice, with `status=fehler`, on another link or
platform) fails the part. A value beyond a threshold of the decision rules is a finding:

- `VERBINDUNGEN-BEFUND abnahme=<E2|E4|E7> ... bestanden=<ja|nein>`: an acceptance threshold of C7. Not
  passed fails the part, so that the next run of the same suite confirms the corrected value (D4, N1).
  The candidates of T other than the shipped one print `kandidat_besteht=<ja|nein>` instead and fail nothing.
- `VERBINDUNGEN-BEFUND bedingung=<B1..B5|...> ... erfuellt=<ja|nein|unbekannt>`: the condition of a
  conditional package or of a decision (E5, E6, E10, E11, C5). Never fails the part. `unbekannt` stands
  where the printed numbers cannot decide (the measurement has no p99) or the codec was not available.
- `VERBINDUNGEN-BEFUND entscheidung=<...>`: the value a decision of D4 takes from the measurement.

Thresholds that C7 names are the plan's (marked `[Plan]` below); those it does not give are fixed here
(marked `[P8]`) and are assumptions of this package, printed next to every verdict.
"""
from __future__ import annotations

import hashlib
import re
import urllib.error
import urllib.request
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

PREFIX = "VERBINDUNGEN-MESSUNG "
LINE_NAME = "VERBINDUNGEN-BEFUND"
PLATFORMS = ("linux-x86_64", "linux-aarch64", "windows-x86_64")
LINES_FILE = "verbindungen-messung.txt"

# [Plan] E2: candidates of the queueing delay target T in ms, picture latency p95 may grow by 50 ms with a
# transfer, the transfer alone under the yielding controller keeps 95 % of Cubic's rate.
T_CANDIDATES = (15, 25, 50)
PICTURE_GROWTH_MS = 50.0
BULK_SHARE = 0.95
# [P8] The transfer next to a picture under the yielding controller may give way, but not starve: at least a
# quarter of Cubic's rate with the same picture (the review of the transport parts, N5; B1 of that review
# left a window of 2 segments per round trip, below 1 %).
YIELD_STARVATION_SHARE = 0.25
# [P8] E4: list requests during a transfer at priority -1 must not be slower than at priority 0 (10 % noise).
PRIORITY_NOISE = 1.10
# [Plan] C3 line 2: a list latency above 2 x RTT + 50 ms asks for a control connection of its own.
CONTROL_EXTRA_MS = 50.0
# [Plan] E7 (b): at least 20 % more tunnel throughput; E5: 4 connections at least 15 % faster than one,
# below 2.5 Gbit/s (298.0 MiB/s) of four connections on Linux x86_64 the TCP question is open again;
# DRIVER_RATE is 80 % of the slowest platform's single-connection rate; E6: 10 % more throughput.
TUNNEL_GAIN = 1.20
FOUR_CONNECTIONS_GAIN = 1.15
FOUR_CONNECTIONS_FLOOR_MIB_S = 2.5e9 / 8 / (1024 * 1024)
DRIVER_RATE_SHARE = 0.80
ACK_GAIN = 1.10
# [Plan] C5: encoding p95 at 1440p above 16.7 ms (60 pictures/s) asks for hardware coding (B2), decoding p95
# above 8 ms for hardware decoding; E10: connection setup at least 50 % of the time to the first byte (B3).
ENCODE_P95_MS = 16.7
DECODE_P95_MS = 8.0
SETUP_SHARE = 0.50
# [Plan] E11: a stall above 200 ms more than once a minute (B1); C5: recovery p95 above 2 x RTT plus two
# picture intervals (B4).
STALL_MS = 200.0

# The link `loss1` of M7: 1 % loss, 40 ms round trip, 100 Mbit/s. Like the profiles of
# native/rdt_task/speed_net.py (name -> netem arguments, round trip in ms, rate in Mbit/s) the delay applies
# per direction.
LOSS1 = ("delay 20ms rate 100mbit loss 1% limit 5000", 40, 100)


class Problem(Exception):
    """Something the part cannot go on without (a missing line, a failed download)."""


@dataclass(frozen=True)
class Case:
    """One ignored test and the lines it must print: `fall` and one line per variant."""
    test: str
    fall: str
    variants: tuple[str, ...]


@dataclass(frozen=True)
class Group:
    """Tests that run together on one link. `profile` names the link (`raw` = loopback unshaped);
    `openh264` lets the tests use Cisco's library, `source` is what the H.264 lines are tagged with."""
    title: str
    profile: str
    cases: tuple[Case, ...]
    openh264: bool = False
    source: str = ""


def _case(name: str, fall: str, *variants: str) -> Case:
    return Case(f"verbindungen_messung_task_{name}", fall, variants)


M1 = (_case("m1_c1_b10x20", "M1", "c1_b10x20"), _case("m1_c1_b40x80", "M1", "c1_b40x80"),
      _case("m1_c4_b10x20", "M1", "c4_b10x20"), _case("m1_c4_b40x80", "M1", "c4_b40x80"),
      _case("m9_c1_b40x80_ack", "M9", "c1_b40x80_ack"))
M4 = (_case("m4_vorher", "M4", "vorher"), _case("m4_nachher", "M4", "nachher"))
M2 = (_case("m2_allein", "M2", "allein"), _case("m2_cubic", "M2", "cubic"),
      *(_case(f"m2_rueckzug_t{t}", "M2", f"rueckzug_t{t}") for t in T_CANDIDATES),
      _case("m2_bulk_cubic", "M2", "bulk_cubic"), _case("m2_bulk_rueckzug", "M2", "bulk_rueckzug"))
M3 = (_case("m3_vorher", "M3", "vorher"), _case("m3_prio", "M3", "prio"))
M8 = (_case("m8_neue_verbindung", "M8", "neue_verbindung"),
      _case("m8_bestehende_verbindung", "M8", "bestehende_verbindung"))
M7 = (_case("m7_bild_unter_verlust", "M7", "paar_1080p"),)
SIZES = ("1920x1080", "2560x1440", "3840x2160")


def _m6(codec: str, mode: str) -> Case:
    return Case(f"verbindungen_messung_task_m6_{codec}_{mode}", "M6",
                tuple(f"{codec}_{size}_{mode}" for size in SIZES))


M6_TILES = (_m6("tiles", "local"), _m6("tiles", "paar"))
M6_H264 = (_m6("h264", "local"), _m6("h264", "paar"))


def platform_key(system: str, machine: str) -> str:
    """`linux-x86_64`, `linux-aarch64`, `windows-x86_64`: what the Rust side prints as `plattform=`."""
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(
        machine.lower(), machine.lower())
    return f"{system.lower()}-{arch}"


def groups_for(key: str) -> list[Group]:
    """C7: M1, M4 and M6 on every platform without shaping; M2, M3 and M8 on `lan` and `wan`, M7 on `loss`
    and `loss1` on Linux x86_64 (the runner with netem). Windows also measures H.264 with the Media
    Foundation software coder (C7: "Windows (MF Software)"); Linux ARM64 has no OpenH264 pin, so its H.264
    lines say `nicht_verfuegbar`."""
    core = (*M1, *M4, *M6_TILES)
    if key == "linux-x86_64":
        return [Group("raw", "raw", (*core, *M6_H264), True, "openh264"),
                *(Group(name, name, (*M2, *M3, *M8)) for name in ("lan", "wan")),
                *(Group(name, name, M7, True, "openh264") for name in ("loss", "loss1"))]
    if key == "linux-aarch64":
        return [Group("raw", "raw", (*core, *M6_H264), False, "system")]
    if key == "windows-x86_64":
        return [Group("raw", "raw", (*core, *M6_H264), True, "openh264"),
                Group("raw (Media Foundation)", "raw", M6_H264, False, "system")]
    return []


def link_profiles() -> dict:
    """Name -> (netem arguments, round trip in ms, rate in Mbit/s): the profiles of
    native/rdt_task/speed_net.py, imported unchanged, and `loss1`."""
    from rdt_task import speed_net  # native/ is the directory of the entrypoint script

    return {**speed_net.PROFILES, "loss1": LOSS1}


def shipped_target_ms(root: Path) -> int:
    """T the build ships (DEFAULT_TARGET in the yielding controller), read from the source."""
    text = (root / "native" / "crates" / "engine" / "src" / "share" / "core" / "yield_controller.rs").read_text(
        encoding="utf-8")
    found = re.search(r"DEFAULT_TARGET\s*:\s*Duration\s*=\s*Duration::from_millis\(\s*(\d+)\s*\)", text)
    if not found:
        raise Problem("DEFAULT_TARGET not found in yield_controller.rs")
    return int(found.group(1))


# ---------------------------------------------------------------------------------------- OpenH264

PINS_FILE = ("native", "crates", "engine", "src", "screen", "os", "native_ffi", "openh264_pins.rs")
PIN_MARKERS = {"linux-x86_64": '#[cfg(all(target_os = "linux", target_arch = "x86_64"))]',
               "windows-x86_64": '#[cfg(all(windows, target_arch = "x86_64"))]'}


def openh264_pin(root: Path, key: str) -> dict | None:
    """URL, size and SHA-256 of Cisco's compressed library for `key` from the pins file of the build; None
    where the build has no pin (Linux ARM64)."""
    marker = PIN_MARKERS.get(key)
    if marker is None:
        return None
    source = root.joinpath(*PINS_FILE).read_text(encoding="utf-8")
    if source.count(marker) != 1:
        raise Problem(f"no unique OpenH264 pin for {key} in {'/'.join(PINS_FILE)}")
    block = source.split(marker, 1)[1].split("});", 1)[0]
    url = re.search(r'url:\s*"([^"]+)"', block)
    size = re.search(r"compressed_size:\s*([0-9_]+)", block)
    digest = re.search(r'compressed_sha256:\s*"([0-9a-f]{64})"', block)
    if not (url and size and digest):
        raise Problem(f"the OpenH264 pin for {key} is incomplete")
    return {"url": url.group(1), "size": int(size.group(1).replace("_", "")), "sha256": digest.group(1)}


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def openh264_download(root: Path, key: str, folder: Path) -> Path | None:
    """The pinned file in `folder` (value of SE_SCREEN_TASK_OPENH264), None without a pin. The download goes
    to the pinned host without redirects and must match size and SHA-256, as the build checks them."""
    pin = openh264_pin(root, key)
    if pin is None:
        return None
    if not pin["url"].startswith("https://ciscobinary.openh264.org/"):
        raise Problem(f"the OpenH264 pin names a foreign host: {pin['url']}")
    target = folder / pin["url"].rsplit("/", 1)[-1]
    try:
        with urllib.request.build_opener(_NoRedirect).open(pin["url"], timeout=120) as response:
            data = response.read(pin["size"] + 1)
    except (urllib.error.URLError, OSError, TimeoutError) as error:
        raise Problem(f"download of {pin['url']} failed: {error}") from error
    digest = hashlib.sha256(data).hexdigest()
    if len(data) != pin["size"] or digest != pin["sha256"]:
        raise Problem(f"{pin['url']}: {len(data)} bytes, SHA-256 {digest}; the pin says {pin['size']} bytes, "
                      f"{pin['sha256']}")
    target.write_bytes(data)
    return target


# ---------------------------------------------------------------------------------------- the lines

def parse(lines, source: str = "") -> list[dict]:
    """The measurement lines among `lines` as dictionaries; `source` (the codec source of the group) tags the
    lines of the picture measurements M6 and M7 as `quelle` when the line has none."""
    records = []
    for line in lines:
        at = line.find(PREFIX)
        if at < 0:
            continue
        fields = {}
        for token in line[at + len(PREFIX):].split():
            name, separator, value = token.partition("=")
            if separator:
                fields[name] = value
        if source and fields.get("fall") in ("M6", "M7") and "quelle" not in fields:
            fields["quelle"] = source
        records.append(fields)
    return records


def render(record: dict) -> str:
    """The line of a record, the keys in a fixed order first."""
    head = ("fall", "plattform", "variante", "profil")
    rest = [name for name in record if name not in head]
    return PREFIX + " ".join(f"{name}={record[name]}" for name in (*head, *rest) if name in record)


def check_group(group: Group, records: list[dict], key: str) -> list[str]:
    """What keeps `records` from being the lines `group` must print: every line of every case exactly once,
    on the platform and link of the group, none with `status=fehler`, none unexpected. A status of
    `nicht_verfuegbar` is allowed for H.264 on a device the suite gave no OpenH264 library (Linux ARM64, the
    Media Foundation coder of Windows)."""
    problems = []
    expected = {(case.fall, variant) for case in group.cases for variant in case.variants}
    seen = Counter((record.get("fall"), record.get("variante")) for record in records)
    for fall, variant in sorted(expected):
        if seen[(fall, variant)] != 1:
            problems.append(f"{fall} {variant}: {seen[(fall, variant)]} lines instead of one")
    for fall, variant in sorted(set(seen) - expected, key=str):
        problems.append(f"{fall} {variant}: a line no case of this group prints")
    for record in records:
        name = f"{record.get('fall')} {record.get('variante')}"
        if record.get("plattform") != key:
            problems.append(f"{name}: platform {record.get('plattform')}, this runner is {key}")
        if record.get("profil") != group.profile:
            problems.append(f"{name}: link {record.get('profil')}, the group runs on {group.profile}")
        status = record.get("status")
        if status == "fehler":
            problems.append(f"{name}: status=fehler")
        elif status == "nicht_verfuegbar" and (group.openh264
                                               or not str(record.get("variante", "")).startswith("h264_")):
            problems.append(f"{name}: status=nicht_verfuegbar for a codec that must exist")
    return problems


# ---------------------------------------------------------------------------------------- decisions

class _Lines:
    """The findings of one evaluation. An acceptance that is not passed (`bestanden=nein`) fails the part;
    every other line is a finding or a number a decision of D4 takes."""

    def __init__(self) -> None:
        self.lines: list[str] = []
        self.failed = False

    def add(self, kind: str, name: str, verdict: tuple[str, str] | None, **fields) -> None:
        text = f"{LINE_NAME} {kind}={name}"
        if verdict:
            text += f" {verdict[0]}={verdict[1]}"
            if verdict == ("bestanden", "nein"):
                self.failed = True
        for field, value in fields.items():
            value = str(value)
            if re.search(r"\s", value):
                value = '"' + value.replace('"', "'") + '"'
            text += f" {field}={value}"
        self.lines.append(text)


def _f(value: float) -> str:
    return f"{value:.1f}"


def _yes(value: bool) -> str:
    return "ja" if value else "nein"


class _View:
    """The records of one platform, looked up by case."""

    def __init__(self, records: list[dict]) -> None:
        self.records = records

    def get(self, fall: str, variant: str, profile: str = "raw", source: str | None = None) -> dict:
        found = [record for record in self.records
                 if record.get("fall") == fall and record.get("variante") == variant
                 and record.get("profil") == profile
                 and (source is None or record.get("quelle") == source)]
        if len(found) != 1:
            raise Problem(f"{fall} {variant} on {profile}: {len(found)} lines")
        return found[0]

    def value(self, fall: str, variant: str, name: str, profile: str = "raw") -> float:
        record = self.get(fall, variant, profile)
        try:
            return float(record[name])
        except (KeyError, ValueError) as error:
            raise Problem(f"{fall} {variant} on {profile}: no number {name}") from error


def _picture_constancy(view: _View, out: _Lines, key: str, profile: str, shipped: int) -> dict:
    """E2 on one link. Per candidate T: the growth of the picture latency p95 with a transfer over the picture
    alone [Plan] and the transfer next to a picture against Cubic's with the same picture [P8]; for the
    shipped T they are acceptances, for the other candidates `kandidat_besteht`. Then the transfer alone
    under the yielding controller against Cubic's [Plan]. Returns what each candidate passed."""
    alone = view.value("M2", "allein", "bild_p95_ms", profile)
    cubic_picture = view.value("M2", "cubic", "bild_p95_ms", profile)
    cubic_bulk = view.value("M2", "cubic", "bulk_mib_s", profile)
    out.add("messung", "M2-cubic", None, plattform=key, profil=profile, bild_allein_p95_ms=_f(alone),
            bild_mit_cubic_p95_ms=_f(cubic_picture), aufschlag_ms=_f(cubic_picture - alone),
            bulk_mit_bild_mib_s=_f(cubic_bulk))
    passed = {}
    for target in T_CANDIDATES:
        variant = f"rueckzug_t{target}"
        picture = view.value("M2", variant, "bild_p95_ms", profile)
        bulk = view.value("M2", variant, "bulk_mib_s", profile)
        growth = picture - alone
        share = bulk / cubic_bulk if cubic_bulk > 0 else 0.0
        latency_ok = growth <= PICTURE_GROWTH_MS
        starve_ok = share >= YIELD_STARVATION_SHARE
        passed[target] = latency_ok and starve_ok
        word = "bestanden" if target == shipped else "kandidat_besteht"
        common = {"plattform": key, "profil": profile, "t_ms": target, "ausgeliefert": _yes(target == shipped)}
        out.add("abnahme", "E2", (word, _yes(latency_ok)), teil="bildlatenz", **common, bild_p95_ms=_f(picture),
                aufschlag_ms=_f(growth), grenze_aufschlag_ms=_f(PICTURE_GROWTH_MS), grenze_quelle="Plan")
        out.add("abnahme", "E2", (word, _yes(starve_ok)), teil="uebertragung_unter_bild", **common,
                bulk_mib_s=_f(bulk), cubic_mit_bild_mib_s=_f(cubic_bulk),
                anteil_prozent=_f(100 * share), grenze_prozent=_f(100 * YIELD_STARVATION_SHARE),
                grenze_quelle="P8")
    bulk_cubic = view.value("M2", "bulk_cubic", "bulk_mib_s", profile)
    bulk_yield = view.value("M2", "bulk_rueckzug", "bulk_mib_s", profile)
    share = bulk_yield / bulk_cubic if bulk_cubic > 0 else 0.0
    out.add("abnahme", "E2", ("bestanden", _yes(share >= BULK_SHARE)), plattform=key, profil=profile,
            teil="massendurchsatz_allein", cubic_mib_s=_f(bulk_cubic), rueckzug_mib_s=_f(bulk_yield),
            anteil_prozent=_f(100 * share), grenze_prozent=_f(100 * BULK_SHARE), grenze_quelle="Plan")
    return passed


def _constancy(view: _View, out: _Lines, key: str, shipped: int) -> None:
    if shipped not in T_CANDIDATES:
        out.add("abnahme", "E2", ("bestanden", "nein"), plattform=key, t_ms=shipped,
                grund=f"die ausgelieferte T ist nicht unter den gemessenen {list(T_CANDIDATES)}")
        return
    per_profile = {profile: _picture_constancy(view, out, key, profile, shipped) for profile in ("lan", "wan")}
    chosen = [t for t in T_CANDIDATES if all(passed[t] for passed in per_profile.values())]
    out.add("entscheidung", "T", None, plattform=key, ausgeliefert_ms=shipped,
            empfehlung_ms=max(chosen) if chosen else "keine",
            regel="groesstes T, dessen Bildaufschlag und Uebertragung auf lan und wan bestehen")


def _priority(view: _View, out: _Lines, key: str, rtts: dict) -> None:
    for profile in ("lan", "wan"):
        before = view.value("M3", "vorher", "liste_p95_ms", profile)
        after = view.value("M3", "prio", "liste_p95_ms", profile)
        out.add("abnahme", "E4", ("bestanden", _yes(after <= before * PRIORITY_NOISE)), plattform=key,
                profil=profile, liste_vorher_p95_ms=_f(before), liste_prio_p95_ms=_f(after),
                liste_vorher_p50_ms=_f(view.value("M3", "vorher", "liste_p50_ms", profile)),
                liste_prio_p50_ms=_f(view.value("M3", "prio", "liste_p50_ms", profile)),
                bulk_vorher_mib_s=_f(view.value("M3", "vorher", "bulk_mib_s", profile)),
                bulk_prio_mib_s=_f(view.value("M3", "prio", "bulk_mib_s", profile)),
                grenze_faktor=PRIORITY_NOISE, grenze_quelle="P8")
        limit = 2 * rtts[profile] + CONTROL_EXTRA_MS
        out.add("bedingung", "Steuerverbindung", ("erfuellt", _yes(after > limit)), plattform=key,
                profil=profile, liste_prio_p95_ms=_f(after), grenze_ms=_f(limit), grenze_quelle="Plan",
                folge="bei ja: eigene Steuerverbindung in einem Planabschnitt nachtragen (C3 Zeile 2)")


def _setup_share(view: _View, out: _Lines, key: str) -> None:
    for profile in ("lan", "wan"):
        fresh = view.value("M8", "neue_verbindung", "auftrag_p50_ms", profile)
        reused = view.value("M8", "bestehende_verbindung", "auftrag_p50_ms", profile)
        share = (fresh - reused) / fresh if fresh > 0 else 0.0
        out.add("bedingung", "B3", ("erfuellt", _yes(share >= SETUP_SHARE and profile == "wan")),
                plattform=key, profil=profile, auftrag_neue_verbindung_p50_ms=_f(fresh),
                auftrag_bestehende_verbindung_p50_ms=_f(reused), aufbau_anteil_prozent=_f(100 * share),
                grenze_prozent=_f(100 * SETUP_SHARE), entschieden_auf="wan", grenze_quelle="Plan")


def _loss(view: _View, out: _Lines, key: str, rtts: dict) -> None:
    for profile in ("loss", "loss1"):
        rtt = rtts[profile]
        p50 = view.value("M7", "paar_1080p", "bild_p50_ms", profile)
        p95 = view.value("M7", "paar_1080p", "bild_p95_ms", profile)
        top = view.value("M7", "paar_1080p", "bild_max_ms", profile)
        gap_p95 = view.value("M7", "paar_1080p", "luecke_p95_ms", profile)
        gap_max = view.value("M7", "paar_1080p", "luecke_max_ms", profile)
        interval = view.value("M7", "paar_1080p", "bildintervall_ms", profile)
        # B1: p99 more than 2 x RTT above p50, or a stall above 200 ms more than once a minute. The line has no
        # p99; it lies between p95 and the maximum. A stall in the 10 s of the measurement is at least six a
        # minute.
        sure_yes = p95 - p50 > 2 * rtt or gap_max > STALL_MS
        sure_no = top - p50 <= 2 * rtt and gap_max <= STALL_MS
        verdict = "ja" if sure_yes else "nein" if sure_no else "unbekannt"
        out.add("bedingung", "B1", ("erfuellt", verdict), plattform=key, profil=profile, rtt_ms=rtt,
                bild_p50_ms=_f(p50), bild_p95_ms=_f(p95), bild_max_ms=_f(top), luecke_max_ms=_f(gap_max),
                grenze_p99_aufschlag_ms=_f(2 * rtt), grenze_stillstand_ms=_f(STALL_MS), grenze_quelle="Plan",
                hinweis="p99 liegt zwischen p95 und Maximum" if verdict == "unbekannt" else "-")
        limit = 2 * rtt + 2 * interval
        out.add("bedingung", "B4", ("erfuellt", _yes(gap_p95 > limit)), plattform=key, profil=profile,
                erholzeit_luecke_p95_ms=_f(gap_p95), grenze_ms=_f(limit), grenze_quelle="Plan")


def _stations(view: _View, out: _Lines, key: str) -> None:
    """B2: encoding p95 at 1440p above 16.7 ms, decoding p95 above 8 ms; meaningful for the Linux platforms,
    where no hardware coder is used today."""
    if not key.startswith("linux-"):
        return
    variant = "h264_2560x1440_local"
    record = view.get("M6", variant, "raw")
    if record.get("status") != "ok":
        for name in ("B2", "B2-Dekodierung"):
            out.add("bedingung", name, ("erfuellt", "unbekannt"), plattform=key, status=record.get("status"),
                    hinweis="kein H.264-Codec auf diesem Geraet gemessen (Linux ARM64 hat keinen OpenH264-Pin)")
        return
    encode = view.value("M6", variant, "encode_p95_ms")
    decode = view.value("M6", variant, "decode_p95_ms")
    out.add("bedingung", "B2", ("erfuellt", _yes(encode > ENCODE_P95_MS)), plattform=key, codec="h264",
            kodierung_p95_ms=_f(encode), grenze_ms=_f(ENCODE_P95_MS), grenze_quelle="Plan")
    out.add("bedingung", "B2-Dekodierung", ("erfuellt", _yes(decode > DECODE_P95_MS)), plattform=key,
            codec="h264", dekodierung_p95_ms=_f(decode), grenze_ms=_f(DECODE_P95_MS), grenze_quelle="Plan")


def _throughput(view: _View, out: _Lines, key: str) -> None:
    """M1, M9 and M4 of one platform: the bundle, four connections against one, the ACK frequency, the tunnel."""
    old = view.value("M1", "c1_b10x20", "mib_s")
    new = view.value("M1", "c1_b40x80", "mib_s")
    four = view.value("M1", "c4_b40x80", "mib_s")
    ack = view.value("M9", "c1_b40x80_ack", "mib_s")
    out.add("messung", "M1", None, plattform=key, c1_b10x20_mib_s=_f(old), c1_b40x80_mib_s=_f(new),
            c4_b10x20_mib_s=_f(view.value("M1", "c4_b10x20", "mib_s")), c4_b40x80_mib_s=_f(four),
            c1_b40x80_ack_mib_s=_f(ack))
    out.add("entscheidung", "DRIVER_RATE", None, plattform=key, einzelrate_mib_s=_f(new),
            wert_mib_s=_f(DRIVER_RATE_SHARE * new), regel="80 % der Einzelverbindungsrate; der kleinste Wert "
            "aller Plattformen gilt (Zeile im Gesamtjob)")
    out.add("bedingung", "E5-Buendel", ("erfuellt", _yes(new > old)), plattform=key,
            gewinn_prozent=_f(100 * (new / old - 1) if old > 0 else 0.0), grenze_quelle="P8",
            regel="40/80 ist nicht langsamer als 10/20")
    out.add("bedingung", "MAX_EXTRA", ("erfuellt", _yes(four >= FOUR_CONNECTIONS_GAIN * new)),
            plattform=key, vier_gegen_eins_prozent=_f(100 * (four / new - 1) if new > 0 else 0.0),
            grenze_prozent=_f(100 * (FOUR_CONNECTIONS_GAIN - 1)), grenze_quelle="Plan",
            folge="auf keiner Plattform ja: MAX_EXTRA bleibt 3 (E5)")
    if key == "linux-x86_64":
        out.add("bedingung", "X-TCP", ("erfuellt", _yes(four < FOUR_CONNECTIONS_FLOOR_MIB_S)), plattform=key,
                vier_verbindungen_mib_s=_f(four), grenze_mib_s=_f(FOUR_CONNECTIONS_FLOOR_MIB_S),
                grenze_quelle="Plan", folge="bei ja: X-TCP in einem eigenen Planabschnitt neu bewerten (E5)")
    gain = ack / new if new > 0 else 0.0
    out.add("bedingung", "B5", ("erfuellt", _yes(gain >= ACK_GAIN)), plattform=key,
            gewinn_prozent=_f(100 * (gain - 1)), grenze_prozent=_f(100 * (ACK_GAIN - 1)), grenze_quelle="Plan",
            folge="E6: mindestens zwei Plattformen (Gesamtjob); CPU-Zeit je GiB wird nicht gemessen")
    before = view.value("M4", "vorher", "mib_s")
    after = view.value("M4", "nachher", "mib_s")
    ratio = after / before if before > 0 else 0.0
    if key == "linux-x86_64":
        out.add("abnahme", "E7", ("bestanden", _yes(ratio >= TUNNEL_GAIN)), plattform=key,
                vorher_mib_s=_f(before), nachher_mib_s=_f(after), gewinn_prozent=_f(100 * (ratio - 1)),
                grenze_prozent=_f(100 * (TUNNEL_GAIN - 1)), grenze_quelle="Plan",
                folge="bei nein: Pufferwachstum (b) bleibt bei 8 KiB (E7)")
    else:
        out.add("messung", "M4", None, plattform=key, vorher_mib_s=_f(before), nachher_mib_s=_f(after),
                gewinn_prozent=_f(100 * (ratio - 1)), hinweis="E7 wird nur auf Linux x86_64 bewertet")


def evaluate_platform(key: str, records: list[dict], shipped: int, profiles: dict) -> tuple[list[str], bool]:
    """The findings of one platform's lines and whether an acceptance failed."""
    out = _Lines()
    view = _View(records)
    rtts = {name: profile[1] for name, profile in profiles.items()}
    try:
        _throughput(view, out, key)
        _stations(view, out, key)
        if key == "linux-x86_64":
            _constancy(view, out, key, shipped)
            _priority(view, out, key, rtts)
            _setup_share(view, out, key)
            _loss(view, out, key, rtts)
    except Problem as error:
        out.add("abnahme", "Auswertung", ("bestanden", "nein"), plattform=key, grund=str(error))
    return out.lines, out.failed


def evaluate_overall(by_platform: dict[str, list[dict]]) -> tuple[list[str], bool]:
    """The decisions that need every platform: DRIVER_RATE (slowest platform), the bundle, four connections
    against one (MAX_EXTRA) and the ACK frequency (B5, E6: at least two platforms). A platform without
    lines fails the evaluation."""
    out = _Lines()
    missing = [key for key in PLATFORMS if not by_platform.get(key)]
    if missing:
        out.add("abnahme", "Gesamtauswertung", ("bestanden", "nein"), fehlend="|".join(missing),
                grund="ohne die Zeilen jeder Plattform sind die Entscheidungen ueber alle Plattformen offen")
    views = {key: _View(by_platform[key]) for key in PLATFORMS if by_platform.get(key)}
    try:
        rates = {key: view.value("M1", "c1_b40x80", "mib_s") for key, view in views.items()}
        slowest = min(rates, key=rates.get) if rates else None
        if slowest:
            out.add("entscheidung", "DRIVER_RATE", None, langsamste_plattform=slowest,
                    einzelrate_mib_s=_f(rates[slowest]), wert_mib_s=_f(DRIVER_RATE_SHARE * rates[slowest]),
                    wert_bytes=int(DRIVER_RATE_SHARE * rates[slowest] * 1024 * 1024),
                    plattformen="|".join(f"{key}:{_f(rate)}" for key, rate in sorted(rates.items())),
                    regel="80 % der Einzelverbindungsrate der langsamsten Plattform (E5)")
        bundles = {key: view.value("M1", "c1_b40x80", "mib_s") > view.value("M1", "c1_b10x20", "mib_s")
                   for key, view in views.items()}
        out.add("bedingung", "E5-Buendel", ("erfuellt", _yes(bool(bundles) and all(bundles.values()))),
                plattformen_mit_gewinn=f"{sum(bundles.values())} von {len(bundles)}",
                regel="40/80 auf jeder Plattform nicht langsamer als 10/20", grenze_quelle="P8")
        faster = {key: view.value("M1", "c4_b40x80", "mib_s") >= FOUR_CONNECTIONS_GAIN
                  * view.value("M1", "c1_b40x80", "mib_s") for key, view in views.items()}
        out.add("bedingung", "MAX_EXTRA", ("erfuellt", _yes(any(faster.values()))),
                plattformen_ueber_15_prozent=f"{sum(faster.values())} von {len(faster)}", grenze_quelle="Plan",
                folge="nein: MAX_EXTRA bleibt 3 (E5)")
        acks = {key: view.value("M9", "c1_b40x80_ack", "mib_s") >= ACK_GAIN
                * view.value("M1", "c1_b40x80", "mib_s") for key, view in views.items()}
        out.add("bedingung", "B5", ("erfuellt", _yes(sum(acks.values()) >= 2)),
                plattformen_ueber_10_prozent=f"{sum(acks.values())} von {len(acks)}", grenze_quelle="Plan",
                folge="E6: zusaetzlich muss M2 mit Rueckzug weiter bestehen (E2 des Linux-x86_64-Jobs, ohne "
                "die ACK-Frequenz gemessen); nach dem Einschalten bestaetigt derselbe Lauf es")
    except Problem as error:
        out.add("abnahme", "Gesamtauswertung", ("bestanden", "nein"), grund=str(error))
    return out.lines, out.failed
