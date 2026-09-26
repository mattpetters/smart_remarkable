"""Bounded pen drawings shared by the Codex and Hermes answer formats."""
import math


def obj(properties):
    return {"type": "object", "properties": properties, "required": list(properties), "additionalProperties": False}


def number(low, high):
    return {"type": "number", "minimum": low, "maximum": high}


ILLUSTRATIONS_SCHEMA = {
    "type": "array", "maxItems": 2,
    "items": obj({
        "title": {"type": "string", "maxLength": 44},
        "strokes": {"type": "array", "minItems": 1, "maxItems": 64, "items": {
            "type": "array", "minItems": 2, "maxItems": 128,
            "items": obj({"x": number(8, 592), "y": number(8, 352)})}},
        "labels": {"type": "array", "maxItems": 24, "items": obj({
            "x": number(8, 592), "y": number(18, 352),
            "text": {"type": "string", "minLength": 1, "maxLength": 40}})},
    }),
}

DRAWING_INSTRUCTIONS = (
    "When a chart, graph, circuit, flowchart or simple line illustration materially helps explain the answer, "
    "include it in illustrations; otherwise return an empty array. Up to two drawings follow the answer. "
    "Each drawing uses a 600 by 360 coordinate canvas, x rightward and y downward. "
    "Provide polylines as strokes (arrays of {x,y} points), and labels as {x,y,text}. "
    "No SVG, images, code, filled shapes or external resources. Use sparse outlines, explicit arrowheads, "
    "and well-spaced labels. Prefer ASCII labels (tau, ohm) for legibility. Text is 18px monospace, left anchored, with y as its baseline; "
    "keep x + 10.8 * label length <= 592, and avoid crossing text with strokes. "
    "Keep all points within x=8..592,y=8..352; labels y>=18. Maximum 64 strokes, 1024 points total, "
    "24 labels, 40 characters per label, and a 44-character title per drawing. "
    "For graphs include axes, units, meaningful ticks and curve labels; label conceptual or illustrative "
    "data as such. Compute numeric plots accurately using tools when helpful. Never invent measured data. "
    "Give source attribution in the answer for researched data. Prefer one clear diagram over decoration. "
)


def validate_illustrations(drawings):
    def text(value, limit):
        return isinstance(value, str) and 0 < len(value) <= limit and all(32 <= ord(c) < 0x2e80 and ord(c) != 127 for c in value)

    def coord(value, low, high):
        return type(value) in (int, float) and math.isfinite(value) and low <= value <= high

    if not isinstance(drawings, list) or len(drawings) > 2:
        raise ValueError("Expected at most two illustrations")
    for drawing in drawings:
        if not isinstance(drawing, dict) or set(drawing) != {"title", "strokes", "labels"} or not text(drawing["title"], 44):
            raise ValueError("Invalid illustration title or fields")
        strokes, labels = drawing["strokes"], drawing["labels"]
        if not isinstance(strokes, list) or not 1 <= len(strokes) <= 64:
            raise ValueError("Invalid illustration strokes")
        count = 0
        for stroke in strokes:
            if not isinstance(stroke, list) or not 2 <= len(stroke) <= 128:
                raise ValueError("Invalid illustration path")
            count += len(stroke)
            for point in stroke:
                if (not isinstance(point, dict) or set(point) != {"x", "y"}
                        or not coord(point["x"], 8, 592) or not coord(point["y"], 8, 352)):
                    raise ValueError("Illustration point outside canvas")
        if count > 1024 or not isinstance(labels, list) or len(labels) > 24:
            raise ValueError("Illustration complexity exceeded")
        for label in labels:
            if (not isinstance(label, dict) or set(label) != {"x", "y", "text"}
                    or not text(label["text"], 40) or not coord(label["x"], 8, 592)
                    or not coord(label["y"], 18, 352) or label["x"] + 10.8 * len(label["text"]) > 592):
                raise ValueError("Illustration label outside canvas")
    return drawings
