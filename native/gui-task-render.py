"""Render the actual egui task meshes/textures into reviewable PNG artifacts.

Remote CI only. Pillow and NumPy are preflighted by the workflow. This consumes
the application's paint output, without reimplementing any UI widgets/layout.
"""
import base64
import json
import pathlib
import sys

import numpy as np
from PIL import Image, ImageDraw


def render(path):
    data = json.loads(path.read_text())
    scale = data["pixels_per_point"]
    width, height = round(data["width"] * scale), round(data["height"] * scale)
    canvas = np.zeros((height, width, 4), dtype=np.float32)
    textures = {}
    for texture in data["textures"]:
        w, h = texture["size"]
        textures[texture["id"]] = np.frombuffer(base64.b64decode(texture["rgba"]),
                                               dtype=np.uint8).reshape(h, w, 4) / 255.0
    for mesh in data["meshes"]:
        vertices = np.array(mesh["vertices"], dtype=np.float32)
        if not len(vertices):
            continue
        vertices[:, :2] *= scale
        vertices[:, 4:] /= 255.0
        texture = textures[mesh["texture"]]
        th, tw, _ = texture.shape
        clip = np.array(mesh["clip"]) * scale
        for triangle in np.array(mesh["indices"]).reshape(-1, 3):
            a, b, c = vertices[triangle]
            x0, y0 = np.maximum(np.floor(np.minimum.reduce([a[:2], b[:2], c[:2]])), clip[:2]).astype(int)
            x1, y1 = np.minimum(np.ceil(np.maximum.reduce([a[:2], b[:2], c[:2]])), clip[2:]).astype(int)
            x0, y0, x1, y1 = max(0, x0), max(0, y0), min(width, x1), min(height, y1)
            determinant = (b[1]-c[1])*(a[0]-c[0]) + (c[0]-b[0])*(a[1]-c[1])
            if x1 <= x0 or y1 <= y0 or abs(determinant) < 1e-8:
                continue
            y, x = np.mgrid[y0:y1, x0:x1].astype(np.float32) + 0.5
            wa = ((b[1]-c[1])*(x-c[0]) + (c[0]-b[0])*(y-c[1])) / determinant
            wb = ((c[1]-a[1])*(x-c[0]) + (a[0]-c[0])*(y-c[1])) / determinant
            wc = 1.0 - wa - wb
            mask = (wa >= 0) & (wb >= 0) & (wc >= 0)
            if not mask.any():
                continue
            attributes = wa[..., None]*a[2:] + wb[..., None]*b[2:] + wc[..., None]*c[2:]
            u = np.clip(attributes[..., 0]*tw-0.5, 0, tw-1)
            v = np.clip(attributes[..., 1]*th-0.5, 0, th-1)
            ix, iy = u.astype(int), v.astype(int)
            fx, fy = (u-ix)[..., None], (v-iy)[..., None]
            ix1, iy1 = np.minimum(ix+1, tw-1), np.minimum(iy+1, th-1)
            sample = ((1-fx)*(1-fy)*texture[iy, ix] + fx*(1-fy)*texture[iy, ix1]
                      + (1-fx)*fy*texture[iy1, ix] + fx*fy*texture[iy1, ix1])
            source = sample * attributes[..., 2:]
            target = canvas[y0:y1, x0:x1]
            target[mask] = (source + target*(1-source[..., 3:4]))[mask]
    image = Image.fromarray(np.clip(canvas*255, 0, 255).astype(np.uint8), "RGBA")
    image.save(path.with_suffix(".png"))


directory = pathlib.Path(sys.argv[1])
paths = sorted(directory.glob("*.json"))
for path in paths:
    render(path)
if paths:
    sheet = Image.new("RGB", (960, 230*((len(paths)+2)//3)), "#e8edf3")
    draw = ImageDraw.Draw(sheet)
    for index, path in enumerate(paths):
        image = Image.open(path.with_suffix(".png")).convert("RGB")
        image.thumbnail((312, 202))
        x, y = (index % 3)*320, (index // 3)*230
        sheet.paste(image, (x, y+22))
        draw.text((x+3, y+3), path.stem, fill="#192435")
    sheet.save(directory / "overview.png")
    print(f"Visual artifacts: {directory}")
