"""Bind the already executed quality fixtures and retained pixels to the candidate."""
import hashlib
import json
from pathlib import Path


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def retained_pixels(logs, value):
    """Verify only files named by an actual pixel observation, without following escapes."""
    if isinstance(value, list):
        for child in value:
            retained_pixels(logs, child)
    elif isinstance(value, dict):
        if "artifact" in value:
            path = (logs / value["artifact"]).resolve()
            if not path.is_relative_to(logs.resolve()) or not path.is_file():
                raise RuntimeError("Missing or escaped quality pixel artifact: " + str(path))
            expected_size = value["width"] * value["height"] * 4
            if path.stat().st_size != expected_size or digest(path) != value["sha256"]:
                raise RuntimeError("Quality pixel bytes disagree with their observation: " + str(path))
        for child in value.values():
            retained_pixels(logs, child)


def quality_evidence(logs, candidate, completed, unavailable, plan, manifest_sha, binary_sha):
    completed = set(completed)
    skipped = {row["fixture"]: row["reason"] for row in unavailable}
    rows = []
    for item in plan["artifacts"]:
        matches = [name for name in completed | skipped.keys()
            if name.endswith("::" + item["case"])]
        if len(matches) != 1:
            raise RuntimeError("Missing or ambiguous quality fixture: " + item["case"])
        name = matches[0]
        path = logs / "quality-acceptance" / (item["label"] + ".json")
        if name in skipped:
            if path.exists():
                raise RuntimeError("Unexecuted quality fixture left an apparent success artifact.")
            rows.append({"fixture": name, "result": "not_executed", "reason": skipped[name]})
            continue
        report = json.loads(path.read_text())
        body = report.get("observations", {})
        if (report.get("schema") != 1 or report.get("candidate") != candidate
                or report.get("fixture") != item["label"] or not report.get("method")
                or body.get("result") != "passed"):
            raise RuntimeError("Quality evidence is unsuccessful or belongs to a different candidate: " + str(path))
        if item["label"] == "openh264-baseline-transport-still":
            if (body.get("provenance", {}).get("release_commit") != plan["release_baseline"]
                    or body.get("no_codec_regression") is not True):
                raise RuntimeError("OpenH264 comparison lacks its immutable release or quality result.")
            profiles = body.get("profiles", [])
            observed = [(row.get("width"), row.get("height"), row.get("bps")) for row in profiles]
            expected = [(row["width"], row["height"], row["bps"]) for row in plan["profiles"]]
            if observed != expected or any(row.get("no_codec_regression") is not True for row in profiles):
                raise RuntimeError("OpenH264 evidence omits or fails a required quality profile.")
            retained_pixels(logs, body)
        rows.append({"fixture": name, "artifact": str(path.relative_to(logs)),
            "sha256": digest(path), "result": "passed", "method": report["method"]})
    (logs / "quality-acceptance.json").write_text(json.dumps({
        "candidate": candidate, "manifest_sha256": manifest_sha, "binary_sha256": binary_sha,
        "release_baseline": plan["release_baseline"], "observations": rows,
        "result": "passed_with_unverified_environment" if any(row["result"] == "not_executed" for row in rows) else "passed",
        "scope": plan["scope"]}, indent=2))
