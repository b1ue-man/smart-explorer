"""Report of the stage family ``speed`` (U55-6 (3) and (4), M-1): the JSON lines the Rust cases append to
``SE_SPEED_REPORT`` become the verdict per case, ``speed_report.json`` and the Markdown table of the job summary.

A case is "bestanden" only when the test program passed, a measured line exists and no requirement was skipped.
A skipped requirement (tool missing) or a ``not_run`` line makes it "nicht ausgeführt" with the reason; a
failed test is "gescheitert" with the failure text of the test.
"""
from __future__ import annotations

import json
from pathlib import Path

from . import common
from .common import FAILED, NOT_RUN, PASSED, first_line, md

REPORT_NAME = "speed_report.json"
CC_LOSS, CC_WAN = "speed_task_share_cc_bbr3_vs_cubic_loss", "speed_task_share_cc_bbr3_vs_cubic_wan"


def report_file(ctx: common.JobContext) -> Path:
    path = ctx.work / "speed" / "report.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def offset(ctx: common.JobContext) -> int:
    path = report_file(ctx)
    return path.stat().st_size if path.is_file() else 0


def read_lines(ctx: common.JobContext, start: int = 0) -> list[dict]:
    path = report_file(ctx)
    if not path.is_file():
        return []
    with open(path, "rb") as handle:
        handle.seek(start)
        text = handle.read().decode("utf-8", "replace")
    lines = []
    for raw in text.splitlines():
        try:
            value = json.loads(raw)
        except ValueError:
            continue
        if isinstance(value, dict) and value.get("name"):
            lines.append(value)
    return lines


def rate(value: float, unit: str) -> str:
    return f"{value / 1e6:.2f} MB/s" if unit == "B/s" else f"{value:.1f} {unit}"


def summary(line: dict) -> str:
    unit = line.get("unit", "B/s")
    text = f"{rate(line.get('se_rate', 0.0), unit)}"
    if line.get("capacity"):
        text += f" = {line.get('share', 0.0):.2f} × {rate(line['capacity'], unit)}"
    if line.get("requirement"):
        text += f" ({line['requirement']})"
    if line.get("limiter"):
        text += f"; Engpass {line['limiter']}"
    notes = [note for note in line.get("notes", []) if note][:2]
    return text + ("; " + " | ".join(notes) if notes else "")


def verdict(name: str, libtest: tuple, lines: list[dict]) -> tuple[str, str]:
    """(status, reason) of a case from the libtest result ``(status, reason, duration)`` and the report lines."""
    status, reason, _ = libtest
    mine = [line for line in lines if line.get("name") == name]
    measured = [line for line in mine if line.get("status") == "measured"]
    not_run = [line for line in mine if line.get("status") == "not_run"]
    if status == FAILED:
        return FAILED, reason or "Test gescheitert"
    if status == NOT_RUN and not mine:
        return NOT_RUN, reason
    if not_run:
        return NOT_RUN, not_run[-1].get("reason", "") + (f" (gemessen: {summary(measured[-1])})" if measured else "")
    if not measured:
        return FAILED, "der Test endete ohne Berichtszeile in SE_SPEED_REPORT"
    last = measured[-1]
    if last.get("skipped"):
        return NOT_RUN, "; ".join(last["skipped"]) + f" (übrige Forderungen bestanden; gemessen: {summary(last)})"
    return PASSED, summary(last)


def cc_flag(lines: list[dict]) -> dict | None:
    """U44-11 (b) from the two controller cases: bbr3 ≥ 1.2 × cubic on ``loss`` and ≥ 0.95 × on ``wan``. The case
    list of M-6 has no ``lan`` case, so ``lan`` is not part of the flag (stated in the result)."""
    shares = {}
    for name in (CC_LOSS, CC_WAN):
        found = [line for line in lines if line.get("name") == name and line.get("status") == "measured"
                 and line.get("capacity") and not line.get("skipped")]
        if found:
            shares[name] = found[-1]["share"]
    if len(shares) < 2:
        return None
    adopt = shares[CC_LOSS] >= 1.2 and shares[CC_WAN] >= 0.95
    return {"loss_bbr3_over_cubic": shares[CC_LOSS], "wan_bbr3_over_cubic": shares[CC_WAN], "lan": "nicht gemessen",
            "default_bbr3": adopt}


def last_lines(ctx: common.JobContext) -> list[dict]:
    last: dict[str, dict] = {}
    for line in read_lines(ctx):
        if line.get("status") == "measured":
            last[line["name"]] = line
    return list(last.values())


def table_markdown(ctx: common.JobContext) -> str:
    lines = last_lines(ctx)
    if not lines:
        return ""
    rows = ["", "### Messwerte der Stufenfamilie speed", "",
            "| Name | Weg | Strecke | Rate | Kapazität | Anteil | Forderung | Engpass |",
            "|---|---|---|---:|---:|---:|---|---|"]
    for line in sorted(lines, key=lambda item: item["name"]):
        unit = line.get("unit", "B/s")
        rows.append(f"| `{line['name']}` | {line.get('way', '')} | {line.get('profile', '')} | "
                    f"{rate(line.get('se_rate', 0.0), unit)} | "
                    f"{rate(line['capacity'], unit) if line.get('capacity') else '–'} | "
                    f"{line.get('share', 0.0):.2f} | {md(line.get('requirement', ''), 120)} | "
                    f"{line.get('limiter') or '–'} |")
    flag = cc_flag(read_lines(ctx))
    if flag:
        rows += ["", f"QUIC-Regler (U44-11): bbr3 ÷ cubic auf loss {flag['loss_bbr3_over_cubic']:.2f}, auf wan "
                     f"{flag['wan_bbr3_over_cubic']:.2f}, lan nicht gemessen; Entscheidungsflagge: "
                     + ("Vorgabe auf bbr3 ändern" if flag["default_bbr3"] else "cubic bleibt")]
    return "\n".join(rows) + "\n"


def write_json(ctx: common.JobContext) -> None:
    data = {"job": ctx.job, "candidate_sha": ctx.candidate_sha, "cases": last_lines(ctx),
            "decision_flag_quic_cc": cc_flag(read_lines(ctx)), "tools": ctx.state.get("speed_tools", {})}
    (ctx.log_root / REPORT_NAME).write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def attach_summary(ctx: common.JobContext) -> None:
    """Appends the table to ``summary.md`` of the job: the report's ``summary`` is wrapped once."""
    if ctx.state.get("speed_summary_attached"):
        return
    ctx.state["speed_summary_attached"] = True
    original = ctx.report.summary
    ctx.report.summary = lambda: original() + table_markdown(ctx)


def why(text: str) -> str:
    return first_line(text) or text
